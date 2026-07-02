# ADR 0002: Use `lsp-server` + `lsp-types`, not an async framework, not hand-rolled

- Status: accepted
- Date: 2026-07-02

## Context

Options for the protocol layer:

1. Hand-roll everything (JSON-RPC framing + all LSP types).
2. Async frameworks: `tower-lsp` (unmaintained ~3 years), its community fork
   `tower-lsp-server`, or `async-lsp` — all tokio-based service abstractions.
3. `lsp-server` + `lsp-types`: the synchronous scaffold rust-analyzer is built on —
   stdio/in-memory transports, `Content-Length` framing, initialize/shutdown helpers,
   and a plain channel of messages you consume in a `for` loop.

## Decision

Option 3: `lsp-server` (0.8) + `lsp-types` (0.97, minor pinned) + `serde_json`.

## Rationale

- Transport is trivial (~200 lines if hand-rolled) but the **LSP type surface is
  enormous** — hundreds of carefully versioned structs. `lsp-types` provides all of it
  spec-correct and serde-ready; reimplementing invites subtle bugs (position encoding
  semantics, capability shapes) and buys nothing.
- CSV parsing is O(n) and takes microseconds–milliseconds; every request can be
  answered synchronously. An async runtime plus service layers is pure overhead here.
- rust-analyzer proves the sync architecture at scales far beyond ours.
- `lsp-types` 0.97 switched `Url` → its own `Uri` type; we pin the minor version and
  treat URIs as opaque keys to stay insulated from churn.

## Consequences

- Library for types + transport; **direct code** for dispatch, state, and features.
- No tokio in the dependency tree; the whole server is a single thread plus
  lsp-server's I/O threads.
- Long-running requests would block the loop — acceptable by design; nothing we do is
  long-running.
