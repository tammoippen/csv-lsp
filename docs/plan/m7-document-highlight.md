# M7 — Column selection via documentHighlight

**Goal:** "select the column under the cursor" in Helix. After this milestone,
cursor in a column + `Space+h` = one selection per cell of that column, ready
for multi-cursor editing (`c`, `A`, pipes, …).

**Non-goals:** semantic tokens (rainbow columns), `selectionRange` (can only
expand around one cursor — a column is disjoint per-row ranges, and Helix uses
tree-sitter for expand-selection anyway).

## Background you need

**Selections are editor state.** No LSP request lets a server set or move the
user's selection — the protocol deliberately keeps that client-side.

**The bridge.** `textDocument/documentHighlight` is a read-only request:
"given this cursor position, which ranges belong together?" (classically: all
occurrences of the symbol under the cursor). Helix's
`select_references_to_symbol_under_cursor` — bound to `Space+h` — takes that
response and **converts the returned ranges into multi-selections**. So a
server that answers "the content spans of the cursor's column" gives Helix
column selection without any protocol extension.

**Which ranges.** Content spans, not full cell spans: multi-cursor edits then
operate on values while alignment padding survives. An empty cell has a
zero-width content span → Helix places a bare cursor there, which is exactly
what you want for typing into empty cells. Row participation mirrors the
column-edit rule (`editable_rows`): clean rows that have the column, header
included; blank rows, parse-error rows and shorter rows contribute nothing.

## TDD cycles

### 1. `feat(capabilities): advertise document highlight`

- **Red**: capability test expects
  `document_highlight_provider == Some(OneOf::Left(true))`.
- **Green**: one line in `server_capabilities`.

### 2. `feat(server): highlight the column under the cursor`

- **Red** (`features/columns.rs` unit): `column_content_spans(table, column)`
  over `"id,name\n1,x\n\n2,\n5\" b,y\nz\n"` for column 1 → spans slicing to
  `name`, `x`, and a zero-width span in the `2,` row; the blank row, the
  stray-quote row and the short `z` row contribute nothing.
- **Red** (e2e): `textDocument/documentHighlight` at a `name`-column position
  returns exactly those ranges with `kind: Text`; a position past EOF returns
  no highlights.
- **Green**: `column_content_spans` reusing `editable_rows`; a
  `DocumentHighlightRequest` arm in `server.rs` (offset → `cell_at` → column →
  spans → ranges).

### 3. `docs: column selection via space+h in readme`

README feature bullet + a "column selection" note in the Helix section
(`Space+h` on any cell), `docs/architecture.md` capability list updated.

## Definition of done

- Gates green; unit + e2e green.
- Manual in Helix: cursor in a column, `Space+h`, type `c` — every cell of the
  column changes simultaneously; padding and other columns untouched.

## Gotchas

- Return `None`/empty instead of erroring when the cursor is on no cell (past
  EOF, unknown document) — clients call this speculatively.
- Zero-width ranges are legal LSP and intentionally kept (empty cells).
- Ranges must be in document order — Helix keeps the primary selection
  sensible that way.
