//! Hostile-client protocol properties (`docs/testing.html#protocol-suite`):
//! generated LSP traffic against the real server over an in-memory
//! connection pair.
//!
//! Every session initializes with generated (possibly bogus) position
//! encodings, replays a generated op sequence — opens, changes and closes
//! in any order, requests about never-opened documents, reversed and
//! out-of-range positions, junk `executeCommand` arguments, malformed
//! params, unknown methods — and must end with a clean shutdown. The
//! server must answer every request (no hang), never trip the
//! `catch_unwind` panic backstop, and only ever push well-formed
//! diagnostics. Every receive uses a timeout so a stuck server fails the
//! test instead of hanging CI.

use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use proptest::prelude::*;
use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(5);

/// The documents the client talks about — requests may reference them
/// before any open, after a close, or never open them at all.
static URIS: [&str; 3] = ["file:///w/a.csv", "file:///w/b.tsv", "file:///w/c"];

static LANGUAGE_IDS: [&str; 6] = ["csv", "tsv", "ssv", "psv", "plaintext", ""];

static DIALECT_ARGS: [&str; 6] = ["csv", "tsv", "ssv", "psv", "klingon", ""];

static COMMANDS: [&str; 3] = ["csv-lsp.setDialect", "csv-lsp.unknown", ""];

static REQUEST_METHODS: [&str; 7] = [
    "initialize",
    "textDocument/codeAction",
    "textDocument/formatting",
    "textDocument/documentHighlight",
    "workspace/executeCommand",
    "textDocument/definition",
    "csv/made-up",
];

/// `exit` is deliberately absent — the session sends it once, at the end.
static NOTIFICATION_METHODS: [&str; 7] = [
    "textDocument/didOpen",
    "textDocument/didChange",
    "textDocument/didClose",
    "textDocument/didSave",
    "initialized",
    "$/cancelRequest",
    "csv/made-up",
];

static ENCODING_OFFERS: [&str; 5] = ["utf-8", "utf-16", "utf-32", "wtf-8", ""];

static FRAGMENTS: &[&str] = &[
    ",", ";", "\t", "|", "\"", "\"\"", "\n", "\r\n", "\r", " ", "\u{feff}", "a", "x9", "é", "😀",
    "名",
];

#[derive(Debug, Clone)]
enum Op {
    Open {
        doc: usize,
        language_id: String,
        text: String,
    },
    Change {
        doc: usize,
        version: i32,
        text: String,
    },
    /// A ranged change under FULL sync — a client bug the server shrugs off.
    ChangeRanged {
        doc: usize,
    },
    Close {
        doc: usize,
    },
    CodeAction {
        doc: usize,
        range: (u32, u32, u32, u32),
        only_mask: Option<u8>,
    },
    Formatting {
        doc: usize,
    },
    Highlight {
        doc: usize,
        line: u32,
        character: u32,
    },
    ExecuteCommand {
        command: String,
        arguments: Vec<Value>,
    },
    RawRequest {
        method: String,
        params: Value,
    },
    RawNotification {
        method: String,
        params: Value,
    },
}

fn doc_index() -> impl Strategy<Value = usize> {
    0..URIS.len()
}

fn small_text() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(FRAGMENTS), 0..24).prop_map(|parts| parts.concat())
}

/// Mostly small, sometimes absurd coordinates.
fn coordinate() -> impl Strategy<Value = u32> {
    prop_oneof![
        4 => 0u32..8,
        1 => 0u32..2000,
        1 => Just(u32::MAX),
    ]
}

/// Small arbitrary JSON for params and arguments nobody should trust.
fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        "[a-zA-Z0-9 /:.-]{0,12}".prop_map(Value::from),
    ];
    leaf.prop_recursive(3, 12, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::btree_map("[a-z]{1,6}", inner, 0..4)
                .prop_map(|map| Value::Object(map.into_iter().collect())),
        ]
    })
}

fn command_arguments() -> impl Strategy<Value = Vec<Value>> {
    prop_oneof![
        2 => (doc_index(), prop::sample::select(&DIALECT_ARGS[..]))
            .prop_map(|(doc, dialect)| vec![json!(URIS[doc]), json!(dialect)]),
        1 => prop::collection::vec(json_value(), 0..3),
    ]
}

fn op() -> impl Strategy<Value = Op> {
    let language_id = prop::sample::select(&LANGUAGE_IDS[..]).prop_map(str::to_owned);
    prop_oneof![
        4 => (doc_index(), language_id, small_text())
            .prop_map(|(doc, language_id, text)| Op::Open { doc, language_id, text }),
        4 => (doc_index(), any::<i32>(), small_text())
            .prop_map(|(doc, version, text)| Op::Change { doc, version, text }),
        1 => doc_index().prop_map(|doc| Op::ChangeRanged { doc }),
        2 => doc_index().prop_map(|doc| Op::Close { doc }),
        4 => (
            doc_index(),
            (coordinate(), coordinate(), coordinate(), coordinate()),
            proptest::option::of(0u8..32),
        )
            .prop_map(|(doc, range, only_mask)| Op::CodeAction { doc, range, only_mask }),
        2 => doc_index().prop_map(|doc| Op::Formatting { doc }),
        3 => (doc_index(), coordinate(), coordinate())
            .prop_map(|(doc, line, character)| Op::Highlight { doc, line, character }),
        2 => (prop::sample::select(&COMMANDS[..]).prop_map(str::to_owned), command_arguments())
            .prop_map(|(command, arguments)| Op::ExecuteCommand { command, arguments }),
        2 => (prop::sample::select(&REQUEST_METHODS[..]).prop_map(str::to_owned), json_value())
            .prop_map(|(method, params)| Op::RawRequest { method, params }),
        2 => (prop::sample::select(&NOTIFICATION_METHODS[..]).prop_map(str::to_owned), json_value())
            .prop_map(|(method, params)| Op::RawNotification { method, params }),
    ]
}

fn encoding_offers() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(
        prop::sample::select(&ENCODING_OFFERS[..]).prop_map(str::to_owned),
        0..4,
    )
}

/// The editor side of one in-memory session against the real server.
struct Client {
    conn: Connection,
    server: Option<std::thread::JoinHandle<Result<(), csv_lsp::server::BoxError>>>,
    next_id: i32,
    notifications: Vec<Notification>,
}

impl Client {
    fn start(encodings: &[String]) -> Client {
        let (client_conn, server_conn) = Connection::memory();
        let server = std::thread::spawn(move || csv_lsp::server::run(server_conn));
        let mut client = Client {
            conn: client_conn,
            server: Some(server),
            next_id: 0,
            notifications: Vec::new(),
        };
        let params = json!({ "capabilities": { "general": { "positionEncodings": encodings } } });
        let response = client.request("initialize", params);
        assert!(
            response.error.is_none(),
            "initialize failed: {:?}",
            response.error
        );
        client.notify("initialized", json!({}));
        client
    }

    /// Send a request and wait for its response, buffering interleaved
    /// notifications. Every failure mode is an assertion: a hang times
    /// out, a panic backstop response fails the property.
    fn request(&mut self, method: &str, params: Value) -> Response {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        let request = Request {
            id: id.clone(),
            method: method.to_owned(),
            params,
        };
        self.conn
            .sender
            .send(Message::Request(request))
            .expect("server hung up before the request");
        loop {
            let message = self
                .conn
                .receiver
                .recv_timeout(TIMEOUT)
                .unwrap_or_else(|_| panic!("`{method}` was never answered"));
            match message {
                Message::Response(response) => {
                    assert_eq!(response.id, id, "response for the wrong request");
                    if let Some(error) = &response.error {
                        assert!(
                            !error.message.contains("panicked"),
                            "`{method}` tripped the panic backstop: {}",
                            error.message
                        );
                    }
                    return response;
                }
                Message::Notification(notification) => self.notifications.push(notification),
                Message::Request(request) => panic!("unexpected server request {request:?}"),
            }
        }
    }

    fn notify(&self, method: &str, params: Value) {
        let notification = Notification {
            method: method.to_owned(),
            params,
        };
        self.conn
            .sender
            .send(Message::Notification(notification))
            .expect("server hung up before the notification");
    }

    /// Shut the session down cleanly and validate everything the server
    /// pushed along the way.
    fn finish(mut self) {
        let response = self.request("shutdown", Value::Null);
        assert!(
            response.error.is_none(),
            "shutdown failed: {:?}",
            response.error
        );
        self.notify("exit", Value::Null);
        match self.server.take().expect("server handle").join() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => panic!("server exited with an error: {err}"),
            Err(_) => panic!("the server thread panicked"),
        }
        while let Ok(message) = self.conn.receiver.try_recv() {
            match message {
                Message::Notification(notification) => self.notifications.push(notification),
                other => panic!("unexpected late message {other:?}"),
            }
        }
        for notification in &self.notifications {
            assert_eq!(
                notification.method, "textDocument/publishDiagnostics",
                "unexpected server notification {notification:?}"
            );
            let params: lsp_types::PublishDiagnosticsParams =
                serde_json::from_value(notification.params.clone())
                    .expect("malformed publishDiagnostics payload");
            for diagnostic in &params.diagnostics {
                let (start, end) = (diagnostic.range.start, diagnostic.range.end);
                assert!(
                    (start.line, start.character) <= (end.line, end.character),
                    "inverted diagnostic range {:?}",
                    diagnostic.range
                );
            }
        }
    }
}

fn play(client: &mut Client, op: Op) {
    match op {
        Op::Open {
            doc,
            language_id,
            text,
        } => client.notify(
            "textDocument/didOpen",
            json!({ "textDocument": {
                "uri": URIS[doc], "languageId": language_id, "version": 1, "text": text,
            }}),
        ),
        Op::Change { doc, version, text } => client.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": URIS[doc], "version": version },
                "contentChanges": [{ "text": text }],
            }),
        ),
        Op::ChangeRanged { doc } => client.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": URIS[doc], "version": 2 },
                "contentChanges": [{
                    "range": { "start": { "line": 0, "character": 0 },
                               "end": { "line": 0, "character": 1 } },
                    "text": "x",
                }],
            }),
        ),
        Op::Close { doc } => client.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": URIS[doc] } }),
        ),
        Op::CodeAction {
            doc,
            range: (line_a, char_a, line_b, char_b),
            only_mask,
        } => {
            let mut context = json!({ "diagnostics": [] });
            if let Some(mask) = only_mask {
                let advertised = [
                    "quickfix",
                    "source",
                    "source.fixAll",
                    "refactor",
                    "refactor.rewrite",
                ];
                let kinds: Vec<&str> = advertised
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| mask & (1 << i) != 0)
                    .map(|(_, kind)| *kind)
                    .collect();
                context["only"] = json!(kinds);
            }
            client.request(
                "textDocument/codeAction",
                json!({
                    "textDocument": { "uri": URIS[doc] },
                    "range": { "start": { "line": line_a, "character": char_a },
                               "end": { "line": line_b, "character": char_b } },
                    "context": context,
                }),
            );
        }
        Op::Formatting { doc } => {
            client.request(
                "textDocument/formatting",
                json!({
                    "textDocument": { "uri": URIS[doc] },
                    "options": { "tabSize": 4, "insertSpaces": true },
                }),
            );
        }
        Op::Highlight {
            doc,
            line,
            character,
        } => {
            client.request(
                "textDocument/documentHighlight",
                json!({
                    "textDocument": { "uri": URIS[doc] },
                    "position": { "line": line, "character": character },
                }),
            );
        }
        Op::ExecuteCommand { command, arguments } => {
            client.request(
                "workspace/executeCommand",
                json!({ "command": command, "arguments": arguments }),
            );
        }
        Op::RawRequest { method, params } => {
            client.request(&method, params);
        }
        Op::RawNotification { method, params } => client.notify(&method, params),
    }
}

proptest! {
    // Every case spawns a real server thread; fewer, richer cases keep the
    // suite in CI-friendly time. Deeper runs: PROPTEST_CASES=1000.
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: Some(Box::new(
            proptest::test_runner::FileFailurePersistence::WithSource("proptest-regressions"),
        )),
        ..ProptestConfig::default()
    })]

    #[test]
    fn the_server_survives_hostile_clients(
        encodings in encoding_offers(),
        ops in prop::collection::vec(op(), 0..12),
    ) {
        let mut client = Client::start(&encodings);
        for op in ops {
            play(&mut client, op);
        }
        client.finish();
    }
}
