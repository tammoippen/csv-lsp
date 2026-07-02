# Development workflow

This project is developed **test-driven** in small red/green cycles with one commit per
cycle. The milestone plans in `docs/plan/` spell out every cycle; this document defines
the rules that apply to all of them.

## Toolchain

- Stable Rust ≥ 1.85 (edition 2024). No nightly features.
- `rustfmt` with default configuration (no `rustfmt.toml` — community default *is* the
  convention).
- `clippy` must be clean at `-D warnings` (default lint groups). Lint *configuration*
  lives in `Cargo.toml` under `[lints]`:
  - `unsafe_code = "forbid"` — this project has no reason to ever use `unsafe`.
  - `missing_docs = "warn"` — every public item carries a doc comment; CI turns
    warnings into errors.

## The loop (red → green → refactor → commit)

1. **Red** — write the smallest test that fails for the right reason. Run it, watch it
   fail. If it passes immediately, the test is wrong or the step is already done.
2. **Green** — write the *minimal* implementation that makes it pass. Resist
   generalizing beyond the test.
3. **Refactor** — with tests green, clean up naming/duplication. No behavior change.
4. **Gate** — all four must pass locally before every commit:

   ```sh
   cargo fmt --all
   cargo clippy --all-targets -- -D warnings
   cargo test
   RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
   ```

5. **Commit** — test + minimal implementation together in one commit (history stays
   bisectable; never commit a red state).

## Commit conventions

[Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <imperative summary ≤ 72 chars>

<optional body: the *why*, wrapped at 72>
```

- Types used here: `feat`, `fix`, `test`, `refactor`, `docs`, `chore`, `ci`.
- Scope = module (`parse`, `position`, `server`, `features`, …).
- One logical change per commit. A TDD cycle is one commit; a pure refactor is its own
  `refactor:` commit.
- Examples:
  - `feat(position): map byte offsets to utf-16 positions`
  - `test(parse): cover unclosed-quote recovery`
  - `refactor(server): extract notification dispatch`

## Testing conventions

- **Unit tests** live next to the code in `#[cfg(test)] mod tests`. Prefer asserting on
  *span slices* (`assert_eq!(span.slice(text), "…")`) over raw indices — failures read
  like text, not numbers.
- **Integration tests** (`tests/e2e.rs`) are black-box: they drive the real server
  through `lsp_server::Connection::memory()` using only the public API, exactly like an
  editor would. Always use `recv_timeout`, never blocking `recv` (a hang must fail the
  test, not the CI job).
- **Golden tests** compare full rendered output against raw string literals inline in
  the test — no snapshot framework, no extra dependency.
- **Corpus test**: the parser is *total*. A list of malformed snippets × all dialects
  is parsed and re-rendered; the only assertion is "no panic, spans in bounds".
- Test names describe behavior (`padding_is_trimmed_from_unquoted_cells`), not methods
  (`test_parse_2`).

## Error handling and panics

- The parser **never fails**: malformed input produces a `Table` plus `ParseError`s.
- Server request handlers return `Result`; the dispatch layer converts errors into LSP
  error responses. `catch_unwind` is the last-resort belt so one buggy feature cannot
  kill the server.
- `unwrap()`/`expect()` are acceptable in tests and in `main.rs` startup, nowhere else.
  In library code, prefer returning the error or handling it locally.

## Documentation

- Module-level `//!` comment stating the module's single responsibility.
- Doc comments on all public items (enforced by `missing_docs`); document *invariants*
  (e.g. "spans lie on char boundaries"), not restatements of the signature.

## Dependencies

Four direct dependencies (`lsp-server`, `lsp-types`, `serde_json`, `unicode-width`) —
see ADRs. Adding a dependency requires an ADR-worthy reason. `Cargo.lock` is
**committed** (this is a binary — reproducible builds trump lockfile churn).

## CI

`.github/workflows/ci.yml` runs the same four gates as the local loop (fmt check,
clippy `-D warnings`, tests, rustdoc `-D warnings`) on pushes and PRs, on stable Rust
with `Swatinem/rust-cache`. CI is the enforcement mechanism; the local loop is the
fast path.

## Logging

stdout belongs to the LSP protocol — **never print to it**. Diagnostics for humans go
to stderr via the `log!` macro, enabled by `CSV_LSP_LOG=1`. With Helix, run `hx -v`
and check `~/.cache/helix/helix.log`.
