# M2 — Quickfix: pad short rows with empty cells

**Goal:** the first *actionable* feature. On a row with missing cells the user gets a
quickfix ("pad row with N empty cells"); a file-wide fix-all pads every short row at
once. This milestone also builds the code-action plumbing every later feature reuses
(`ActionContext`, kind filtering, the request handler).

**Non-goals:** no fix for `row-extra-cells` (deleting data needs more thought than
adding empty cells — backlog), no rendering.

## Background you need (LSP code actions in 10 minutes)

Flow: the user triggers "code actions" at a cursor/selection (Helix: `space+a`) →
client sends `textDocument/codeAction`:

```jsonc
{ "textDocument": {"uri": …},
  "range": {…},                  // cursor = zero-width range
  "context": {
    "diagnostics": [ … ],        // the client's CURRENT diagnostics overlapping range
    "only": ["quickfix"]         // optional kind filter
  } }
```

The server answers with a list of `CodeAction`s:

```jsonc
{ "title": "Pad row with 2 empty cells",   // shown verbatim in the picker
  "kind": "quickfix",
  "diagnostics": [ … ],                    // which squiggles this action fixes
  "isPreferred": true,                     // hint: the default choice
  "edit": { "changes": { "<uri>": [ {"range": …, "newText": ","} ] } } }
```

The user picks one; the **client applies the edit locally** (then sends `didChange`).
No server round-trip — which is why we advertise `resolveProvider: false` and compute
edits eagerly (they are cheap for CSV), and why we do **not** use `executeCommand`.

Rules that matter:

- **Kinds are hierarchical dotted strings** and `only` filters by *prefix segment*:
  `"source"` matches `"source.fixAll"`; `"quickfix"` does not match `"quickfixes"`.
  Filtering happens once, centrally (Registry), not in each provider.
- **`context.diagnostics` is the client's view — linkage only, never input.**
  Providers recompute everything from the freshly parsed `Table`; we only *emit*
  matching diagnostics in the response so the editor can connect action ↔ squiggle.
  (Relying on `data` echoed back breaks with clients that drop or reorder it.)
- **TextEdit semantics**: ranges use the negotiated position encoding, all edits of
  one action apply atomically to the *current* document state, and they must not
  overlap. An insertion is a zero-width range.
- **Cursor conventions** (from M1's `cell_at`): offset on a delimiter resolves to the
  cell left of it; offset at end-of-line belongs to that row (inclusive row end) —
  so a cursor sitting at the EOL of a short row still gets the quickfix.

## The fix, concretely

Header has 4 columns; row 2 is `x,y` (2 cells, 2 missing). The edit inserts the
delimiter twice at the row's end (zero-width span at `row.span.end`):

```
before:  x,y
after:   x,y,,        → 4 cells: x, y, "", ""
```

Parser guarantees make this safe: `row.span.end` excludes the terminator, and
trailing padding is part of the last cell's span, so the insertion lands after any
alignment padding (re-aligning later is M3's job).

## TDD cycles

### 1. `feat(features): action context with cursor-to-cell resolution`

- **Red** (unit): `ActionContext { doc, range, client_diagnostics, only }` with
  `cell_at_cursor()` / `column_at_cursor()` (delegating to `Table::cell_at` at
  `range.start`): mid-cell → `(row, col)`; on a delimiter → left cell; at EOL of a
  row → its last cell; inside a multi-line quoted cell → that cell; past EOF →
  `None`.
- **Green**: the struct + helpers in `features/mod.rs`; extend `Registry` with
  `providers: Vec<Box<dyn ActionProvider>>` and `actions(&ctx)` (no filtering yet),
  plus the `Action` struct (`title`, `kind`, `edits: Vec<(Span, String)>`, `fixes`,
  `is_preferred`).

### 2. `feat(features): pad-row quickfix inserts missing empty cells`

- **Red**: doc `"a,b,c\n1,2\nx\n"`; cursor in row 1 → exactly one action: title
  `Pad row with 1 empty cell`, kind `quickfix`, `is_preferred`, edits
  `[(Span{end,end} of row 1, ",")]`; cursor in row 2 → `Pad row with 2 empty cells`,
  edit text `",,"`; cursor in the (complete) header row → no per-row action; TSV doc
  → inserts `"\t"` (delimiter-aware); singular/plural in the title.
- **Green**: `pad_rows.rs` provider (per-row part): rows intersecting `ctx.range`
  (inclusive-touch), skipping blank/error rows, using `expected_columns()`; `fixes`
  reuses the `ragged_rows` rule's `Diag` for that row (call the rule, filter by row —
  do not duplicate message formatting). Register in `Registry::standard()`.

### 3. `feat(features): fix-all action pads every short row`

- **Red**: same doc; cursor anywhere (even in the header) → an action
  `Pad all short rows (2)` of kind `source.fixAll` with two insert edits (row 1 and
  row 2), `is_preferred == false`; absent when the file has no short rows.
- **Green**: extend `pad_rows.rs`; edits collected in row order (non-overlapping by
  construction).

### 4. `feat(features): honor the client's code-action kind filter`

- **Red**: with `only = ["quickfix"]` the fix-all disappears; with
  `only = ["source"]` only fix-all remains (prefix match!); with
  `only = ["source.fixAll"]` idem; `None` → everything.
- **Green**: `kind_matches(only, kind)` with dotted-prefix semantics, applied once in
  `Registry::actions`.

### 5. `feat(server): serve pad-row code actions with workspace edits`

- **Red** (e2e): open `"a,b,c\n1,2\n"`; `codeAction` request with a zero-width range
  at line 1 → two actions (quickfix first); the quickfix's
  `edit.changes[uri]` contains one `TextEdit`; **apply it client-side** (tests get an
  `apply_edits(text, &[TextEdit], enc)` helper using the same `LineIndex` math) →
  `didChange` with the result → next `publishDiagnostics` has no
  `row-missing-cells`; separate test: `only=["source.fixAll"]` → apply fix-all →
  zero ragged diagnostics on a file with several short rows.
- **Green**: replace the M0 stub: deserialize `CodeActionParams`, convert range →
  byte span, build `ActionContext` (pass `context.diagnostics` + `only`), run
  registry, convert `Action → CodeAction` (edits → `TextEdit`s via
  `position::range`, `fixes → diagnostics`, `WorkspaceEdit { changes: {uri: …} }`).

## Definition of done

- Gates green; cycles 1–5 tests green.
- Manual in Helix: cursor on a short row → `space+a` shows both actions; picking the
  quickfix pads the row and the squiggle disappears; fix-all repairs the whole file;
  `space+a` on a healthy row offers only fix-all (or nothing when the file is clean).

## Gotchas

- Helix sends the *cursor* as a zero-width range — intersection tests must treat
  touching (`range.start == row.span.end`) as intersecting, or EOL cursors lose the
  fix.
- Never return an action with an empty `edits` list — a no-op entry in the picker is
  worse than no entry.
- `WorkspaceEdit`: use the plain `changes` map, not `documentChanges` — broadest
  client compatibility and we don't need versioned edits (the action was computed
  for the version the client holds).
- Multiple actions: quickfix (preferred) sorts before fix-all in our response; do not
  rely on client-side ordering.
