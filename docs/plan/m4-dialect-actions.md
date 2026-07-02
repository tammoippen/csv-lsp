# M4 — Dialect actions: reinterpret + convert

**Goal:** two complementary dialect features. *Reinterpret* changes how the server
**parses** the open file (zero text changes — for files named `.csv` that are
actually semicolon-separated); *Convert* rewrites the **text** to a different
delimiter with quoting adapted. Combined they fix the classic mislabeled-export
case: reinterpret `data.csv` as SSV so diagnostics make sense, then convert to
CSV so the content finally matches the extension.

**Non-goals:** renaming the file via LSP resource operations (backlog — the
README documents renaming as the user's follow-up), auto-detection heuristics
beyond the existing open-time sniffing, converting files with quoting errors.

## Background you need

**Why reinterpretation must live server-side.** The dialect is resolved once at
`didOpen` (languageId → extension → sniff → CSV) and the extension wins for
`data.csv` — even when the content is semicolon-separated. Under the wrong
dialect every feature misbehaves (a `1;2;3` row is *one* CSV cell). No text edit
can fix that: the change needed is to the **server's state**, not the document.

**`workspace/executeCommand` in 3 sentences.** A `CodeAction` may carry a
`Command { title, command, arguments }` instead of (or besides) an edit. When
the user picks it, the client sends `workspace/executeCommand` with that name
and arguments, and the server does whatever the command means — here: flip the
document's dialect, reparse, republish diagnostics. Commands must be announced
in `capabilities.executeCommandProvider.commands`, and the server answers the
request with `null` (the *effects* travel as notifications, e.g. the fresh
`publishDiagnostics`).

Reinterpretation is **session-scoped**: on the next `didOpen` the extension
wins again. That is by design — the durable fixes are renaming the file or
converting its content.

**How conversion interacts with quoting.** Changing the delimiter changes which
cells need quotes:

- A **quoted** cell is already protected — its content span (quotes included) is
  emitted verbatim. `"a,b"` survives csv→tsv untouched.
- A clean **unquoted** cell needs quoting exactly when its value contains the
  **target** delimiter. It can never contain quotes or newlines — those rows
  carry parse errors, and files with parse errors don't offer conversion at all
  (their rows would pass through verbatim with old delimiters — fix quoting
  first). Ragged rows convert fine.

The canonical case is German data: `bolzen;1,50` (SSV, decimal comma) must
become `bolzen,"1,50"` in CSV. This is `QuotePolicy::PreserveOrRequired`, the
third render policy: preserve original quoting, add quotes only where the new
delimiter forces them.

**Why the dialect must flip after a conversion is applied.** The convert action
is a text edit; the client applies it and sends `didChange`. If the server then
reparses tabs under CSV rules, everything goes red. But the server must not
assume the edit was applied (the user can dismiss the menu). Solution: when
answering `codeAction`, record `(converted full text, target dialect)` per URI;
when a `didChange` arrives whose text **equals** a recorded conversion, adopt
that dialect for the reparse. Any other change clears the record. Unapplied
actions therefore change nothing, and a manual edit that happens to produce the
exact conversion *is* the conversion.

## Deliverables

`src/render.rs` (policy), `src/dialect.rs` (`name()`), `src/edits.rs`
(`apply`), `src/capabilities.rs` (executeCommand), `src/features/mod.rs`
(`Action.command`, `Action.dialect_change`, `ServerCommand`),
`src/features/reinterpret.rs`, `src/features/transform.rs`, `src/server.rs`
(executeCommand arm, pending-transform map), `src/document.rs`
(`Store::set_dialect`, `dialect_override`), e2e tests, README/architecture.

## TDD cycles

### 1. `feat(render): quote policy for dialect conversion`

- **Red** (`render.rs` unit tests): rendering `"a,b",x` from CSV with
  `dialect: Tsv` + `PreserveOrRequired` keeps `"a,b"` byte-identical and joins
  with tabs; an unquoted CSV cell containing a tab gets quoted when rendered as
  TSV; SSV `bolzen;1,50;10` rendered as CSV becomes `bolzen,"1,50",10`; cells
  needing nothing stay byte-identical. Plus `Dialect::name()` returns
  `"CSV"`/`"TSV"`/`"SSV"` (`dialect.rs` test).
- **Green**: third `QuotePolicy` variant; in `render`, `Quoted` → content
  verbatim, `Unquoted` → `encode_cell(value, opts.dialect, false)` iff the value
  contains the target delimiter, else verbatim.

### 2. `feat(edits): apply edits helper`

- **Red** (`edits.rs`): `apply(text, &edits)` splices non-overlapping,
  document-ordered edits (reverse-order application); cases: two edits, pure
  insertion, empty list, `apply(old, &minimize(old, new)) == new` round-trip.
- **Green**: promote the reverse-splice loop the tests already use privately;
  rewrite those tests on top of it.

### 3. `feat(capabilities): advertise the set-dialect command`

- **Red**: `server_capabilities` includes
  `execute_command_provider.commands == ["csv-lsp.setDialect"]`.
- **Green**: `ExecuteCommandOptions` in `capabilities.rs`; export the command
  name as a `pub const SET_DIALECT_COMMAND` (single source of truth).

### 4. `feat(features): reinterpret-dialect actions`

- **Red** (`features/reinterpret.rs`): a CSV document offers exactly
  `Reinterpret as TSV` and `Reinterpret as SSV` (kind `SOURCE`, empty `edits`,
  `command == Some(ServerCommand::SetDialect { dialect })`); an SSV document
  offers TSV and CSV; the current dialect is never offered.
- **Green**: `Action` gains `command: Option<ServerCommand>` (with
  `ServerCommand::SetDialect { dialect: Dialect }` in `features/mod.rs`;
  `None`/`vec![]` everywhere else — update existing constructors); the provider
  + registration line.

### 5. `feat(server): execute the set-dialect command`

- **Red** (e2e): open semicolon content under a `.csv` uri (`a;b;c\n1;2\n` —
  parses as one-column CSV, so no ragged diagnostics!); `codeAction` offers
  `Reinterpret as SSV` carrying a `Command`; send `workspace/executeCommand`
  with its name/arguments → response ok, then a fresh `publishDiagnostics`
  arrives with the *sane* SSV view (one `row-missing-cells`); follow-up
  `codeAction` offers `Reinterpret as CSV`. Unknown command name →
  `InvalidParams`.
- **Green**: `to_lsp_action` maps `ServerCommand` → `Command` with arguments
  `[uri_string, dialect_name_lowercase]`; `ExecuteCommand::METHOD` arm parses
  name + arguments, calls new `Store::set_dialect(&uri, dialect)` (mutate +
  reparse via `Document::update`-style rebuild), republishes diagnostics,
  answers `null`.

### 6. `feat(features): convert-dialect source actions`

- **Red** (`features/transform.rs`): a CSV doc offers `Convert to TSV` +
  `Convert to SSV` (kind `SOURCE`, non-empty edits, `dialect_change` set);
  German golden: SSV `artikel;preis\nbolzen;1,50\n` → CSV
  `artikel,preis\nbolzen,"1,50"\n` (apply via `edits::apply`); a file with a
  stray quote offers **no** conversions; a single-column file (conversion is a
  no-op) offers none.
- **Green**: `Action.dialect_change: Option<Dialect>`; provider = for each
  target ≠ current: `render(target, PreserveOrRequired)` + `minimize`, skip
  empty; suppressed entirely when `table.errors` is non-empty; registration.

### 7. `feat(server): flip dialect when a conversion is applied`

- **Red** (e2e): open `a,b,c\n1,2\n` (one short row); apply `Convert to TSV`;
  `didChange` with the converted text → diagnostics still show exactly one
  `row-missing-cells` (no explosion) and a follow-up `codeAction` offers
  `Convert to CSV` (proof the dialect flipped). Counter-test: after requesting
  actions, send a *different* manual change → dialect stays CSV. Combo e2e:
  reinterpret ssv-in-`.csv` (cycle 5 flow), then `Convert to CSV`, apply →
  comma-separated content, diagnostics stay sane.
- **Green**: `ServerState.pending_transforms: HashMap<String, Vec<(String,
  Dialect)>>`; `handle_code_action` records `(edits::apply(doc.text, edits),
  dialect)` for every action with `dialect_change`; the `didChange` arm takes
  the entry, matches the incoming text, passes `Some(dialect)` into
  `Store::change(..., dialect_override)`; cleared on every change/close.

### 8. `docs: dialect actions in readme and architecture`

README: feature bullets (reinterpret vs convert, when each applies), semantics
notes (reinterpretation is session-scoped, conversion is in-place — renaming
the file is your follow-up), diagnostics/actions reference updated.
`docs/architecture.md`: transform moves out of the future-features list;
executeCommand documented as the state-change channel.

## Definition of done

- Gates green; all new unit + e2e tests green (incl. the reinterpret→convert
  combo).
- Manual: semicolon `.csv` in Helix → `space+a` → reinterpret → diagnostics
  sane → convert → file is genuinely comma-separated.

## Gotchas

- `executeCommand` **arguments arrive as `Vec<serde_json::Value>`** — parse
  defensively, answer `InvalidParams` on shape mismatches (never panic).
- Record pending transforms per *offered* action, not per applied one — and
  match on full text equality, so dismissing the menu costs nothing.
- The reinterpret action must be built from `ctx.doc.dialect`, not from the
  extension — after one reinterpretation the *next* offers must reflect the new
  dialect.
- Don't offer `Convert to …` on files with parse errors: verbatim error rows
  would keep old delimiters and produce a mixed-dialect file.
