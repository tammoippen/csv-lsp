# ADR 0003: Hand-written error-tolerant parser, not the `csv` crate

- Status: accepted
- Date: 2026-07-02

## Context

BurntSushi's `csv`/`csv-core` is the canonical Rust CSV reader. An LSP server,
however, is an *IDE backend*, and its parser has different requirements than a data
reader — the same reason rust-analyzer does not parse Rust with `syn`.

What the LSP needs, and what the `csv` crate offers:

| Need | `csv` crate |
|---|---|
| Per-cell byte spans (cell vs content vs quotes vs padding) for diagnostics and precise edits | per-record positions only; field spans not exposed |
| Error *reporting* with recovery (unclosed quote, stray quote, text after closing quote) | lenient by design: silently "repairs" malformed input — destroying exactly the information we must surface |
| Lossless layout model (alignment padding, original quoting, CRLF, BOM) so align⇄compact round-trips | normalizes on read; layout is discarded |
| Tolerate spaces after a closing quote (`"abc"  ,` — produced by our own align feature) | not modeled |
| Multi-line quoted cells mapped back to editor lines | spans missing |

## Decision

Write our own state-machine parser (~300–400 lines) with total error recovery,
producing the span-rich `Table`/`Row`/`Cell` model described in
`docs/architecture.md`. Use `csv-core` only as a behavioral reference for edge cases.

## Consequences

- The parser is the project's core asset and gets the deepest test coverage (span
  assertions, error-recovery cases, a never-panics corpus).
- We own RFC 4180 edge-case decisions explicitly (documented in the parser plan)
  instead of inheriting silent library behavior.
- No dependency on `csv`; grammar evolution (e.g. a future whitespace dialect) stays
  in our hands.
