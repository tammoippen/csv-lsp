# Architecture

csv-lsp is a language server for delimiter-separated files (CSV/TSV/SSV). It is a
single Rust binary speaking the Language Server Protocol (LSP) over stdio, built on a
**synchronous** main loop (`lsp-server`) and a **hand-written, error-tolerant CSV
parser**. Everything below `main.rs` lives in the library target so tests can exercise
it directly.

Design invariant: **all internal offsets are byte offsets** into the document text.
Conversion to LSP `(line, character)` positions happens in exactly one place
(`position.rs`), parametrized by the position encoding negotiated with the client.

## Module map

```
src/
├── main.rs               binary shim: stdio connection → server::run() → join io threads
├── lib.rs                module tree; lint configuration; re-exports for tests
├── server.rs             ServerState, main message loop, dispatch, diagnostics publishing
├── capabilities.rs       position-encoding negotiation + ServerCapabilities construction
├── document.rs           Document (text + version + dialect + parse cache), Store (open docs)
├── position.rs           PositionEncoding, LineIndex, Span ↔ lsp::Range conversion
├── dialect.rs            Dialect (Csv|Tsv|Ssv), detection: languageId → extension → sniff → Csv
├── parse.rs              Span, Table/Row/Cell, ParseError, the state-machine parser
├── render.rs             Table → text: render(), column_widths(), encode_cell()
├── edits.rs              (Span, String) edits → lsp TextEdits; whole-doc diff minimization
└── features/
    ├── mod.rs            Diag/Action types, DiagnosticRule + ActionProvider traits,
    │                     ActionContext, Registry::standard()   ← the ONE registration point
    ├── parse_errors.rs   rule: parser errors → diagnostics (quoting problems)
    ├── ragged_rows.rs    rule: per-row cell count vs. header
    ├── pad_rows.rs       action: quickfix "pad row with empty cells" + source.fixAll
    ├── align.rs          action + formatting: align columns
    └── compact.rs        action: remove alignment padding
tests/
└── e2e.rs                black-box protocol tests over lsp_server::Connection::memory()
```

**Extensibility rule:** feature N+1 = one new file in `src/features/` + one line in
`Registry::standard()`. A capability tweak in `capabilities.rs` is only needed when a
feature introduces a new LSP *method* (rare — most features are code actions).

## Data model (`parse.rs`)

```rust
pub struct Span { pub start: usize, pub end: usize }   // byte offsets, half-open

pub enum Dialect { Csv, Tsv, Ssv }                     // delimiters: b','  b'\t'  b';'
pub enum LineTerminator { Lf, CrLf }                   // first terminator seen in the file

pub struct Table {
    pub rows: Vec<Row>,
    pub errors: Vec<ParseError>,
    pub dialect: Dialect,
    pub line_terminator: LineTerminator,
    pub has_bom: bool,
    pub ends_with_newline: bool,
}

pub struct Row { pub span: Span, pub cells: Vec<Cell> }  // span EXCLUDES the line terminator

pub struct Cell {
    pub span: Span,           // full extent incl. alignment padding, excl. delimiters
    pub content_span: Span,   // padding-trimmed; for quoted cells INCLUDES the quotes
    pub quoting: Quoting,     // Unquoted | Quoted
    pub has_escaped_quotes: bool,   // "" occurred → value() must allocate to unescape
}

pub struct ParseError { pub kind: ParseErrorKind, pub span: Span, pub row: usize }
pub enum ParseErrorKind { UnclosedQuote, StrayQuote, TextAfterClosingQuote }

pub fn parse(text: &str, dialect: Dialect) -> Table;   // total: never fails, never panics
```

Invariants:

- `cell.content_span ⊆ cell.span ⊆ row.span`; all spans lie on `char` boundaries
  (delimiter/quote/CR/LF are ASCII and can never occur inside a UTF-8 multibyte
  sequence, so a byte-wise scanner is safe).
- Padding is *derived*, not stored: leading = `span.start..content_span.start`,
  trailing = `content_span.end..span.end`. Padding is ASCII space only.
- `Cell::value(&text) -> Cow<str>` decodes lazily; it only allocates when `""`
  unescaping is required.
- A `Row` is *blank* iff it has exactly one unquoted cell with empty content. Blank
  rows are legal (no diagnostic), skipped by column-count checks, and rendered as
  empty lines.
- The **header is the first non-blank row**; its cell count is the expected column
  count for the whole file.
- A row that overlaps a `ParseError` is excluded from column-count checks (one broken
  quote must not cascade into dozens of ragged-row errors) and is passed through
  **verbatim** by the renderer.

## Parser

Byte-wise state machine per cell: `CellStart → InUnquoted | InQuoted → AfterQuoted`.
See `docs/plan/m1-parser-and-diagnostics.md` for the full transition table, error
taxonomy, and recovery rules. Key policies:

| Topic | Policy |
|---|---|
| Quoted cells | RFC 4180: may contain delimiter, `""` escapes, and **newlines** (a row can span lines) |
| Errors | `UnclosedQuote` (Error), `TextAfterClosingQuote` (Error), `StrayQuote` (Warning — common in real data: `5" bolt`) |
| Space after closing quote | tolerated silently (`"abc"  ,` is what our own align feature produces) |
| BOM | recorded, spans start after it, re-emitted on render |
| CRLF / LF / lone CR | all accepted; first-seen terminator is recorded and used for re-rendering (mixed files get normalized — documented) |
| Trailing final newline | recorded as a flag, never a phantom empty row |

## Positions and encodings (`position.rs`)

LSP positions are `(line, character)` where *character* counts **units of the
negotiated encoding** (UTF-16 by default, for historical reasons). We negotiate
UTF-8 when the client offers it (Helix does), else UTF-32, else UTF-16 — and implement
all three: `LineIndex` stores line-start byte offsets (splitting on `\n`, `\r\n`, and
lone `\r`, matching the LSP spec) and converts `Span ↔ lsp::Range`.

## LSP wiring (`server.rs`, `capabilities.rs`)

- Handshake via `initialize_start()/initialize_finish()` so client capabilities are
  known before ours are built.
- Advertised capabilities: `positionEncoding` (negotiated), `textDocumentSync = FULL`
  (+ open/close), `codeActionProvider` with kinds `[quickfix, source, source.fixAll,
  refactor, refactor.rewrite]` (no lazy resolve — edits are cheap and computed
  eagerly), `documentFormattingProvider`, `documentHighlightProvider` (the
  cursor's column as per-cell ranges — Helix's `Space+h` turns them into a
  column multi-selection), and one `executeCommand`: `csv-lsp.setDialect`.
- Text actions carry a complete `WorkspaceEdit` (`changes` map form — no
  client→server round trip). `executeCommand` exists solely for actions that
  change **server state** instead of text: `Reinterpret as …` flips a
  document's parsing dialect and republishes diagnostics.
- Applied conversions flip the dialect too: `Convert to …` actions are
  remembered per URI (converted text + target dialect) and a `didChange`
  matching one adopts that dialect — dismissed actions cost nothing.
- Diagnostics use the push model: reparse + publish on `didOpen`/`didChange`; publish
  an empty list on `didClose` (clears the editor gutter).
- Request handlers are wrapped in `catch_unwind`: a bug in one feature answers that
  one request with an error instead of killing the server.
- stdout carries protocol frames only; **all logging goes to stderr**, gated by
  `CSV_LSP_LOG=1` (Helix surfaces it via `hx -v` in its log file).

## Feature framework (`features/mod.rs`)

```rust
pub struct Diag {           // internal diagnostic; converted to lsp at the boundary
    pub span: Span,
    pub severity: Severity,             // Error | Warning | Info | Hint
    pub code: &'static str,             // "row-missing-cells", "unclosed-quote", ...
    pub message: String,
    pub data: Option<serde_json::Value>,
}
pub trait DiagnosticRule {
    fn name(&self) -> &'static str;
    fn check(&self, doc: &Document) -> Vec<Diag>;
}

pub struct ActionContext<'a> {
    pub doc: &'a Document,
    pub range: Span,                                     // request range in bytes
    pub client_diagnostics: &'a [lsp_types::Diagnostic], // linkage only, never trusted
    pub only: Option<&'a [lsp_types::CodeActionKind]>,   // client's kind filter
}
pub trait ActionProvider {
    fn name(&self) -> &'static str;
    fn actions(&self, ctx: &ActionContext) -> Vec<Action>;
}
pub struct Action {
    pub title: String,
    pub kind: lsp_types::CodeActionKind,
    pub edits: Vec<(Span, String)>,     // replace span with string
    pub fixes: Vec<Diag>,               // populates CodeAction.diagnostics
    pub is_preferred: bool,
}

pub struct Registry { /* Vec<Box<dyn DiagnosticRule>>, Vec<Box<dyn ActionProvider>> */ }
```

Providers **recompute applicability from the parsed `Table`** — they never depend on
the client echoing diagnostic payloads back. `client_diagnostics` only enriches the
response.

## Renderer (`render.rs`) and edits (`edits.rs`)

```rust
pub struct RenderOptions {
    pub dialect: Dialect,               // target delimiter (enables future transform)
    pub align: Option<Vec<usize>>,      // Some(col display widths) = pad; None = compact
    pub quote_policy: QuotePolicy,      // Preserve (MVP) | Required (future transform)
    pub line_terminator: LineTerminator,
    pub include_bom: bool,
    pub final_newline: bool,
}
pub fn render(text: &str, table: &Table, opts: &RenderOptions) -> String;
pub fn column_widths(text: &str, table: &Table) -> Vec<usize>;  // unicode display width
pub fn encode_cell(value: &str, dialect: Dialect, force_quote: bool) -> String;
```

Align and compact are the *same* pipeline with `align: Some(widths)` vs `None`.
`edits::minimize(old, new)` turns a full re-render into at most one small
`TextEdit` by trimming the common prefix/suffix (char-boundary-snapped), which keeps
the editor cursor stable and makes formatting idempotent (`[]` when already aligned).

## Backlog

All features from the original scope are implemented. Candidates for later:

- **Unquote cell/column** — the inverse of the quote actions, only where the
  value survives unquoted.
- **Extra-cells quickfix** — merging or dropping surplus cells (deleting data
  needs more care than adding empty cells).
- **Configuration** via `initializationOptions`: severity overrides,
  blank-line policy, header conventions.
- **Incremental sync** (isolated in `Document`) if very large files ever make
  FULL sync noticeable.
- **Rename-file-on-convert** via LSP `RenameFile` resource operations, where
  clients support them.

## Performance stance

Full reparse on every keystroke is O(n) and fine into the tens of MB; `FULL` document
sync keeps the server trivial. The isolated escape hatches, if ever needed, are
incremental sync in `Document` and a size threshold for align. Do not optimize before
that shows up in practice.
