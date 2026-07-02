//! End-to-end protocol tests: drive the real server over an in-memory
//! connection pair, playing the editor's role. Every receive uses a timeout
//! so a stuck server fails the test instead of hanging CI.

use std::thread::JoinHandle;
use std::time::Duration;

use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{Exit, Initialized, Notification as _, PublishDiagnostics};
use lsp_types::request::{Initialize, Shutdown};
use lsp_types::{
    ClientCapabilities, GeneralClientCapabilities, InitializeParams, InitializeResult, OneOf,
    PositionEncodingKind, PublishDiagnosticsParams, TextDocumentSyncCapability,
};

const TIMEOUT: Duration = Duration::from_secs(5);

/// The editor side of an in-memory LSP session.
struct TestClient {
    conn: Connection,
    server: Option<JoinHandle<Result<(), csv_lsp::server::BoxError>>>,
    next_id: i32,
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
                Message::Notification(_) => continue,
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
    #[expect(dead_code, reason = "used from the document-lifecycle cycle on")]
    fn recv_diagnostics(&self) -> PublishDiagnosticsParams {
        loop {
            match self.recv() {
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
