//! The synchronous LSP server: handshake, main loop and request dispatch.
//!
//! One thread, one loop (rust-analyzer's `lsp-server` architecture, see
//! ADR 0002): requests are answered in order, notifications update state.
//! Handlers are panic-isolated so a bug in one feature answers one request
//! with an error instead of killing the server.

use std::error::Error;
use std::panic::{AssertUnwindSafe, catch_unwind};

use lsp_server::{Connection, ErrorCode, Message, Request, RequestId, Response};
use lsp_types::InitializeParams;
use lsp_types::request::{CodeActionRequest, Formatting, Request as _};

use crate::capabilities;
use crate::document::Store;
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
    let _ = &mut state.store; // used from M1 on; keep construction honest
    for message in &connection.receiver {
        match message {
            Message::Request(request) => {
                if connection.handle_shutdown(&request)? {
                    return Ok(());
                }
                let response = handle_request(&mut state, request);
                connection.sender.send(Message::Response(response))?;
            }
            Message::Notification(_notification) => {
                // Document lifecycle lands in the next cycle.
            }
            // We never send server→client requests, so no responses arrive.
            Message::Response(_) => {}
        }
    }
    Ok(())
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
