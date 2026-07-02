# M6 — Add / delete column at the cursor

**Goal:** structural column editing: insert an empty column left or right of the
column under the cursor, or delete that column across the whole file — the last
two features from the original request.

**Non-goals:** moving/reordering columns, multi-column selection, undo prompts
(LSP has none — a single editor undo restores everything).

## Background you need

**Span arithmetic on the cell model.** The parser gives every cell a `span`
covering its full extent *including* alignment padding and *excluding* the
delimiters around it. That makes column edits pure offset arithmetic:

```
row:      · a ·, · b ·, c          (· = space, cells: [ a ], [ b ], [c])
add left  of column 1: insert "," at cells[1].span.start   → · a ·,, · b ·, c
add right of column 1: insert "," at cells[1].span.end     → · a ·, · b ·,, c
delete       column 1: remove cells[0].span.end .. cells[1].span.end
                                                            → · a ·, c
```

- Inserting **one delimiter** at a cell boundary creates one empty cell —
  before any leading padding (add-left) or after any trailing padding
  (add-right), so aligned files stay tidy.
- Deleting column `c > 0` removes the *preceding* delimiter plus the cell
  (`cells[c-1].span.end .. cells[c].span.end`); deleting column 0 with ≥ 2
  cells removes the cell plus the *following* delimiter
  (`cells[0].span.start .. cells[1].span.start`); a single-cell row just has
  its cell content removed (the row becomes blank).

**Which rows participate.** Clean rows **that have the column**: blank rows and
parse-error rows are skipped (the verbatim principle), and rows shorter than
the target column are also skipped — they are already flagged ragged and keep
their pad quickfix; guessing where their missing column "would be" helps
nobody.

**The contract shifts coherently.** Because the header row is edited too, the
expected column count grows/shrinks with the edit: a fully clean file is still
fully clean after add or delete (worth asserting end-to-end). Pre-existing
short rows simply stay short.

**Cursor resolution** reuses `ctx.column_at_cursor()` (M2): a cursor on a
delimiter belongs to the cell left of it; a cursor at the row end to the last
cell. Titles reuse M5's header-name helper: `Add column left of "name"`,
`Delete column "qty"`. Kind: plain `refactor` (structure change, not a
rewrite).

## Deliverables

`src/features/columns.rs` (one provider, three actions; shares the
`column_title` helper with M5 — hoist it to `features/mod.rs` if M5 landed it
locally), registration line, e2e round-trip test, README/architecture backlog
rewrite.

## TDD cycles

### 1. `feat(features): add column left and right of the cursor`

- **Red** (`features/columns.rs`): on `a,b\n1,2\n` with the cursor in column 1,
  `Add column left of "b"` applies to `a,,b\n1,,2\n` and `Add column right of
  "b"` to `a,b,\n1,2,\n` (assert via `edits::apply`); first/last column edges;
  aligned file ` a , b \n` keeps padding on the correct side of the new
  delimiter; cursor on the delimiter edits column 0's right side; a short row
  (`x\n` under a 2-column header) receives no edit from column-1 actions; blank
  and error rows untouched; titles use the header value.
- **Green**: provider resolving the column, emitting both add actions (kind
  `REFACTOR`) with per-row inserts over qualifying rows; `column_title`
  helper; registration.

### 2. `feat(features): delete the column under the cursor`

- **Red**: deleting column 1 of `a,b,c\n1,2,3\n` yields `a,c\n1,3\n` (preceding
  delimiter consumed); deleting column 0 yields `b,c\n2,3\n` (following
  delimiter consumed); a quoted cell `"x,y"` is removed wholesale; deleting the
  only column of a one-column file leaves blank lines; short rows keep their
  text; title `Delete column "b"`.
- **Green**: the delete action alongside the adds, using the three-case span
  rule from the background section.

### 3. `test(e2e): column edits round trip`

- **Red** (e2e): open a clean file; `Add column right of` column 0 → apply →
  `didChange` → diagnostics still empty (header moved with the rows); then
  `Delete column` at the new empty column → apply → text is **byte-identical**
  to the original; both action sets requested with `only=["refactor"]`.
- **Green**: nothing new expected; the cycle pins insertion/deletion symmetry.

### 4. `docs: column editing in readme`

README: feature bullets + actions reference; `docs/architecture.md`: the
future-features section becomes the real backlog (config via
`initializationOptions`, incremental sync, unquote actions, extra-cells
quickfix, rename-file-on-convert).

## Definition of done

- Gates green; unit + e2e green (round-trip byte-identical).
- Manual in Helix: cursor in a column → `space+a` → add left/right/delete all
  behave; one `u` undoes a whole column deletion.

## Gotchas

- Skipped rows mean the edit list can be shorter than the row count — never
  index rows and edits in parallel; build edits row-by-row.
- Delete on the *last* remaining column must not produce overlapping or
  inverted spans (single-cell rows take the cell-content-only branch).
- Add-right on a row's **last** cell inserts after trailing padding — the new
  empty cell sits at the true row end, which is exactly where the pad quickfix
  would put it.
- Column indices shift after every applied edit; providers always compute from
  the *current* parse (stateless), so consecutive invocations are safe.
