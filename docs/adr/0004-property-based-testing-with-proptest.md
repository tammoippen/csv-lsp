# ADR 0004: Property-based testing with `proptest`

- Status: accepted
- Date: 2026-07-03

## Context

The server's robustness story rests on documented *invariants* ("`parse` is
total", "all spans lie on `char` boundaries", "align is idempotent", "action
edits are non-overlapping and in document order"). So far these are checked
against hand-picked examples and a fixed adversarial corpus. Hand-picked
inputs only cover the failure modes we already thought of; the interesting
bugs live in the inputs we did not.

Python's Hypothesis solved this with *property-based testing*: generate
thousands of inputs from composable strategies, assert the invariant, and
shrink any counterexample to a minimal reproducer. The Rust ecosystem offers
several equivalents:

| Tool | Model | Fit |
|---|---|---|
| `proptest` | Hypothesis-style: strategy combinators, integrated shrinking, failure persistence files | runs in `cargo test` on stable; strategies compose (weighted CSV-shaped fragments, not just uniform noise) |
| `quickcheck` | Haskell QuickCheck port: type-directed `Arbitrary` | simpler, but shrinking is per-type, not per-strategy — poor for "string built from CSV fragments" |
| `bolero` | one property, many engines (RNG, libFuzzer, AFL, Kani) | attractive later; extra harness indirection not justified yet |
| `cargo-fuzz` / `afl.rs` + `arbitrary` | coverage-guided fuzzing | strongest input explorer, but nightly toolchain, indefinite runtime — a campaign tool, not a CI gate |

An LSP server has two untrusted input surfaces, and both need this:

1. **Document text** — anything a user opens (the parser and everything
   downstream of it must be total).
2. **Protocol traffic** — anything a client sends: reversed ranges,
   positions past EOF or inside multi-byte characters, unknown URIs,
   malformed params, junk `executeCommand` arguments.

## Decision

Adopt `proptest` as a **dev-dependency** (the shipped binary is unaffected)
— it is the closest Rust equivalent of Hypothesis and its failure
persistence gives us a growing regression corpus for free.

Two integration-test suites exercise the public API exactly as the server
uses it:

- `tests/properties.rs` — library invariants over generated documents
  (CSV-shaped fragment strategies × all dialects): parser totality and span
  soundness, render round-trips (align/compact idempotence, value
  preservation), dialect-conversion completeness on clean tables,
  `minimize`/`apply` correctness, position-encoding round-trips, and the
  action/diagnostic edit contract.
- `tests/protocol.rs` — a hostile client driving the real server over
  `Connection::memory()` with generated request/notification sequences;
  every request must be answered without tripping the `catch_unwind`
  backstop, and shutdown must stay clean.

Coverage-guided fuzzing (`cargo-fuzz` on the `parse` entry point) remains
the documented escalation path if the property suites stop finding bugs;
`bolero` is the migration route if we ever want both under one harness.

## Consequences

- `proptest` and its transitive dev-dependencies join `Cargo.lock`; the
  runtime dependency set is unchanged (still four crates).
- Property tests run under plain `cargo test`, so CI enforces them with no
  workflow changes. Case counts are tuned so the suite stays in seconds;
  deeper local runs via `PROPTEST_CASES=10000 cargo test`.
- When proptest finds a counterexample it writes a seed file next to the
  suite (`tests/*.proptest-regressions`): **commit it** — replaying past
  failures first is the Hypothesis example-database workflow.
- Properties double as executable documentation: the invariants in
  `docs/architecture.md` now have machine-checked counterparts.
