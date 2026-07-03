# csv-lsp

A language server for CSV, TSV, SSV (semicolon-separated) and PSV
(pipe-separated) files, built for [Helix](https://helix-editor.com/) but
working with any LSP client. Ships a tree-sitter grammar for rainbow
column colors on all four dialects.

Editing delimiter-separated files by hand is fiddly: a missing cell breaks the
column contract three screens later, quoting rules are easy to violate, and
nothing lines up. csv-lsp parses your file on every keystroke with an
error-tolerant parser and turns the problems into precise editor diagnostics —
with fixes attached.

## Features

- **Diagnostics** — rows with missing or extra cells (checked against the
  header), unclosed quotes, stray quotes, text after a closing quote. Exact
  spans, updated live.
- **Quickfix: pad short rows** — insert the missing empty cells at the row
  end, per row or file-wide (`source.fixAll`).
- **Align columns** — pad cells with spaces so delimiters line up under the
  header (`:format` or the `Align columns` source action):

  ```
  id,name,qty          id,name ,qty
  1,"a,b",3      ⇄     1 ,"a,b",3
  20,x,400             20,x    ,400
  ```

- **Compact columns** — strip that padding again (`Compact columns` source
  action). Align ⇄ compact round-trips byte-for-byte.
- **Reinterpret as CSV/TSV/SSV/PSV** — for files whose extension lies (a
  `.csv` that is actually semicolon-separated): switches how the server
  *parses* the file, zero text changes. Session-scoped; the durable fixes
  are renaming the file or converting it.
- **Convert to CSV/TSV/SSV/PSV** — rewrite the text to a different
  delimiter. Quoting adapts automatically (`bolzen;1,50` → `bolzen,"1,50"`),
  and the server keeps parsing under the new dialect after you apply it.
- **Quote cell / quote column** — wrap the cell under the cursor (or every
  unquoted cell of its column, header included) in RFC 4180 quotes. Padding
  stays outside the quotes; already-quoted cells are left alone.
- **Add / delete columns** — insert an empty column left or right of the one
  under the cursor, or delete it across the whole file (header included, so
  clean files stay clean; one undo restores everything).
- **Select a column** (Helix) — the server answers
  `textDocument/documentHighlight` with every cell of the cursor's column, and
  Helix's `Space+h` turns that into one selection per cell: change, append or
  pipe the whole column with ordinary multi-cursor editing.
- Unicode-aware alignment (CJK and accented characters measure by display
  width), BOM/CRLF/final-newline preservation, multi-line quoted cells.

## Install

```sh
git clone https://github.com/tammoippen/csv-lsp
cd csv-lsp
cargo install --path .
```

Requires stable Rust ≥ 1.85. The binary lands in `~/.cargo/bin/csv-lsp`.

## Helix setup

Add to `~/.config/helix/languages.toml`:

```toml
[language-server.csv-lsp]
command = "csv-lsp"

# Rainbow-column grammar shipped in this repository (see below; pin `rev`
# to a commit for reproducible builds).
[[grammar]]
name = "csv"
source = { git = "https://github.com/tammoippen/csv-lsp", rev = "main", subpath = "tree-sitter-rainbow-csv" }

[[language]]
name = "csv"
scope = "text.csv"
file-types = ["csv"]
language-servers = ["csv-lsp"]
auto-format = false            # true = align on every save

[[language]]
name = "tsv"
scope = "text.tsv"
file-types = ["tsv", "tab"]
language-servers = ["csv-lsp"]
grammar = "csv"                # tsv/ssv/psv reuse the csv grammar
auto-format = false

[[language]]
name = "ssv"
scope = "text.ssv"
file-types = ["ssv"]
language-servers = ["csv-lsp"]
grammar = "csv"
auto-format = false

[[language]]
name = "psv"
scope = "text.psv"
file-types = ["psv"]
language-servers = ["csv-lsp"]
grammar = "csv"
auto-format = false
```

Diagnostics, code actions and `:format` need nothing beyond the language
entries — csv-lsp works without any grammar. Verify with `hx --health csv`.

Daily driving: diagnostics appear as you type; `space`+`a` opens the code
actions (pad row, pad all, align, compact, reinterpret, convert, quote
cell/column, add/delete column); `:format` aligns; `space`+`h` selects the
column under the cursor (one selection per cell — empty cells become bare
cursors, ready for typing).

### Rainbow columns (syntax colors)

Helix highlights exclusively through tree-sitter — it has no LSP
semantic-token support, so csv-lsp (or any language server) cannot color
columns. Colors come from a grammar plus per-language queries, and this
repository ships both in
[`tree-sitter-rainbow-csv/`](tree-sitter-rainbow-csv/): the grammar Helix
bundles for csv derails on empty cells (`a,b,,,` breaks its parse and
neighboring rows change color), errors past the 7th column and does not
know tab delimiters; the vendored rewrite fixes all of that — same node
names, so existing themes and queries keep working.

With the `[[grammar]]` override above, build the parser once:

```sh
hx --grammar fetch && hx --grammar build
```

Helix resolves queries by *language* name. Install this repo's queries for
`csv` (they override the bundled ones and add the tab delimiter) and let
the other languages inherit them:

```sh
Q=~/.config/helix/runtime/queries
mkdir -p "$Q"/{csv,tsv,ssv,psv}
cp csv-lsp/tree-sitter-rainbow-csv/queries/highlights.scm "$Q"/csv/
for l in tsv ssv psv; do echo '; inherits: csv' > "$Q"/$l/highlights.scm; done
```

One caveat, cosmetic and shared with the bundled grammar: the grammar
splits on all four delimiters in every file (tree-sitter has no per-file
dialect detection), so a bare `,` inside an unquoted SSV cell starts a new
color. Quote the cell if it bothers you — csv-lsp's diagnostics always
follow the file's real dialect and are unaffected. Skipping the grammar
override also works: Helix ≥ 25.07 still rainbows csv/ssv/psv through the
bundled grammar, with its empty-cell color shifts; tsv rows render as a
single column.

## Dialects and conventions

| Dialect | Delimiter | Extensions |
|---|---|---|
| csv | `,` | `.csv` |
| tsv | tab | `.tsv`, `.tab` |
| ssv | `;` | `.ssv` |
| psv | pipe | `.psv` |

- Dialect detection order: LSP `languageId` → file extension → content
  sniffing (delimiters counted outside quotes in the first non-blank line,
  ties favor comma) → comma. The dialect is fixed when a file is opened.
- **Reinterpretation** (`Reinterpret as …`) overrides that detection for the
  current session only — on reopen the extension wins again. Rename the file
  (or convert its content) for a durable fix.
- **Conversion** (`Convert to …`) rewrites in place and emits the compact
  form (re-align afterwards if you like). It is only offered on files
  without quoting errors, and renaming the file afterwards is up to you.
- **The header is the first non-blank row**; its cell count is the column
  contract the rest of the file is checked against.
- Blank lines are separators: legal, never padded, never counted.
- **Padding is layout, not data**: cell values are space-trimmed on parse,
  `Compact columns` removes padding, and quoted cell *interiors* are never
  touched. Space between a closing quote and the delimiter (`"abc"  ,`) is
  tolerated — align produces exactly that.
- Quoting follows RFC 4180: quoted cells may contain delimiters, line breaks
  and `""` escapes.
- Rows the parser could not fully understand are excluded from column checks
  and passed through **verbatim** by align/compact.
- Mixed line terminators are normalized to the file's first one when
  align/compact rewrite the document.

## Diagnostics reference

| Code | Meaning | Severity | Fix |
|---|---|---|---|
| `row-missing-cells` | fewer cells than the header | error | quickfix: pad with empty cells |
| `row-extra-cells` | more cells than the header | error | manual (deleting data is not auto-fixed) |
| `unclosed-quote` | quoted cell never closed | error | manual |
| `stray-quote` | bare `"` in an unquoted cell | warning | quote the cell or double the quote |
| `text-after-quote` | text after a closing quote | error | manual |

## Logging

stdout is the protocol channel; logs go to stderr, gated by an environment
variable:

```sh
CSV_LSP_LOG=1 hx -v data.csv    # then check ~/.cache/helix/helix.log
```

## Development

The documentation lives in [`docs/`](docs/) as self-contained HTML pages —
clone the repo and open `docs/index.html` in a browser (no build step, no
network needed):

- `docs/quickstart.html` — install, Helix setup, a sixty-second tour
- `docs/features.html` — every feature in depth, plus the GIF recording kit
  (`docs/gifs/`)
- `docs/lsp.html` — protocol wiring, dataflow, position encodings
- `docs/parser.html` — the error-tolerant parser and its invariants
- `docs/testing.html` — the test pyramid and property-based testing
- `docs/contributing.html` — structure, design decisions, workflow

The short version: everything below `main.rs` is a library; all offsets are
byte spans converted to LSP positions at one boundary; and **adding a feature
= one new module in `src/features/` + one line in `Registry::standard()`**.

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

Known limits: alignment of ambiguous-width/ZWJ-emoji cells may render
off-by-one in some terminals (UAX #11); very large files reparse on every
keystroke (fine into the tens of MB).

## License

MIT
