//! End-to-end protocol tests: drive the real server over an in-memory
//! connection pair, playing the editor's role. Every receive uses a timeout
//! so a stuck server fails the test instead of hanging CI.

use std::collections::VecDeque;
use std::thread::JoinHandle;
use std::time::Duration;

use csv_lsp::position::{LineIndex, PositionEncoding};
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Exit, Initialized,
    Notification as _, PublishDiagnostics,
};
use lsp_types::request::{
    CodeActionRequest, ExecuteCommand, Formatting, Initialize, Request as _, Shutdown,
};
use lsp_types::{
    ClientCapabilities, CodeAction, CodeActionContext, CodeActionKind, CodeActionOrCommand,
    CodeActionParams, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentFormattingParams, ExecuteCommandParams, FormattingOptions,
    GeneralClientCapabilities, InitializeParams, InitializeResult, NumberOrString, OneOf, Position,
    PositionEncodingKind, PublishDiagnosticsParams, Range, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentItem, TextDocumentSyncCapability, TextEdit, Uri,
    VersionedTextDocumentIdentifier,
};

const TIMEOUT: Duration = Duration::from_secs(5);

/// The editor side of an in-memory LSP session.
struct TestClient {
    conn: Connection,
    server: Option<JoinHandle<Result<(), csv_lsp::server::BoxError>>>,
    next_id: i32,
    /// Notifications received while waiting for a response (e.g. the fresh
    /// diagnostics a command publishes *before* answering) — consumed by
    /// `recv_diagnostics` instead of being dropped.
    pending: VecDeque<Message>,
}

impl TestClient {
    /// Spawn the server, run `initialize`/`initialized` offering `encodings`.
    fn start_with(encodings: &[PositionEncodingKind]) -> (Self, InitializeResult) {
        let (client_conn, server_conn) = Connection::memory();
        let server = std::thread::spawn(move || csv_lsp::server::run(server_conn));
        let mut client = TestClient {
            conn: client_conn,
            server: Some(server),
            next_id: 0,
            pending: VecDeque::new(),
        };
        let params = InitializeParams {
            capabilities: ClientCapabilities {
                general: Some(GeneralClientCapabilities {
                    position_encodings: Some(encodings.to_vec()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let result = client.request::<Initialize>(params);
        client.notify::<Initialized>(lsp_types::InitializedParams {});
        (client, result)
    }

    fn recv(&self) -> Message {
        self.conn
            .receiver
            .recv_timeout(TIMEOUT)
            .expect("timed out waiting for a server message")
    }

    /// Buffered messages first, then the wire.
    fn next_message(&mut self) -> Message {
        if let Some(message) = self.pending.pop_front() {
            return message;
        }
        self.recv()
    }

    /// Send a request and wait for its response, skipping interleaved
    /// notifications (e.g. diagnostics).
    fn raw_request(&mut self, method: &str, params: serde_json::Value) -> Response {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        let request = Request {
            id: id.clone(),
            method: method.to_owned(),
            params,
        };
        self.conn.sender.send(Message::Request(request)).unwrap();
        loop {
            match self.recv() {
                Message::Response(response) => {
                    assert_eq!(response.id, id, "response for the wrong request");
                    return response;
                }
                notification @ Message::Notification(_) => self.pending.push_back(notification),
                Message::Request(request) => panic!("unexpected server request {request:?}"),
            }
        }
    }

    /// Typed request that must succeed.
    fn request<R: lsp_types::request::Request>(&mut self, params: R::Params) -> R::Result {
        let response = self.raw_request(R::METHOD, serde_json::to_value(params).unwrap());
        assert!(
            response.error.is_none(),
            "`{}` failed: {:?}",
            R::METHOD,
            response.error
        );
        serde_json::from_value(response.result.unwrap_or(serde_json::Value::Null)).unwrap()
    }

    fn notify<N: lsp_types::notification::Notification>(&self, params: N::Params) {
        let notification = Notification::new(N::METHOD.to_owned(), params);
        self.conn
            .sender
            .send(Message::Notification(notification))
            .unwrap();
    }

    /// Wait for the next `textDocument/publishDiagnostics` notification.
    fn recv_diagnostics(&mut self) -> PublishDiagnosticsParams {
        loop {
            match self.next_message() {
                Message::Notification(notification)
                    if notification.method == PublishDiagnostics::METHOD =>
                {
                    return serde_json::from_value(notification.params).unwrap();
                }
                Message::Notification(_) => continue,
                other => panic!("expected diagnostics, got {other:?}"),
            }
        }
    }

    /// Open a document with the given language id, version and text.
    fn open(&self, uri: &Uri, language_id: &str, version: i32, text: &str) {
        self.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: language_id.to_owned(),
                version,
                text: text.to_owned(),
            },
        });
    }

    /// Replace a document's text (FULL sync shape).
    fn change(&self, uri: &Uri, version: i32, text: &str) {
        self.notify::<DidChangeTextDocument>(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: text.to_owned(),
            }],
        });
    }

    fn close(&self, uri: &Uri) {
        self.notify::<DidCloseTextDocument>(DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
        });
    }

    /// Orderly shutdown: request + exit notification + thread join.
    fn shutdown(mut self) {
        let () = self.request::<Shutdown>(());
        self.notify::<Exit>(());
        self.server
            .take()
            .expect("server already joined")
            .join()
            .expect("server thread panicked")
            .expect("server returned an error");
    }
}

/// Apply LSP edits client-side (back-to-front so earlier offsets stay
/// valid), exactly like an editor would.
fn apply_edits(text: &str, edits: &[TextEdit], enc: PositionEncoding) -> String {
    let index = LineIndex::new(text);
    let mut replacements: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|edit| {
            (
                index.offset(text, edit.range.start, enc),
                index.offset(text, edit.range.end, enc),
                edit.new_text.as_str(),
            )
        })
        .collect();
    replacements.sort_by_key(|&(start, _, _)| start);
    let mut result = text.to_owned();
    for &(start, end, new_text) in replacements.iter().rev() {
        result.replace_range(start..end, new_text);
    }
    result
}

/// Request code actions at `range`, unwrapping to plain `CodeAction`s.
fn code_actions(
    client: &mut TestClient,
    uri: &Uri,
    range: Range,
    only: Option<Vec<CodeActionKind>>,
) -> Vec<CodeAction> {
    let params = CodeActionParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        range,
        context: CodeActionContext {
            diagnostics: Vec::new(),
            only,
            trigger_kind: None,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = client.request::<CodeActionRequest>(params);
    response
        .unwrap_or_default()
        .into_iter()
        .map(|entry| match entry {
            CodeActionOrCommand::CodeAction(action) => action,
            CodeActionOrCommand::Command(command) => panic!("unexpected command {command:?}"),
        })
        .collect()
}

/// The edits an action carries for `uri`.
fn edits_for(action: &CodeAction, uri: &Uri) -> Vec<TextEdit> {
    action
        .edit
        .as_ref()
        .expect("action carries an edit")
        .changes
        .as_ref()
        .expect("changes map form")
        .get(uri)
        .expect("edits for the document")
        .clone()
}

fn cursor(line: u32, character: u32) -> Range {
    let at = Position { line, character };
    Range { start: at, end: at }
}

#[test]
fn reinterpret_fixes_a_mislabeled_semicolon_file() {
    let (mut client, _) = TestClient::start_with(&[PositionEncodingKind::UTF8]);
    let uri: Uri = "file:///t/mislabeled.csv".parse().unwrap();
    // Semicolon content under a .csv name parses as single-column CSV —
    // the missing cell on row 1 is invisible.
    client.open(&uri, "csv", 1, "a;b;c\n1;2\n");
    assert!(client.recv_diagnostics().diagnostics.is_empty());

    let actions = code_actions(
        &mut client,
        &uri,
        cursor(0, 0),
        Some(vec![CodeActionKind::SOURCE]),
    );
    let reinterpret = actions
        .iter()
        .find(|action| action.title == "Reinterpret as SSV")
        .expect("reinterpret offered");
    assert!(reinterpret.edit.is_none(), "no text change");
    let command = reinterpret.command.clone().expect("carries a command");

    let _: Option<serde_json::Value> = client.request::<ExecuteCommand>(ExecuteCommandParams {
        command: command.command,
        arguments: command.arguments.unwrap_or_default(),
        work_done_progress_params: Default::default(),
    });

    // The server reparsed and pushed the sane SSV view.
    let published = client.recv_diagnostics();
    assert_eq!(published.diagnostics.len(), 1);
    assert_eq!(
        published.diagnostics[0].code,
        Some(NumberOrString::String("row-missing-cells".into()))
    );

    // Follow-up actions reflect the new dialect.
    let actions = code_actions(
        &mut client,
        &uri,
        cursor(0, 0),
        Some(vec![CodeActionKind::SOURCE]),
    );
    assert!(
        actions
            .iter()
            .any(|action| action.title == "Reinterpret as CSV")
    );

    client.shutdown();
}

#[test]
fn applying_a_conversion_flips_the_dialect() {
    let (mut client, _) = TestClient::start_with(&[PositionEncodingKind::UTF8]);
    let uri: Uri = "file:///t/convert.csv".parse().unwrap();
    let text = "a,b,c\n1,2\n"; // one short row
    client.open(&uri, "csv", 1, text);
    assert_eq!(client.recv_diagnostics().diagnostics.len(), 1);

    let actions = code_actions(
        &mut client,
        &uri,
        cursor(0, 0),
        Some(vec![CodeActionKind::SOURCE]),
    );
    let to_tsv = actions
        .iter()
        .find(|action| action.title == "Convert to TSV")
        .expect("tsv conversion offered");
    let converted = apply_edits(text, &edits_for(to_tsv, &uri), PositionEncoding::Utf8);
    assert_eq!(converted, "a\tb\tc\n1\t2\n");

    client.change(&uri, 2, &converted);
    // Same single diagnostic — no error explosion from a stale dialect.
    let published = client.recv_diagnostics();
    assert_eq!(published.diagnostics.len(), 1);
    assert_eq!(
        published.diagnostics[0].code,
        Some(NumberOrString::String("row-missing-cells".into()))
    );

    // The way back is offered: the dialect flipped to TSV.
    let actions = code_actions(
        &mut client,
        &uri,
        cursor(0, 0),
        Some(vec![CodeActionKind::SOURCE]),
    );
    assert!(
        actions
            .iter()
            .any(|action| action.title == "Convert to CSV")
    );

    client.shutdown();
}

#[test]
fn manual_changes_do_not_flip_the_dialect() {
    let (mut client, _) = TestClient::start_with(&[PositionEncodingKind::UTF8]);
    let uri: Uri = "file:///t/manual.csv".parse().unwrap();
    client.open(&uri, "csv", 1, "a,b\n1,2\n");
    client.recv_diagnostics();

    // Conversions get offered (and recorded)…
    code_actions(
        &mut client,
        &uri,
        cursor(0, 0),
        Some(vec![CodeActionKind::SOURCE]),
    );
    // …but the user types something else instead.
    client.change(&uri, 2, "a,b\n1,2\n3,4\n");
    client.recv_diagnostics();

    let actions = code_actions(
        &mut client,
        &uri,
        cursor(0, 0),
        Some(vec![CodeActionKind::SOURCE]),
    );
    assert!(
        actions
            .iter()
            .any(|action| action.title == "Convert to TSV")
    );
    assert!(
        !actions
            .iter()
            .any(|action| action.title == "Convert to CSV")
    );

    client.shutdown();
}

#[test]
fn reinterpret_then_convert_repairs_a_mislabeled_file() {
    let (mut client, _) = TestClient::start_with(&[PositionEncodingKind::UTF8]);
    let uri: Uri = "file:///t/preise.csv".parse().unwrap();
    let text = "artikel;preis\nbolzen;1,50\n";
    client.open(&uri, "csv", 1, text);
    client.recv_diagnostics();

    // Step 1: reinterpret as SSV — diagnostics become the sane view.
    let actions = code_actions(
        &mut client,
        &uri,
        cursor(0, 0),
        Some(vec![CodeActionKind::SOURCE]),
    );
    let command = actions
        .iter()
        .find(|action| action.title == "Reinterpret as SSV")
        .and_then(|action| action.command.clone())
        .expect("reinterpret command");
    let _: Option<serde_json::Value> = client.request::<ExecuteCommand>(ExecuteCommandParams {
        command: command.command,
        arguments: command.arguments.unwrap_or_default(),
        work_done_progress_params: Default::default(),
    });
    assert!(client.recv_diagnostics().diagnostics.is_empty());

    // Step 2: convert to CSV so the content matches the extension again.
    let actions = code_actions(
        &mut client,
        &uri,
        cursor(0, 0),
        Some(vec![CodeActionKind::SOURCE]),
    );
    let to_csv = actions
        .iter()
        .find(|action| action.title == "Convert to CSV")
        .expect("csv conversion offered");
    let converted = apply_edits(text, &edits_for(to_csv, &uri), PositionEncoding::Utf8);
    assert_eq!(converted, "artikel,preis\nbolzen,\"1,50\"\n");
    client.change(&uri, 2, &converted);
    assert!(client.recv_diagnostics().diagnostics.is_empty());

    // The document is genuinely CSV now.
    let actions = code_actions(
        &mut client,
        &uri,
        cursor(0, 0),
        Some(vec![CodeActionKind::SOURCE]),
    );
    assert!(
        actions
            .iter()
            .any(|action| action.title == "Convert to SSV")
    );

    client.shutdown();
}

#[test]
fn quote_column_applies_across_all_rows() {
    let (mut client, _) = TestClient::start_with(&[PositionEncodingKind::UTF8]);
    let uri: Uri = "file:///t/quote.csv".parse().unwrap();
    let text = "id,name\n1,\"x\"\n2,y\n";
    client.open(&uri, "csv", 1, text);
    assert!(client.recv_diagnostics().diagnostics.is_empty());

    // Cursor in the `name` column, filtered to refactors only.
    let actions = code_actions(
        &mut client,
        &uri,
        cursor(0, 3),
        Some(vec![CodeActionKind::REFACTOR]),
    );
    assert!(actions.iter().all(|action| {
        action
            .kind
            .as_ref()
            .is_some_and(|kind| kind.as_str().starts_with("refactor"))
    }));
    let column = actions
        .iter()
        .find(|action| action.title == "Quote column \"name\"")
        .expect("quote column offered");

    let quoted = apply_edits(text, &edits_for(column, &uri), PositionEncoding::Utf8);
    assert_eq!(quoted, "id,\"name\"\n1,\"x\"\n2,\"y\"\n");
    client.change(&uri, 2, &quoted);
    // Quoting is structure-neutral: still no diagnostics.
    assert!(client.recv_diagnostics().diagnostics.is_empty());

    // The column is fully quoted now — the action disappears.
    let actions = code_actions(
        &mut client,
        &uri,
        cursor(0, 3),
        Some(vec![CodeActionKind::REFACTOR]),
    );
    assert!(
        !actions
            .iter()
            .any(|action| action.title.starts_with("Quote column"))
    );

    client.shutdown();
}

#[test]
fn column_edits_round_trip() {
    let (mut client, _) = TestClient::start_with(&[PositionEncodingKind::UTF8]);
    let uri: Uri = "file:///t/columns.csv".parse().unwrap();
    let text = "id,name\n1,x\n2,y\n";
    client.open(&uri, "csv", 1, text);
    assert!(client.recv_diagnostics().diagnostics.is_empty());

    // Add an empty column right of `id`.
    let only = Some(vec![CodeActionKind::REFACTOR]);
    let actions = code_actions(&mut client, &uri, cursor(0, 0), only.clone());
    let add_right = actions
        .iter()
        .find(|action| action.title == "Add column right of \"id\"")
        .expect("add right offered");
    let with_column = apply_edits(text, &edits_for(add_right, &uri), PositionEncoding::Utf8);
    assert_eq!(with_column, "id,,name\n1,,x\n2,,y\n");
    client.change(&uri, 2, &with_column);
    // The header moved with the rows: still clean.
    assert!(client.recv_diagnostics().diagnostics.is_empty());

    // Delete the new (empty, headerless) column: cursor inside it.
    let actions = code_actions(&mut client, &uri, cursor(0, 3), only);
    let delete = actions
        .iter()
        .find(|action| action.title == "Delete column #2")
        .expect("delete offered");
    let restored = apply_edits(
        &with_column,
        &edits_for(delete, &uri),
        PositionEncoding::Utf8,
    );
    assert_eq!(restored, text); // byte-identical round trip
    client.change(&uri, 3, &restored);
    assert!(client.recv_diagnostics().diagnostics.is_empty());

    client.shutdown();
}

#[test]
fn unknown_commands_get_invalid_params() {
    let (mut client, _) = TestClient::start_with(&[PositionEncodingKind::UTF8]);
    let response = client.raw_request(
        ExecuteCommand::METHOD,
        serde_json::json!({ "command": "csv-lsp.frobnicate", "arguments": [] }),
    );
    let error = response.error.expect("expected an error response");
    assert_eq!(error.code, ErrorCode::InvalidParams as i32);
    client.shutdown();
}

#[test]
fn pad_quickfix_round_trips_to_clean_diagnostics() {
    let (mut client, _) = TestClient::start_with(&[PositionEncodingKind::UTF8]);
    let uri: Uri = "file:///t/pad.csv".parse().unwrap();
    let text = "a,b,c\n1,2\nx\n";

    client.open(&uri, "csv", 1, text);
    assert_eq!(client.recv_diagnostics().diagnostics.len(), 2);

    // Cursor at the end of the `1,2` row.
    let actions = code_actions(&mut client, &uri, cursor(1, 3), None);
    let quickfix = actions
        .iter()
        .find(|action| action.kind == Some(CodeActionKind::QUICKFIX))
        .expect("a pad quickfix for the short row");
    assert_eq!(quickfix.is_preferred, Some(true));
    assert_eq!(quickfix.title, "Pad row with 1 empty cell");
    assert_eq!(quickfix.diagnostics.as_ref().unwrap().len(), 1);

    // Apply the edit like an editor would, then sync the new text.
    let text2 = apply_edits(text, &edits_for(quickfix, &uri), PositionEncoding::Utf8);
    assert_eq!(text2, "a,b,c\n1,2,\nx\n");
    client.change(&uri, 2, &text2);
    // Only the `x` row is still short.
    assert_eq!(client.recv_diagnostics().diagnostics.len(), 1);

    client.shutdown();
}

#[test]
fn fix_all_source_action_repairs_the_whole_file() {
    let (mut client, _) = TestClient::start_with(&[PositionEncodingKind::UTF8]);
    let uri: Uri = "file:///t/fixall.csv".parse().unwrap();
    let text = "a,b,c\n1,2\nx\n";

    client.open(&uri, "csv", 1, text);
    client.recv_diagnostics();

    // Cursor in the header, filtered to source.fixAll: only the fix-all.
    let actions = code_actions(
        &mut client,
        &uri,
        cursor(0, 0),
        Some(vec![CodeActionKind::SOURCE_FIX_ALL]),
    );
    assert_eq!(actions.len(), 1);
    let fix_all = &actions[0];
    assert_eq!(fix_all.title, "Pad all short rows (2)");
    assert_eq!(fix_all.kind, Some(CodeActionKind::SOURCE_FIX_ALL));

    let text2 = apply_edits(text, &edits_for(fix_all, &uri), PositionEncoding::Utf8);
    assert_eq!(text2, "a,b,c\n1,2,\nx,,\n");
    client.change(&uri, 2, &text2);
    assert!(client.recv_diagnostics().diagnostics.is_empty());

    client.shutdown();
}

/// Request document formatting for `uri`.
fn format(client: &mut TestClient, uri: &Uri) -> Option<Vec<TextEdit>> {
    client.request::<Formatting>(DocumentFormattingParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        options: FormattingOptions::default(),
        work_done_progress_params: Default::default(),
    })
}

#[test]
fn formatting_aligns_columns_under_the_utf16_fallback() {
    // A client offering nothing gets the mandatory utf-16; the emoji cell
    // stresses the surrogate-pair position math.
    let (mut client, init) = TestClient::start_with(&[]);
    assert_eq!(
        init.capabilities.position_encoding,
        Some(PositionEncodingKind::UTF16)
    );

    let uri: Uri = "file:///t/fmt.csv".parse().unwrap();
    let text = "a,😀é\nlong,x\n";
    client.open(&uri, "csv", 1, text);
    client.recv_diagnostics();

    let edits = format(&mut client, &uri).expect("alignment edits");
    let aligned = apply_edits(text, &edits, PositionEncoding::Utf16);
    assert_eq!(aligned, "a   ,😀é\nlong,x\n");

    // Formatting the aligned document is a no-op (idempotence at the
    // protocol level — critical for format-on-save).
    client.change(&uri, 2, &aligned);
    client.recv_diagnostics();
    assert_eq!(format(&mut client, &uri), None);

    client.shutdown();
}

#[test]
fn source_actions_offer_align_and_compact_as_applicable() {
    let (mut client, _) = TestClient::start_with(&[PositionEncodingKind::UTF8]);
    let uri: Uri = "file:///t/source.csv".parse().unwrap();
    let text = "id,name\n1,x\n";
    client.open(&uri, "csv", 1, text);
    client.recv_diagnostics();

    let only = Some(vec![CodeActionKind::SOURCE]);
    let actions = code_actions(&mut client, &uri, cursor(0, 0), only.clone());
    let titles: Vec<_> = actions.iter().map(|action| action.title.as_str()).collect();
    assert!(titles.contains(&"Align columns"), "got {titles:?}");
    assert!(!titles.contains(&"Compact columns"), "got {titles:?}");

    // After aligning, compact becomes the applicable action.
    let align = actions
        .iter()
        .find(|action| action.title == "Align columns")
        .unwrap();
    let aligned = apply_edits(text, &edits_for(align, &uri), PositionEncoding::Utf8);
    assert_eq!(aligned, "id,name\n1 ,x\n");
    client.change(&uri, 2, &aligned);
    client.recv_diagnostics();

    let actions = code_actions(&mut client, &uri, cursor(0, 0), only);
    let titles: Vec<_> = actions.iter().map(|action| action.title.as_str()).collect();
    assert!(titles.contains(&"Compact columns"), "got {titles:?}");
    assert!(!titles.contains(&"Align columns"), "got {titles:?}");

    client.shutdown();
}

#[test]
fn initialize_negotiates_utf8_and_advertises_features() {
    let (client, result) =
        TestClient::start_with(&[PositionEncodingKind::UTF16, PositionEncodingKind::UTF8]);

    assert_eq!(result.server_info.unwrap().name, "csv-lsp");
    let caps = result.capabilities;
    assert_eq!(caps.position_encoding, Some(PositionEncodingKind::UTF8));
    assert!(matches!(
        caps.text_document_sync,
        Some(TextDocumentSyncCapability::Options(_))
    ));
    assert!(caps.code_action_provider.is_some());
    assert_eq!(caps.document_formatting_provider, Some(OneOf::Left(true)));

    client.shutdown();
}

#[test]
fn initialize_falls_back_to_utf16() {
    let (client, result) = TestClient::start_with(&[]);
    assert_eq!(
        result.capabilities.position_encoding,
        Some(PositionEncodingKind::UTF16)
    );
    client.shutdown();
}

#[test]
fn document_lifecycle_publishes_and_clears_diagnostics() {
    let (mut client, _) = TestClient::start_with(&[PositionEncodingKind::UTF8]);
    let uri: Uri = "file:///t/data.csv".parse().unwrap();

    client.open(&uri, "csv", 1, "a,b\n1,2\n");
    let published = client.recv_diagnostics();
    assert_eq!(published.uri, uri);
    assert_eq!(published.version, Some(1));
    assert!(published.diagnostics.is_empty());

    client.change(&uri, 2, "a,b\n1,2\n3,4\n");
    let published = client.recv_diagnostics();
    assert_eq!(published.version, Some(2));

    client.close(&uri);
    let published = client.recv_diagnostics();
    assert_eq!(published.uri, uri);
    assert!(published.diagnostics.is_empty());

    client.shutdown();
}

#[test]
fn ragged_and_quoting_diagnostics_flow_to_the_client() {
    let (mut client, _) = TestClient::start_with(&[PositionEncodingKind::UTF8]);
    let uri: Uri = "file:///t/ragged.csv".parse().unwrap();

    client.open(&uri, "csv", 1, "a,b,c\n1,2\n");
    let published = client.recv_diagnostics();
    assert_eq!(published.diagnostics.len(), 1);
    let diag = &published.diagnostics[0];
    assert_eq!(
        diag.code,
        Some(NumberOrString::String("row-missing-cells".into()))
    );
    assert_eq!(diag.severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(diag.source.as_deref(), Some("csv-lsp"));
    // Zero-width range at the end of line 1 ("1,2" → character 3).
    let expected = Position {
        line: 1,
        character: 3,
    };
    assert_eq!(diag.range.start, expected);
    assert_eq!(diag.range.end, expected);

    // Fixing the row clears the squiggle.
    client.change(&uri, 2, "a,b,c\n1,2,3\n");
    assert!(client.recv_diagnostics().diagnostics.is_empty());

    // Quoting errors arrive with their own code.
    client.change(&uri, 3, "a,b,c\n\"x,y\n");
    let published = client.recv_diagnostics();
    assert_eq!(published.diagnostics.len(), 1);
    assert_eq!(
        published.diagnostics[0].code,
        Some(NumberOrString::String("unclosed-quote".into()))
    );

    client.shutdown();
}

#[test]
fn unknown_requests_get_method_not_found_and_the_server_survives() {
    let (mut client, _) = TestClient::start_with(&[PositionEncodingKind::UTF8]);

    let response = client.raw_request("textDocument/hover", serde_json::json!({}));
    let error = response.error.expect("expected an error response");
    assert_eq!(error.code, ErrorCode::MethodNotFound as i32);

    // The server keeps serving after the error.
    let response = client.raw_request("textDocument/hover", serde_json::json!({}));
    assert!(response.error.is_some());

    client.shutdown();
}

#[test]
fn malformed_request_params_get_invalid_params_and_the_server_survives() {
    let (mut client, _) = TestClient::start_with(&[PositionEncodingKind::UTF8]);

    let response = client.raw_request(
        CodeActionRequest::METHOD,
        serde_json::json!({ "bogus": true }),
    );
    let error = response.error.expect("expected an error response");
    assert_eq!(error.code, ErrorCode::InvalidParams as i32);

    // Still serving.
    let response = client.raw_request("textDocument/hover", serde_json::json!({}));
    assert!(response.error.is_some());

    client.shutdown();
}
