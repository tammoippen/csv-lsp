# M1 — Error-tolerant parser + real diagnostics

**Goal:** the span-rich CSV parser (the project's core asset) and the diagnostic rule
framework, wired into the server: opening a broken CSV in the editor shows precise,
useful squiggles that update on every keystroke.

**Non-goals:** no fixes yet (M2), no rendering (M3).

## Background you need (CSV in 10 minutes)

RFC 4180 defines CSV: records separated by line breaks, fields separated by `,`.
A field may be **quoted** with `"…"`; inside quotes, delimiters, line breaks and
escaped quotes (`""` → one literal `"`) are content. So this is ONE record with two
fields — a row can span multiple lines:

```
"first
line",second
```

Consequence: **row ≠ line**. All our structures use byte spans; the `LineIndex` from
M0 maps spans to editor lines, so multi-line cells cost nothing extra.

We support three dialects that differ *only* in the delimiter byte: CSV `,`
TSV `\t`, SSV `;`. Quoting rules are identical. (In SSV — common in
German-locale exports — `1,5` is a decimal number and `;` separates fields.)

Real-world files deviate from the RFC. We parse **totally** (never fail) and report
deviations as diagnostics with exact spans:

| Kind | Example | Error span | Recovery | Severity |
|---|---|---|---|---|
| `UnclosedQuote` | `"abc` … EOF | the opening `"` (1 byte) | rest of file becomes the cell's content | Error |
| `StrayQuote` | `5" bolt` | the stray `"` | quote is literal content, keep scanning | Warning (extremely common in the wild) |
| `TextAfterClosingQuote` | `"x" y,z` | the garbage run (`y`) | garbage skipped, cell value stays `x` | Error |

**Cell anatomy.** Aligned files pad cells with spaces so delimiters line up. Padding
is *layout*, not data — the model separates them. For the row `id, "a""b" ,x`:

```
offset:   0    3          12
          id , · " a "" b " · , x        (· = space)
cell 1:      span 3..11  = ` "a""b" `    (includes padding)
             content 4..10 = `"a""b"`    (quotes included for quoted cells)
             value        = `a"b`        (decoded lazily, Cow)
```

- All-padding cell → zero-width content span at `span.start` (padding counts as
  trailing — consistent with left-aligned columns).
- Space between a closing quote and the delimiter is **silently tolerated** — our own
  align feature (M3) produces exactly that; it must not be an error.
- Padding is ASCII space only (a tab is the TSV delimiter; inside CSV cells it is
  content).

**Structural policies** (decided; see `docs/architecture.md`):

- Header = first **non-blank** row; its cell count is the expected column count.
- Blank row (exactly one unquoted, empty cell) = separator: no diagnostics, excluded
  from checks, rendered as an empty line.
- Rows overlapping a parse error are excluded from column-count checks (one unclosed
  quote must not cascade) — `ParseError.row` exists for this.
- BOM (`EF BB BF`): flag + skip; spans never include it.
- First terminator seen (`\r\n` vs `\n`; lone `\r` counts as `\n`) is recorded for
  re-rendering. Trailing final newline = flag, not a phantom empty row.

**Diagnostics in LSP.** Server → client notification
`textDocument/publishDiagnostics { uri, version, diagnostics: [{ range, severity,
code, source, message, data }] }`. Push model: we publish after every
open/change; each publish **replaces** the previous set for that file. `data` is an
arbitrary payload the client echoes back in code-action requests — we fill it for
context but **never rely on it** (M2 recomputes everything server-side).

## Parser state machine (per cell)

```
CellStart ──space──▶ CellStart                    (leading padding)
CellStart ──"──────▶ InQuoted
CellStart ──delim/EOL/EOF─▶ emit empty cell
CellStart ──other──▶ InUnquoted

InUnquoted ──delim/EOL/EOF─▶ emit (content = raw minus outer spaces)
InUnquoted ──"─────▶ record StrayQuote; quote is content; stay

InQuoted ──""──────▶ escaped quote (flag cell); stay
InQuoted ──"───────▶ InAfterQuoted (content ends after this quote)
InQuoted ──CR/LF───▶ content (multi-line cell); stay
InQuoted ──EOF─────▶ record UnclosedQuote at the opening quote; emit cell
InQuoted ──other───▶ content; stay

InAfterQuoted ──space──────▶ stay                 (tolerated: alignment layout)
InAfterQuoted ──delim/EOL/EOF─▶ emit cell
InAfterQuoted ──other──────▶ record TextAfterClosingQuote; skip to delim/EOL/EOF
                              (error span = garbage trimmed of trailing spaces)
```

After each cell: delimiter → next cell, same row; `\r\n`/`\n`/`\r` → row ends (row
span excludes the terminator); EOF → row ends, and `ends_with_newline` stays false.

Safety property making byte-wise scanning legal: delimiter/quote/CR/LF are ASCII,
and ASCII bytes never occur inside a UTF-8 multibyte sequence → every span boundary
is a `char` boundary.

## TDD cycles

### 1. `feat(parse): unquoted cells and lf rows with exact spans`

- **Red**: `"a,b,cc\n1,,2\n"` → 2 rows × 3 cells; assert via slices:
  `rows[0].cells[2].span.slice(text) == "cc"`; empty middle cell has zero-width
  span; row spans exclude `\n`; `ends_with_newline`; `"a"` (no newline) → 1 row,
  `ends_with_newline == false`; `parse("")` → 0 rows.
- **Green**: `Table`/`Row`/`Cell`/`Quoting` structs; scanner for
  `CellStart→InUnquoted` + delimiter + `\n` only. `content_span == span` for now.

### 2. `feat(parse): trim alignment padding into content spans`

- **Red**: `" a , bb ,c \n"`: cell spans cover the padding, content spans slice to
  `"a"`, `"bb"`, `"c"`; all-space cell `"  "` → content zero-width **at
  `span.start`**; `"a b"` keeps its interior space.
- **Green**: track first/last non-space while scanning.

### 3. `feat(parse): quoted cells with rfc 4180 escapes`

- **Red**: `"\"a,b\",c\n"` → 2 cells, first quoted with content `"a,b"` (quotes
  included in `content_span`), `value() == "a,b"` and is `Cow::Borrowed`;
  `"\"x\"\"y\"\n"` → `has_escaped_quotes`, `value() == "x\"y"`, `Cow::Owned`;
  padding around quoted cell ` "q" ,z` → no error, content `"q"`.
- **Green**: `InQuoted`/`InAfterQuoted` happy paths; `Cell::value` with lazy
  unescape.

### 4. `feat(parse): line terminators, bom and multi-line quoted cells`

- **Red**: `"\u{feff}a,b\r\n\"x\ny\",2\r\n"` → `has_bom`, first cell span starts at
  byte 3, terminator `CrLf`, row 1 contains a cell whose value is `"x\ny"` (row
  spans two editor lines); `"a\rb\n"` → lone CR ends a row, terminator recorded as
  `Lf` family, 2 rows.
- **Green**: BOM skip, CR/CRLF handling, first-terminator recording. (Newlines
  inside quotes already work — they never reach the terminator logic.)

### 5. `feat(parse): recover from unclosed quotes`

- **Red**: `"a,\"bc\nd"` → 1 row, 2 cells, and exactly 1 error `UnclosedQuote` with
  span = the opening `"` byte; the quoted cell's content runs to EOF
  (`value() == "bc\nd"`), `ends_with_newline == false` (that `\n` is cell content,
  not a row terminator!).
- **Green**: EOF arm of `InQuoted`.

### 6. `feat(parse): recover from stray quotes and trailing garbage`

- **Red**: `"5\" bolt,x\n"` → StrayQuote (Warning-class) at byte 1, cell value is
  literally `5" bolt`; `"\"x\" y,z\n"` → TextAfterClosingQuote with span slicing to
  `"y"`, cell value `x`, cell span covers `"x" y`; error's `row` index is set.
- **Green**: stray-quote arm + garbage skip with trailing-space-trimmed span.

### 7. `feat(parse): blank-row semantics and table lookup helpers`

- **Red**: `"\na,b\n\n1,2\n"` → `rows[0].is_blank()`, `header()` is the `a,b` row,
  `expected_columns() == Some(2)`; `row_at`/`cell_at`: offset in the middle of a
  cell → `(row, col)`; offset **on** a delimiter → the cell left of it; offset at
  row end (on the `\n`) → last cell of that row (inclusive end — cursor at EOL);
  offset past EOF → `None`.
- **Green**: `is_blank`, `header`, `expected_columns`, binary-search lookups with
  inclusive row end.

### 8. `test(parse): corpus proves the parser is total`

- **Red**: a `const CORPUS: &[&str]` of ~20 nasty snippets (`"`, `"""`, `a"b"c`,
  `",\n\r`, `"\r\r\n`, BOM-only, emoji straddling quotes, 10-cell row, `\0` byte, …)
  parsed under all three dialects; assert only: no panic; every span within bounds
  and on char boundaries (`text.is_char_boundary`); `content_span ⊆ span ⊆
  row.span`; errors' `row < rows.len()`.
- **Green**: fix whatever it flushes out (typically off-by-ones at EOF).

### 9. `feat(features): diagnostic rule framework with quoting-error rule`

- **Red** (`features/` unit): `parse_errors` rule maps the three `ParseErrorKind`s to
  `Diag`s with codes `unclosed-quote` / `stray-quote` / `text-after-quote`,
  severities Error/Warning/Error, spans passed through, human messages ("quoted cell
  is never closed", …).
- **Green**: `Diag`, `Severity`, `DiagnosticRule` trait, `Registry` (rules only for
  now) with `Registry::standard()`, `parse_errors.rs`.

### 10. `feat(features): ragged-row rule against the header column count`

- **Red**: header `a,b,c`; row `1,2` → code `row-missing-cells`, **zero-width span at
  the row's end**, message `row has 2 cells, expected 3`, `data == {"row":1,
  "missing":1}`; row `1,2,3,4` → `row-extra-cells`, span from the first extra cell to
  row end, `data.extra == 1`; blank rows and rows with parse errors produce nothing;
  a file with only a header produces nothing.
- **Green**: `ragged_rows.rs` using `expected_columns()` + `ParseError.row` skip set.

### 11. `feat(server): publish real diagnostics from the registry`

- **Red** (e2e): open `"a,b,c\n1,2\n"` → one diagnostic: code `row-missing-cells`,
  severity Error, `source == "csv-lsp"`, range at line 1's end (utf-8 encoding);
  `didChange` to `"a,b,c\n1,2,3\n"` → empty diagnostics; a quoting error appears with
  its code too.
- **Green**: `Document` gains `table: Table` (parsed in open/change);
  `publish` runs `Registry::standard().diagnostics(doc)` and converts
  `Diag → lsp_types::Diagnostic` (span→range via LineIndex + negotiated encoding,
  severity map, `code`, `data`).

### 12. `feat(server): harden request dispatch against panics and bad params`

- **Red** (e2e): a `codeAction` request with structurally invalid params → error
  response (`InvalidParams`), and the server **still answers** a following request;
  unknown method → `MethodNotFound` (may already pass — keep as regression).
- **Green**: params deserialization errors → `InvalidParams`; wrap handler calls in
  `catch_unwind(AssertUnwindSafe(…))` → `InternalError` response; panic hook logs to
  stderr.

## Definition of done

- Gates green; all cycles' tests green including the corpus.
- Manual: open a ragged CSV in Helix — squiggles appear at row ends with readable
  messages, update live while typing, and a multi-line quoted cell shows the
  diagnostic on the correct editor line.

## Gotchas

- `ends_with_newline` must come from the parser's terminator handling, **not**
  `text.ends_with('\n')` — a trailing newline inside an unclosed quote is content.
- Zero-width ranges (`start == end`) are legal LSP and render as a caret/EOL mark.
- Keep `ParseError.row` correct when the error occurs mid-row (it is `rows.len()` *at
  emission time* only if the row is pushed after its errors — safer: pass the row
  index into the cell parser).
- Diagnostic **message style**: state the fact + the expectation
  (`row has 2 cells, expected 5`), lowercase, no trailing period — consistent across
  rules.
