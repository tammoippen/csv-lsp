//! The synchronous LSP server: handshake, main loop and request dispatch.
//!
//! One thread, one loop (rust-analyzer's `lsp-server` architecture, see
//! ADR 0002): requests are answered in order, notifications update state.
//! Handlers are panic-isolated so a bug in one feature answers one request
//! with an error instead of killing the server.

use std::error::Error;
use std::panic::{AssertUnwindSafe, catch_unwind};

use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{CodeActionRequest, Formatting, Request as _};
use lsp_types::{InitializeParams, PublishDiagnosticsParams};

use crate::capabilities;
use crate::document::{Document, Store};
use crate::log;

/// Boxed error type used at the server boundary.
pub type BoxError = Box<dyn Error + Send + Sync>;

/// Mutable state shared by all handlers.
struct ServerState {
    store: Store,
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
                publish_diagnostics(connection, doc)?;
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
                    publish_diagnostics(connection, doc)?;
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

/// Publish the document's diagnostics (empty until the M1 registry lands).
fn publish_diagnostics(connection: &Connection, doc: &Document) -> Result<(), BoxError> {
    let params = PublishDiagnosticsParams {
        uri: doc.uri.clone(),
        diagnostics: Vec::new(),
        version: Some(doc.version),
    };
    send_notification::<PublishDiagnostics>(connection, params)
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
    let _ = &state;
    match request.method.as_str() {
        CodeActionRequest::METHOD => {
            let (id, _params) = cast_request::<CodeActionRequest>(request)?;
            // Stub until M2: no actions, but the capability is answerable.
            Ok(Response::new_ok(id, serde_json::json!([])))
        }
        Formatting::METHOD => {
            let (id, _params) = cast_request::<Formatting>(request)?;
            // Stub until M3: no edits, but the capability is answerable.
            Ok(Response::new_ok(id, serde_json::Value::Null))
        }
        _ => Ok(Response::new_err(
            request.id,
            ErrorCode::MethodNotFound as i32,
            format!("csv-lsp does not handle `{}`", request.method),
        )),
    }
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
