# M5 — Quote cell / quote column

**Goal:** cursor-anchored refactors that wrap cells in RFC 4180 quotes: the cell
under the cursor, or a whole column at once. Useful before pasting delimiter-ish
content into cells, for defensive quoting of free-text columns, and as the
building block users asked for ("escape columns with quotes").

**Non-goals:** *un*quoting (backlog), quoting rows, auto-quoting on type.

## Background you need

**Quoting mechanics.** Wrapping a cell means replacing its `content_span` (the
padding-trimmed extent) with `encode_cell(value, dialect, force_quote: true)` —
which adds surrounding quotes and doubles any embedded ones. For the cells this
feature touches the value never contains quotes (a bare quote in an unquoted
cell makes the row a parse-error row, and those are skipped), so in practice it
is a pure wrap: ` a ` → ` "a" ` with the padding untouched. An **all-padding
cell** has a zero-width content span at its start, so quoting yields `""`
followed by the spaces — consistent with the trailing-padding model the
renderer uses.

**Which cells qualify.** Only `Unquoted` cells on clean rows: blank rows have
nothing to quote, parse-error rows are never rewritten (the crate-wide verbatim
principle), and already-quoted cells are no-ops the picker must not show.

**Refactor action kinds.** LSP groups actions by hierarchical kind strings.
Quoting rewrites content in place, which is `refactor.rewrite`
(`CodeActionKind::REFACTOR_REWRITE`). We currently advertise only
`quickfix`/`source`/`source.fixAll`, so `capabilities.rs` must add `refactor` +
`refactor.rewrite` — a server should not emit kinds it does not advertise, and
clients may filter by them (`only: ["refactor"]` must match via the existing
dotted-prefix logic).

**Column titles.** Cursor-anchored column actions read much better with the
header name in the title: `Quote column "name"`. The header cell's *value*
(decoded, truncated to ~24 chars) is the title; when the header row is shorter
than the target column, fall back to `column N` (1-based, as users count).

## Deliverables

`src/capabilities.rs` (kinds), `src/features/quote.rs` (both actions, one
provider), registration line, e2e test, README/architecture notes.

## TDD cycles

### 1. `feat(capabilities): advertise refactor action kinds`

- **Red**: capability test expects `REFACTOR` and `REFACTOR_REWRITE` in
  `code_action_kinds`.
- **Green**: extend the list.

### 2. `feat(features): quote-cell action`

- **Red** (`features/quote.rs`): cursor on unquoted `b` in `a,b\n` → action
  `Quote cell` (kind `refactor.rewrite`) whose single edit turns the doc into
  `a,"b"\n` (assert via `edits::apply`); cursor on a quoted cell → no quote-cell
  action; padded ` a ` → ` "a" ` (padding outside); empty cell → `""`; cursor
  on a blank row or a parse-error row → nothing.
- **Green**: provider resolving `ctx.cell_at_cursor()`, checking the row is
  clean (not blank, row index not in the error-row set) and the cell
  `Unquoted`; edit = replace `content_span` with
  `encode_cell(value, dialect, true)`. Registration line.

### 3. `feat(features): quote-column action`

- **Red**: mixed column `name\n"x"\ny\n` with cursor anywhere in it → action
  `Quote column "name"` whose edits touch only the header and `y` (row order,
  non-overlapping); header shorter than the column → title `column 3`; a fully
  quoted column → no column action; blank + error rows skipped.
- **Green**: same module; iterate rows in order collecting unquoted cells at
  the cursor column; title from the header cell value (truncate 24) or the
  fallback.

### 4. `test(e2e): quote column applies across all rows`

- **Red** (e2e): open a 3-row doc, request actions with `only=["refactor"]` →
  quote-cell + quote-column present (and *only* refactor kinds); apply the
  column action client-side, `didChange`; a reparse assertion via a follow-up
  request: the column action is no longer offered; diagnostics unchanged
  (quoting must not create or fix ragged rows).
- **Green**: nothing new expected — the cycle pins the protocol path
  (kind filtering, multi-edit application) forever.

### 5. `docs: quoting actions in readme`

README feature bullet + actions table row; architecture.md future-features list
updated (quote implemented, *unquote* explicitly listed as backlog).

## Definition of done

- Gates green; unit + e2e green.
- Manual in Helix: `space+a` on an unquoted cell shows `Quote cell` and
  `Quote column "…"`; applying either round-trips visibly and diagnostics stay
  put.

## Gotchas

- Replace the **content span**, not the cell span — quoting must not eat
  alignment padding.
- Multiple edits in one action must be emitted in document order and never
  overlap (distinct cells guarantee this; keep the iteration row-major anyway).
- Quoting an empty cell changes its width from 0 to 2 — aligned files may need
  re-aligning afterwards; that is expected, not a bug to compensate for.
- Truncate header-derived titles (a 300-char header cell must not explode the
  picker); truncation is display-only.
