//! The synchronous LSP server: handshake, main loop and request dispatch.
//!
//! One thread, one loop (rust-analyzer's `lsp-server` architecture, see
//! ADR 0002): requests are answered in order, notifications update state.
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
use lsp_types::request::{CodeActionRequest, Formatting, Request as _};
use lsp_types::{
    CodeAction, CodeActionOrCommand, CodeActionParams, Diagnostic, DiagnosticSeverity,
    DocumentFormattingParams, InitializeParams, NumberOrString, PublishDiagnosticsParams, TextEdit,
    WorkspaceEdit,
};

use crate::capabilities;
use crate::document::{Document, Store};
use crate::features::{Action, ActionContext, Diag, Registry, Severity};
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
    };
    for message in &connection.receiver {
        match message {
            Message::Request(request) => {
                if connection.handle_shutdown(&request)? {
                    return Ok(());
                }
                let response = handle_request(&mut state, request);
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
                if let Some(doc) = state.store.change(&id.uri, id.version, change.text) {
                    publish_diagnostics(connection, state.encoding, &state.registry, doc)?;
                }
            }
        }
        DidCloseTextDocument::METHOD => {
            if let Some(params) = cast_notification::<DidCloseTextDocument>(notification) {
                let uri = params.text_document.uri;
                state.store.close(&uri);
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
fn handle_request(state: &mut ServerState, request: Request) -> Response {
    let id = request.id.clone();
    match catch_unwind(AssertUnwindSafe(|| dispatch_request(state, request))) {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => Response::new_err(id, err.code as i32, err.message),
        Err(_) => Response::new_err(
            id,
            ErrorCode::InternalError as i32,
            "request handler panicked (this is a csv-lsp bug)".to_owned(),
        ),
    }
}

fn dispatch_request(state: &mut ServerState, request: Request) -> Result<Response, RequestError> {
    match request.method.as_str() {
        CodeActionRequest::METHOD => {
            let (id, params) = cast_request::<CodeActionRequest>(request)?;
            Ok(Response::new_ok(id, handle_code_action(state, &params)))
        }
        Formatting::METHOD => {
            let (id, params) = cast_request::<Formatting>(request)?;
            Ok(Response::new_ok(id, handle_formatting(state, &params)))
        }
        _ => Ok(Response::new_err(
            request.id,
            ErrorCode::MethodNotFound as i32,
            format!("csv-lsp does not handle `{}`", request.method),
        )),
    }
}

/// Compute the code actions for the requested range.
fn handle_code_action(state: &ServerState, params: &CodeActionParams) -> Vec<CodeActionOrCommand> {
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
    state
        .registry
        .actions(&ctx)
        .into_iter()
        .map(|action| to_lsp_action(action, doc, state.encoding))
        .collect()
}

/// The boundary conversion for actions: spans → ranges, plus one workspace
/// edit per action targeting this document (`changes` map form — no
/// executeCommand round trip, broadest client compatibility).
fn to_lsp_action(
    action: Action,
    doc: &Document,
    encoding: PositionEncoding,
) -> CodeActionOrCommand {
    let edits: Vec<TextEdit> = action
        .edits
        .into_iter()
        .map(|(span, new_text)| TextEdit {
            range: doc.line_index.range(&doc.text, span, encoding),
            new_text,
        })
        .collect();
    let diagnostics: Vec<Diagnostic> = action
        .fixes
        .into_iter()
        .map(|diag| to_lsp_diagnostic(diag, doc, encoding))
        .collect();
    CodeActionOrCommand::CodeAction(CodeAction {
        title: action.title,
        kind: Some(action.kind),
        diagnostics: (!diagnostics.is_empty()).then_some(diagnostics),
        edit: Some(WorkspaceEdit {
            changes: Some(HashMap::from([(doc.uri.clone(), edits)])),
            ..Default::default()
        }),
        is_preferred: action.is_preferred.then_some(true),
        ..Default::default()
    })
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
