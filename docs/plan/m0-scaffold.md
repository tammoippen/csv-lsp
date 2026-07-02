# M0 — Scaffold: project, protocol handshake, document lifecycle

**Goal:** a running LSP server that initializes against a real client, negotiates the
position encoding, tracks open documents, and publishes (empty) diagnostics — plus all
project infrastructure (lints, CI, licenses). After M0 the server is *connectable but
featureless*.

**Non-goals:** no CSV parsing, no real diagnostics, no code actions (stubs only).

## Background you need (LSP in 10 minutes)

An LSP server is a process that talks **JSON-RPC 2.0 over stdio**. Every message is a
JSON object prefixed with a header:

```
Content-Length: 85\r\n
\r\n
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{...}}
```

Two message flavors:

- **Request** — has an `id`; the receiver MUST answer with a Response (`result` or
  `error`) carrying the same `id`.
- **Notification** — no `id`; never answered.

Lifecycle (client = editor, server = us):

1. Client sends `initialize` (request) with its **capabilities**; server answers with
   its own capabilities — the contract of what may be called later.
2. Client sends `initialized` (notification). Session is live.
3. Document sync notifications flow: `textDocument/didOpen` (full text + `languageId`
   + version), `didChange` (new text; we advertise FULL sync, so the client always
   sends the complete document, not deltas), `didClose`.
4. Server pushes `textDocument/publishDiagnostics` notifications whenever it likes
   (we do: after every open/change).
5. Client sends `shutdown` (request), then `exit` (notification); server process ends.

**Position encoding — the one genuinely tricky bit.** LSP positions are
`(line, character)`, where `character` counts **code units of the negotiated
encoding**, *not* bytes and *not* codepoints. The historical default is UTF-16 (VS
Code's internal string encoding). Example — in the line `😀x`:

| encoding | column of `x` |
|---|---|
| utf-8 (bytes) | 4 |
| utf-16 | 2 (😀 is a surrogate pair) |
| utf-32 (codepoints) | 1 |

Since LSP 3.17 the client may offer `general.positionEncodings` in `initialize`; the
server picks one and echoes it in `capabilities.positionEncoding`. We prefer `utf-8`
(internally everything is byte offsets — conversion becomes trivial), then `utf-32`,
and fall back to the mandatory `utf-16`. Helix offers all three. We must implement
all three conversions correctly; `position.rs` is the only module allowed to do this.

**Crates.** `lsp-server` provides `Connection` (with `stdio()` and `memory()`
constructors), the `Message`/`Request`/`Response`/`Notification` types,
`initialize_start()/initialize_finish()` (handshake helpers that let us read client
capabilities before answering), and `handle_shutdown()`. `lsp-types` provides every
params/result struct plus method-name constants via the traits
`lsp_types::request::Request` and `lsp_types::notification::Notification` (e.g.
`DidOpenTextDocument::METHOD`) — always use the constants, never string literals.
Note: `lsp-types` 0.97 has its own `Uri` type (not `url::Url`); we treat URIs as
opaque keys (`uri.to_string()` for the document map, string-level extension
extraction for dialect detection).

**Testing approach.** `Connection::memory()` returns two connected endpoints. Tests
spawn `server::run(server_side)` on a thread and play *editor* on the client side.
Always receive with `recv_timeout` so a hang fails fast.

## Deliverables

`Cargo.toml` (+ committed `Cargo.lock`), `.gitignore`, `LICENSE-MIT`,
`LICENSE-APACHE`, `.github/workflows/ci.yml`, `src/{main,lib,server,capabilities,
document,position,dialect,parse}.rs` (parse.rs contains only `Span` for now),
`tests/e2e.rs`.

## TDD cycles

Each cycle ends in one commit with the four gates green
(`fmt`, `clippy -D warnings`, `test`, `doc`) — see `docs/development.md`.

### 0. Infrastructure (no tests — pure chore commits)

1. `chore: init cargo package with lints and dependencies`
   - `Cargo.toml`: name `csv-lsp`, version `0.1.0`, edition `2024`,
     `rust-version = "1.85"`, `license = "MIT OR Apache-2.0"`, description,
     repository; dependencies `lsp-server = "0.8"`, `lsp-types = "0.97"` (pin minor —
     it had breaking `Uri` churn), `serde_json = "1"`, `unicode-width = "0.2"`;
     `[lints.rust] unsafe_code = "forbid"`, `missing_docs = "warn"`.
   - Minimal `src/main.rs` + `src/lib.rs` that compile; `.gitignore` with `/target`;
     commit `Cargo.lock` (binary crate).
2. `chore: add mit and apache-2.0 license texts`
3. `ci: run fmt, clippy, tests and rustdoc on push` —
   `dtolnay/rust-toolchain@stable` + `Swatinem/rust-cache`; steps:
   `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`,
   `cargo test`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`.

### 1. `feat(position): index line starts over lf, crlf and lone cr`

- **Red** (`position.rs` unit tests): `LineIndex::new` records line-start byte
  offsets; cases: `""` → `[0]`; `"a\nb"` → `[0, 2]`; `"a\r\nb"` → `[0, 3]`;
  `"a\rb"` → `[0, 2]` (lone CR is a line break in LSP!); trailing `"a\n"` → `[0, 2]`.
- **Green**: byte scan; on `\r` peek for `\n`.

### 2. `feat(position): byte offset to lsp position in all three encodings`

- **Red**: for text `"id,x\né😀,名\n"` assert `position(offset_of('名'), enc)` for
  `Utf8`/`Utf16`/`Utf32` (compute expected columns by hand: é = 2/1/1 units,
  😀 = 4/2/1); offset at end of text; offset > len clamps.
- **Green**: `partition_point` over line starts → line; encode-width sum over
  `text[line_start..offset]` → character. Add `PositionEncoding` enum here.

### 3. `feat(position): lsp position to byte offset with spec clamping`

- **Red**: round-trips of cycle-2 cases; clamping per spec: `line` past EOF → last
  line; `character` past line end → line end (before the terminator); `character`
  landing inside a surrogate pair (utf-16) → start of that char. Add
  `range(span) -> lsp::Range` sanity case.
- **Green**: walk chars of the line accumulating encoding widths; stop at target or
  line-content end.

### 4. `feat(parse): byte span primitive`

- **Red** (`parse.rs`): `Span::slice(text)`, `len`, `is_empty`,
  `contains(offset)` (half-open), `overlaps(other)`.
- **Green**: plain struct + helpers, `#[derive(Clone, Copy, Debug, PartialEq, Eq)]`.

### 5. `feat(dialect): dialect enum with language-id and extension detection`

- **Red**: `delimiter()` per variant; `from_language_id("csv"|"tsv"|"ssv")`
  (case-insensitive, `None` otherwise); `from_path`: `data.csv` → Csv,
  `x.tsv`/`x.tab` → Tsv, `x.ssv` → Ssv, `x.txt`/no extension → None.
- **Green**: match on lowercased extension after the last `.` of the last `/` segment.

### 6. `feat(dialect): sniff delimiter from first non-blank line`

- **Red**: `sniff("a,b,c\n")` → Csv; tabs → Tsv; semicolons → Ssv;
  `"\"a,b\";c\n"` → Ssv (delimiters inside quotes don't count);
  `sniff("")`/blank-only → None; tie → Csv (documented bias; sniffing is the last
  resort after languageId and extension).
- **Green**: scan first non-blank line, count `,`/`\t`/`;` with a simple
  in-quotes toggle; max count wins.

### 7. `feat(document): document store with full-sync updates`

- **Red** (unit, no LSP): `Store::open(uri, language_id, version, text)` resolves the
  dialect (order: languageId → extension → sniff → Csv — one test per precedence
  step); `change` replaces text, bumps version, rebuilds the line index;
  `close` removes; `get` returns by uri.
- **Green**: `Document { uri, version, text, dialect, line_index }`,
  `Store(HashMap<String, Document>)` keyed by `uri.to_string()`.

### 8. `feat(capabilities): negotiate encoding and advertise capabilities`

- **Red**: `negotiate_position_encoding`: offers `[utf-16, utf-8]` → Utf8;
  `[utf-32]` → Utf32; `[]`/absent → Utf16. `server_capabilities(enc)`: sync is FULL
  with `open_close`, code-action kinds `[quickfix, source, source.fixAll]`,
  `resolve_provider == Some(false)`, formatting on, `position_encoding` echoed.
- **Green**: read `InitializeParams.capabilities.general.position_encodings`;
  construct `ServerCapabilities` (stubs will back the advertised methods in cycle 9 —
  a server must never advertise what it cannot answer).

### 9. `feat(server): initialize handshake and main loop with stub handlers`

- **Red** (`tests/e2e.rs`): build the `TestClient` helper —

  ```rust
  struct TestClient { conn: lsp_server::Connection, server: JoinHandle<…>, next_id: i32 }
  impl TestClient {
      fn start() -> Self;                     // Connection::memory() + thread::spawn(server::run)
      fn initialize(&mut self, encodings: &[PositionEncodingKind]) -> InitializeResult;
      fn request<R: lsp_types::request::Request>(&mut self, params: R::Params) -> R::Result;
      fn notify<N: lsp_types::notification::Notification>(&mut self, params: N::Params);
      fn recv_diagnostics(&mut self) -> PublishDiagnosticsParams;  // recv_timeout(5s)
      fn shutdown_and_join(self);             // shutdown request + exit notification + join
  }
  ```

  Test `initialize_negotiates_utf8`: offer `[utf-16, utf-8]`; assert
  `position_encoding == utf-8`, serverInfo name `csv-lsp`, capabilities as in cycle 8;
  clean shutdown (thread join returns `Ok`).
- **Green**: `server::run(connection)`: `initialize_start()` → deserialize params →
  negotiate → `initialize_finish(json!({capabilities, serverInfo}))` → loop:
  requests through `handle_shutdown` first, then dispatch — `codeAction` stub returns
  `Some(vec![])`, `formatting` stub returns `None`, anything else `MethodNotFound`;
  notifications and responses ignored for now. `main.rs`: `Connection::stdio()`,
  `run`, `io_threads.join()`.

### 10. `feat(server): document lifecycle publishes diagnostics`

- **Red** (e2e): `did_open_publishes_and_did_close_clears`: `didOpen`
  (`languageId: "csv"`, version 1, `"a,b\n1,2\n"`) → one `publishDiagnostics` with
  `version: Some(1)` and empty list; `didChange` (version 2) → publish with
  `version: Some(2)`; `didClose` → publish with empty list again.
- **Green**: wire `didOpen`/`didChange`/`didClose` into `Store`; `publish(doc)` sends
  the (for now always empty) diagnostics notification. Add the `CSV_LSP_LOG`-gated
  stderr `log!` macro and use it in the dispatch paths.

## Definition of done

- All four gates green locally and in CI.
- e2e: handshake negotiates utf-8 with Helix-like offers; lifecycle publishes and
  clears diagnostics; unknown request → `MethodNotFound`; clean shutdown.
- Manual (optional): `CSV_LSP_LOG=1 hx -v some.csv` with the README's
  `languages.toml` snippet shows the handshake in Helix's log.

## Gotchas

- Never print to stdout. Ever. It corrupts the protocol stream.
- `initialize_start/finish` — not the one-shot `initialize()` — because encoding
  negotiation needs the client capabilities *before* building ours.
- lsp-server answers `shutdown` itself inside `handle_shutdown` and waits for `exit`;
  don't answer it again.
- The `didChange` handler must take the *last* element of `contentChanges` (FULL sync
  clients send exactly one).
