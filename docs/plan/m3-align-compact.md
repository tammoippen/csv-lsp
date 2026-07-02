# M3 — Align & compact columns, README, release polish

**Goal:** whole-document layout features built on a lossless renderer: **align**
(pad cells so delimiters line up — via `:format` *and* a source action) and
**compact** (strip that padding — source action). Ship the README with complete
Helix setup. After M3 the MVP is done.

**Non-goals:** dialect transform / quote-column / add-delete-column (they reuse this
renderer — see `docs/architecture.md`), numeric right-alignment, config surface.

## Background you need

**Display width ≠ bytes ≠ chars.** Terminal editors render in character *cells*:
`é` is 1 column (2 UTF-8 bytes), `名` is 2 columns (3 bytes), `😀` is 2 columns
(4 bytes). Aligning by byte or char count misaligns any non-ASCII file; the
`unicode-width` crate implements the Unicode rules (UAX #11). Ambiguous-width and
ZWJ-emoji edge cases may still render off-by-one in some terminals — accepted,
documented.

**Alignment semantics** (from the parser's model — padding is *layout*, values are
the trimmed content):

```
compact:            aligned (target widths = max content width per column):
id,name,qty         id , name    , qty
1,"a,b",3     ⇄     1  , "a,b"   , 3
20,x,400            20 , x       , 400
```

- Quoted cells participate with their quotes (`"a,b"` is width 5); their *interior*
  is never touched.
- **Last column is never padded** (no trailing whitespace) — and generally a cell is
  padded only when a delimiter follows it, so short rows get no trailing spaces
  either.
- Blank rows stay empty lines; rows overlapping parse errors are emitted **verbatim**
  (never reformat what we couldn't parse).
- BOM / line terminator / final newline are reproduced (mixed terminators normalize
  to the first seen — documented).
- `QuotePolicy::Preserve` (MVP): emit each cell's `content_span` slice unchanged —
  align/compact are pure whitespace transforms; a `Required` variant re-encodes
  values (needed by the future dialect transform; `encode_cell` is written now,
  used by tests only).

**Formatting in LSP.** Client sends `textDocument/formatting` (Helix: `:format`, or
on save with `auto-format = true`); server returns `TextEdit[]` or null. The
`options` field (tab size etc.) is meaningless for CSV — ignored. **Idempotence is a
hard requirement**: formatting an aligned file must return no edits, or save-loops
ensue.

**Minimal edits.** Naively replacing the whole document works but scrolls/moves the
cursor in some clients. `edits::minimize(old, new)` trims the common prefix and
suffix (snapped to `char` boundaries) and returns at most one small `TextEdit` —
`[]` when `old == new`, which gives idempotence for free.

## TDD cycles

### 1. `feat(render): measure column display widths`

- **Red**: `column_widths` on `"id,name\n1,héllo\n999,名前\n"` → `[3, 5]`
  (col 0: max of 2/1/3; col 1: `name`=4, `héllo`=5 despite 6 bytes, `名前`=4 from
  2 chars); blank rows and error rows contribute nothing; a ragged long row still
  contributes to the columns it has; empty doc → `[]`.
- **Green**: `render.rs` with `column_widths(text, table)` over clean rows using
  `unicode_width::UnicodeWidthStr` on `content_span` slices.

### 2. `feat(render): compact rendering strips layout padding`

- **Red** (goldens, raw string literals — one small input per concern rather than a
  single mega-golden): (a) padding stripped around quoted and unquoted cells,
  (b) blank line preserved as empty line, (c) a row with an unclosed quote is
  emitted byte-for-byte verbatim, (d) CRLF preserved, (e) BOM preserved, (f) file
  without trailing newline stays without one.
- **Green**: `render(text, table, opts)` for `align: None`, `QuotePolicy::Preserve`;
  `RenderOptions::compact_for(&table)` convenience constructor mirroring the
  table's dialect/terminator/BOM/final-newline. Also implement `encode_cell`
  (RFC 4180: quote iff delimiter/quote/CR/LF present or forced; `"` → `""`) with its
  own unit tests — the renderer's `Required` policy and future features use it.

### 3. `feat(render): aligned rendering pads to column widths`

- **Red**: golden with mixed quoted/unquoted + CJK from cycle 1 — delimiters line
  up, last column unpadded, short row `20\n` gets no trailing spaces; property
  tests: `align(align(x)) == align(x)` and `compact(align(x)) == compact(x)` over
  the corpus + goldens.
- **Green**: the `align: Some(widths)` branch (pad a cell only when another cell
  follows in its row).

### 4. `feat(edits): minimal single-edit diff between renders`

- **Red**: `minimize(x, x) == []`; middle change → exactly one `(Span, String)`
  whose span slices the differing region; multibyte guard: `"aé,b"` vs `"aè,b"`
  snaps to char boundaries (no mid-é span); overlap guard: `minimize("aa", "a")`
  (suffix must not overlap prefix); insertion at EOF.
- **Green**: `edits.rs`: byte-wise common prefix, then common suffix over the
  remainder, boundary-snapped; plus `to_text_edits(edits, text, line_index, enc)`.

### 5. `feat(features): align and compact source actions`

- **Red**: `align::align_edits(doc)` on an unaligned doc → one edit; on the aligned
  output → `[]`; provider offers `Align columns` (kind `source`) only when edits are
  non-empty; same for `compact.rs` (`Compact columns`); both registered.
- **Green**: thin providers over `column_widths` + `render` + `minimize`.

### 6. `feat(server): document formatting aligns columns`

- **Red** (e2e): client that offers **only utf-16**; doc with an emoji cell
  (`"a,😀é\nlong,x\n"`); `textDocument/formatting` → edits; apply client-side (the
  M2 `apply_edits` helper, utf-16 math) → equals the aligned golden; formatting
  again → `null`/empty (idempotent); `codeAction` with `only=["source"]` lists
  `Align columns`/`Compact columns` appropriately.
- **Green**: replace the M0 formatting stub with `align::align_edits` conversion.

### 7. `test(e2e): stdio smoke test drives the real binary`

- **Red**: spawn `env!("CARGO_BIN_EXE_csv-lsp")` (cargo builds & points at the
  binary), speak raw framed JSON-RPC over its stdin/stdout: `initialize` →
  assert a framed `InitializeResult` comes back; `shutdown` + `exit` → process exits
  0 within a timeout. This is the only test of `main.rs`'s stdio glue.
- **Green**: usually nothing — the test exists to catch stdout pollution and
  framing regressions forever.

### 8. `docs: user guide with helix setup`

README sections (replaces the stub): what/why (2 paragraphs); feature tour with a
before/after align snippet; install (`cargo install --path .`, later
`--git`); **Helix setup** — the complete `languages.toml` below + `hx --health csv`;
dialect detection order (languageId → extension → sniff → comma); conventions
(header = first non-blank row = column contract; padding is layout, values are
trimmed, `"abc"  ,` is tolerated); diagnostics reference table (code / meaning /
severity / fix); logging (`CSV_LSP_LOG=1`, `hx -v`); development pointer to
`docs/`; license note.

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
auto-format = false
```

(If a Helix version ships a built-in `csv` language, keeping `name = "csv"`
identical makes our entry merge with/override it.)

## Definition of done

- Gates green; all cycles green (goldens, idempotence properties, utf-16 e2e, smoke).
- Manual in Helix: `:format` aligns a messy file; `space+a` → `Compact columns`
  restores it; align → compact → byte-identical to the original compact file
  (modulo documented terminator normalization); README instructions verified
  end-to-end by following them literally.

## Gotchas

- Compute widths from **content** spans but render padding *outside* quoted cells —
  both fall out of using `content_span` slices; don't be tempted to decode values
  here (`Preserve` means byte-preserving).
- Idempotence must hold through *parse → render*, i.e. the parser must treat the
  renderer's own output (padding after closing quotes!) as clean input — that
  tolerance was built in M1; the property test proves the loop closes.
- `minimize` on `Vec<u8>` prefixes is fine, but snap **both** ends to char
  boundaries of *both* strings (identical prefix bytes ⇒ identical boundaries, so
  snapping against `old` suffices — assert it in debug builds).
- Formatting must return `None`/empty for an already-aligned file — Helix with
  `auto-format` would otherwise mark the buffer dirty on every save.
