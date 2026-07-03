//! The synchronous LSP server: handshake, main loop and request dispatch.
//!
//! One thread, one loop (rust-analyzer's `lsp-server` architecture, see
//! `docs/lsp.html#crates`): requests are answered in order, notifications
//! update state.
//! Handlers are panic-isolated so a bug in one feature answers one request
//! with an error instead of killing the server.

use std::collections::HashMap;
use std::error::Error;
use std::panic::{AssertUnwindSafe, catch_unwind};

use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{
    CodeActionRequest, DocumentHighlightRequest, ExecuteCommand, Formatting, Request as _,
};
use lsp_types::{
    CodeAction, CodeActionOrCommand, CodeActionParams, Command, Diagnostic, DiagnosticSeverity,
    DocumentFormattingParams, DocumentHighlight, DocumentHighlightKind, DocumentHighlightParams,
    ExecuteCommandParams, InitializeParams, NumberOrString, PublishDiagnosticsParams, TextEdit,
    Uri, WorkspaceEdit,
};

use crate::capabilities;
use crate::dialect::Dialect;
use crate::document::{Document, Store};
use crate::edits;
use crate::features::{Action, ActionContext, Diag, Registry, ServerCommand, Severity};
use crate::log;
use crate::parse::Span;
use crate::position::PositionEncoding;

/// Boxed error type used at the server boundary.
pub type BoxError = Box<dyn Error + Send + Sync>;

/// Mutable state shared by all handlers.
struct ServerState {
    store: Store,
    encoding: PositionEncoding,
    registry: Registry,
    /// Dialect conversions offered in the last `codeAction` response, per
    /// URI: `(converted full text, target dialect)`. When a `didChange`
    /// carries exactly one of these texts, the user applied that conversion
    /// and the document flips dialect. Cleared on every change/close, so
    /// dismissed actions cost nothing.
    pending_transforms: HashMap<String, Vec<(String, Dialect)>>,
}

/// A request failure that becomes an LSP error response.
struct RequestError {
    code: ErrorCode,
    message: String,
}

/// Run the server on `connection` until the client asks it to exit.
pub fn run(connection: Connection) -> Result<(), BoxError> {
    let (id, params) = connection.initialize_start()?;
    let params: InitializeParams = serde_json::from_value(params)?;
    let encoding = capabilities::negotiate_position_encoding(&params.capabilities);
    let result = serde_json::json!({
        "capabilities": capabilities::server_capabilities(encoding),
        "serverInfo": { "name": "csv-lsp", "version": env!("CARGO_PKG_VERSION") },
    });
    connection.initialize_finish(id, result)?;
    log!("initialized with {encoding:?} positions");

    let mut state = ServerState {
        store: Store::default(),
        encoding,
        registry: Registry::standard(),
        pending_transforms: HashMap::new(),
    };
    for message in &connection.receiver {
        match message {
            Message::Request(request) => {
                if connection.handle_shutdown(&request)? {
                    return Ok(());
                }
                let response = handle_request(&mut state, &connection, request);
                connection.sender.send(Message::Response(response))?;
            }
            Message::Notification(notification) => {
                handle_notification(&mut state, &connection, notification)?;
            }
            // We never send server→client requests, so no responses arrive.
            Message::Response(_) => {}
        }
    }
    Ok(())
}

/// Track the document lifecycle and push diagnostics after every change.
fn handle_notification(
    state: &mut ServerState,
    connection: &Connection,
    notification: Notification,
) -> Result<(), BoxError> {
    match notification.method.as_str() {
        DidOpenTextDocument::METHOD => {
            if let Some(params) = cast_notification::<DidOpenTextDocument>(notification) {
                let item = params.text_document;
                let doc = state
                    .store
                    .open(item.uri, &item.language_id, item.version, item.text);
                publish_diagnostics(connection, state.encoding, &state.registry, doc)?;
            }
        }
        DidChangeTextDocument::METHOD => {
            if let Some(mut params) = cast_notification::<DidChangeTextDocument>(notification) {
                // FULL sync: exactly one change carrying the whole text.
                let Some(change) = params.content_changes.pop() else {
                    return Ok(());
                };
                if change.range.is_some() {
                    log!("ignoring ranged change under FULL sync (client bug)");
                    return Ok(());
                }
                let id = params.text_document;
                // A change matching an offered conversion IS that
                // conversion: adopt its dialect for the reparse.
                let dialect_override =
                    state
                        .pending_transforms
                        .remove(id.uri.as_str())
                        .and_then(|offers| {
                            offers
                                .into_iter()
                                .find(|(expected, _)| *expected == change.text)
                                .map(|(_, dialect)| dialect)
                        });
                if let Some(doc) =
                    state
                        .store
                        .change(&id.uri, id.version, change.text, dialect_override)
                {
                    publish_diagnostics(connection, state.encoding, &state.registry, doc)?;
                }
            }
        }
        DidCloseTextDocument::METHOD => {
            if let Some(params) = cast_notification::<DidCloseTextDocument>(notification) {
                let uri = params.text_document.uri;
                state.store.close(&uri);
                state.pending_transforms.remove(uri.as_str());
                // Clear this file's squiggles in the editor.
                let clear = PublishDiagnosticsParams {
                    uri,
                    diagnostics: Vec::new(),
                    version: None,
                };
                send_notification::<PublishDiagnostics>(connection, clear)?;
            }
        }
        // didSave, willSave, $/cancelRequest, $/setTrace: nothing to do —
        // a synchronous server answers before cancellation could matter.
        _ => {}
    }
    Ok(())
}

/// Run all diagnostic rules over the document and push the result.
fn publish_diagnostics(
    connection: &Connection,
    encoding: PositionEncoding,
    registry: &Registry,
    doc: &Document,
) -> Result<(), BoxError> {
    let diagnostics = registry
        .diagnostics(&doc.text, &doc.table)
        .into_iter()
        .map(|diag| to_lsp_diagnostic(diag, doc, encoding))
        .collect();
    let params = PublishDiagnosticsParams {
        uri: doc.uri.clone(),
        diagnostics,
        version: Some(doc.version),
    };
    send_notification::<PublishDiagnostics>(connection, params)
}

/// The boundary conversion: byte spans → positions in the negotiated
/// encoding, internal severities → LSP severities.
fn to_lsp_diagnostic(diag: Diag, doc: &Document, encoding: PositionEncoding) -> Diagnostic {
    Diagnostic {
        range: doc.line_index.range(&doc.text, diag.span, encoding),
        severity: Some(match diag.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
            Severity::Info => DiagnosticSeverity::INFORMATION,
            Severity::Hint => DiagnosticSeverity::HINT,
        }),
        code: Some(NumberOrString::String(diag.code.to_owned())),
        source: Some("csv-lsp".to_owned()),
        message: diag.message,
        data: diag.data,
        ..Default::default()
    }
}

fn send_notification<N: lsp_types::notification::Notification>(
    connection: &Connection,
    params: N::Params,
) -> Result<(), BoxError> {
    let notification = Notification::new(N::METHOD.to_owned(), params);
    connection
        .sender
        .send(Message::Notification(notification))?;
    Ok(())
}

/// Deserialize notification params; malformed ones are logged and dropped
/// (notifications must never be answered, not even with errors).
fn cast_notification<N: lsp_types::notification::Notification>(
    notification: Notification,
) -> Option<N::Params> {
    match notification.extract(N::METHOD) {
        Ok(params) => Some(params),
        Err(err) => {
            log!("malformed `{}` notification: {err}", N::METHOD);
            None
        }
    }
}

/// Answer a request, converting errors and panics into error responses.
fn handle_request(state: &mut ServerState, connection: &Connection, request: Request) -> Response {
    let id = request.id.clone();
    match catch_unwind(AssertUnwindSafe(|| {
        dispatch_request(state, connection, request)
    })) {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => Response::new_err(id, err.code as i32, err.message),
        Err(_) => Response::new_err(
            id,
            ErrorCode::InternalError as i32,
            "request handler panicked (this is a csv-lsp bug)".to_owned(),
        ),
    }
}

fn dispatch_request(
    state: &mut ServerState,
    connection: &Connection,
    request: Request,
) -> Result<Response, RequestError> {
    match request.method.as_str() {
        CodeActionRequest::METHOD => {
            let (id, params) = cast_request::<CodeActionRequest>(request)?;
            Ok(Response::new_ok(id, handle_code_action(state, &params)))
        }
        Formatting::METHOD => {
            let (id, params) = cast_request::<Formatting>(request)?;
            Ok(Response::new_ok(id, handle_formatting(state, &params)))
        }
        ExecuteCommand::METHOD => {
            let (id, params) = cast_request::<ExecuteCommand>(request)?;
            handle_execute_command(state, connection, params)?;
            // Effects travel as notifications (fresh diagnostics); the
            // response itself is empty.
            Ok(Response::new_ok(id, serde_json::Value::Null))
        }
        DocumentHighlightRequest::METHOD => {
            let (id, params) = cast_request::<DocumentHighlightRequest>(request)?;
            Ok(Response::new_ok(
                id,
                handle_document_highlight(state, &params),
            ))
        }
        _ => Ok(Response::new_err(
            request.id,
            ErrorCode::MethodNotFound as i32,
            format!("csv-lsp does not handle `{}`", request.method),
        )),
    }
}

/// Compute the code actions for the requested range, remembering offered
/// dialect conversions so the matching `didChange` can flip the document's
/// dialect.
fn handle_code_action(
    state: &mut ServerState,
    params: &CodeActionParams,
) -> Vec<CodeActionOrCommand> {
    let Some(doc) = state.store.get(&params.text_document.uri) else {
        return Vec::new();
    };
    let start = doc
        .line_index
        .offset(&doc.text, params.range.start, state.encoding);
    let end = doc
        .line_index
        .offset(&doc.text, params.range.end, state.encoding);
    let ctx = ActionContext {
        doc,
        range: Span::new(start.min(end), start.max(end)),
        client_diagnostics: &params.context.diagnostics,
        only: params.context.only.as_deref(),
    };
    let mut offered_transforms = Vec::new();
    let encoding = state.encoding;
    let actions: Vec<CodeActionOrCommand> = state
        .registry
        .actions(&ctx)
        .into_iter()
        .map(|action| {
            if let Some(dialect) = action.dialect_change {
                offered_transforms.push((edits::apply(&doc.text, &action.edits), dialect));
            }
            to_lsp_action(action, doc, encoding)
        })
        .collect();
    if !offered_transforms.is_empty() {
        state
            .pending_transforms
            .insert(doc.uri.as_str().to_owned(), offered_transforms);
    }
    actions
}

/// The boundary conversion for actions: spans → ranges, one workspace edit
/// per text action (`changes` map form — broadest client compatibility),
/// and an LSP `Command` for actions carrying a server-side effect.
fn to_lsp_action(
    action: Action,
    doc: &Document,
    encoding: PositionEncoding,
) -> CodeActionOrCommand {
    let Action {
        title,
        kind,
        edits,
        command,
        dialect_change: _, // consumed by handle_code_action's pending map
        fixes,
        is_preferred,
    } = action;
    let edit = (!edits.is_empty()).then(|| {
        let text_edits: Vec<TextEdit> = edits
            .into_iter()
            .map(|(span, new_text)| TextEdit {
                range: doc.line_index.range(&doc.text, span, encoding),
                new_text,
            })
            .collect();
        WorkspaceEdit {
            changes: Some(HashMap::from([(doc.uri.clone(), text_edits)])),
            ..Default::default()
        }
    });
    let command = command.map(|server_command| to_lsp_command(&title, server_command, doc));
    let diagnostics: Vec<Diagnostic> = fixes
        .into_iter()
        .map(|diag| to_lsp_diagnostic(diag, doc, encoding))
        .collect();
    CodeActionOrCommand::CodeAction(CodeAction {
        title,
        kind: Some(kind),
        diagnostics: (!diagnostics.is_empty()).then_some(diagnostics),
        edit,
        command,
        is_preferred: is_preferred.then_some(true),
        ..Default::default()
    })
}

/// Encode a [`ServerCommand`] as the LSP `Command` the client echoes back
/// via `workspace/executeCommand`.
fn to_lsp_command(title: &str, command: ServerCommand, doc: &Document) -> Command {
    match command {
        ServerCommand::SetDialect { dialect } => Command {
            title: title.to_owned(),
            command: capabilities::SET_DIALECT_COMMAND.to_owned(),
            arguments: Some(vec![
                serde_json::json!(doc.uri.as_str()),
                serde_json::json!(dialect.name().to_ascii_lowercase()),
            ]),
        },
    }
}

/// Execute `csv-lsp.setDialect`: re-parse the document under the requested
/// dialect and republish its diagnostics.
fn handle_execute_command(
    state: &mut ServerState,
    connection: &Connection,
    params: ExecuteCommandParams,
) -> Result<(), RequestError> {
    let invalid = |message: String| RequestError {
        code: ErrorCode::InvalidParams,
        message,
    };
    if params.command != capabilities::SET_DIALECT_COMMAND {
        return Err(invalid(format!("unknown command `{}`", params.command)));
    }
    let [uri_arg, dialect_arg] = params.arguments.as_slice() else {
        return Err(invalid(format!(
            "`{}` expects [uri, dialect], got {} argument(s)",
            params.command,
            params.arguments.len()
        )));
    };
    let uri: Uri = uri_arg
        .as_str()
        .and_then(|raw| raw.parse().ok())
        .ok_or_else(|| invalid(format!("invalid uri argument {uri_arg}")))?;
    let dialect = dialect_arg
        .as_str()
        .and_then(Dialect::from_language_id)
        .ok_or_else(|| invalid(format!("invalid dialect argument {dialect_arg}")))?;

    // Unknown documents are tolerated (closed since the action was offered).
    if let Some(doc) = state.store.set_dialect(&uri, dialect) {
        publish_diagnostics(connection, state.encoding, &state.registry, doc).map_err(|err| {
            RequestError {
                code: ErrorCode::InternalError,
                message: err.to_string(),
            }
        })?;
    }
    Ok(())
}

/// Highlight every cell of the column under the cursor. Helix turns these
/// ranges into multi-selections (`Space+h`), which is how "select the
/// column" works; `None` when the cursor is on no cell — clients call this
/// speculatively.
fn handle_document_highlight(
    state: &ServerState,
    params: &DocumentHighlightParams,
) -> Option<Vec<DocumentHighlight>> {
    let position = &params.text_document_position_params;
    let doc = state.store.get(&position.text_document.uri)?;
    let offset = doc
        .line_index
        .offset(&doc.text, position.position, state.encoding);
    let (_, column) = doc.table.cell_at(offset)?;
    let highlights = crate::features::columns::column_content_spans(&doc.table, column)
        .into_iter()
        .map(|span| DocumentHighlight {
            range: doc.line_index.range(&doc.text, span, state.encoding),
            kind: Some(DocumentHighlightKind::TEXT),
        })
        .collect();
    Some(highlights)
}

/// Formatting = align columns. `FormattingOptions` (tab width etc.) carry
/// no meaning for CSV and are ignored; `None` for already-aligned files
/// keeps save-time formatting idempotent.
fn handle_formatting(
    state: &ServerState,
    params: &DocumentFormattingParams,
) -> Option<Vec<TextEdit>> {
    let doc = state.store.get(&params.text_document.uri)?;
    let edits = crate::features::align::align_edits(doc);
    if edits.is_empty() {
        return None;
    }
    Some(
        edits
            .into_iter()
            .map(|(span, new_text)| TextEdit {
                range: doc.line_index.range(&doc.text, span, state.encoding),
                new_text,
            })
            .collect(),
    )
}

/// Deserialize request params, mapping failures to `InvalidParams`.
fn cast_request<R: lsp_types::request::Request>(
    request: Request,
) -> Result<(RequestId, R::Params), RequestError> {
    request.extract(R::METHOD).map_err(|err| RequestError {
        code: ErrorCode::InvalidParams,
        message: format!("invalid `{}` params: {err}", R::METHOD),
    })
}
