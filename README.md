# csv-lsp

A language server for CSV, TSV and SSV (semicolon-separated) files, built for
[Helix](https://helix-editor.com/) but working with any LSP client.

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
- **Reinterpret as CSV/TSV/SSV** — for files whose extension lies (a `.csv`
  that is actually semicolon-separated): switches how the server *parses*
  the file, zero text changes. Session-scoped; the durable fixes are
  renaming the file or converting it.
- **Convert to CSV/TSV/SSV** — rewrite the text to a different delimiter.
  Quoting adapts automatically (`bolzen;1,50` → `bolzen,"1,50"`), and the
  server keeps parsing under the new dialect after you apply it.
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
auto-format = false

[[language]]
name = "ssv"
scope = "text.ssv"
file-types = ["ssv"]
language-servers = ["csv-lsp"]
grammar = "csv"                # rainbow columns — reuse the built-in csv grammar
auto-format = false
```

Diagnostics, code actions and `:format` need nothing beyond these entries —
csv-lsp works without any grammar. Keeping `name = "csv"` identical to the
built-in language makes the entry merge with it. Verify with
`hx --health csv`.

Daily driving: diagnostics appear as you type; `space`+`a` opens the code
actions (pad row, pad all, align, compact, reinterpret, convert, quote
cell/column, add/delete column); `:format` aligns; `space`+`h` selects the
column under the cursor (one selection per cell — empty cells become bare
cursors, ready for typing).

### Rainbow columns (syntax colors)

Helix highlights exclusively through tree-sitter — it has no LSP
semantic-token support, so csv-lsp (or any language server) cannot color
columns. The rainbow on `.csv` files comes from the `csv` grammar and
queries Helix ships since 25.07; the entry above merges with that built-in
language by name and keeps them.

- **ssv** — the built-in grammar splits on `;` and `|` too, which is what
  `grammar = "csv"` above taps into. Helix resolves query files by
  *language* name, so a new language needs its own queries — one line,
  inheriting the bundled ones:

  ```sh
  mkdir -p ~/.config/helix/runtime/queries/ssv
  echo '; inherits: csv' > ~/.config/helix/runtime/queries/ssv/highlights.scm
  ```

- **tsv** — no rainbow: the grammar does not treat tab as a delimiter.
  Everything csv-lsp provides works regardless.
- **Empty cells shift colors** — the grammar cannot represent empty cells
  (and errors past the 7th column), so a row like `a,b,,,` fails to parse
  and tree-sitter's error recovery bleeds the column cycle into
  neighboring rows. That is an upstream bug in
  [weartist/rainbow-csv-tree-sitter] (the grammar Helix pins), out of
  reach of csv-lsp and of custom query files alike — it needs a grammar
  fix.

[weartist/rainbow-csv-tree-sitter]: https://github.com/weartist/rainbow-csv-tree-sitter

## Dialects and conventions

| Dialect | Delimiter | Extensions |
|---|---|---|
| csv | `,` | `.csv` |
| tsv | tab | `.tsv`, `.tab` |
| ssv | `;` | `.ssv` |

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

See `docs/architecture.md` (module map and data model),
`docs/development.md` (TDD workflow, commit conventions, quality gates),
`docs/adr/` (the language/library/parser decisions) and `docs/plan/` (the
milestone plans this codebase was built from).

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
