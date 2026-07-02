# ADR 0001: Implement in Rust rather than Zig

- Status: accepted
- Date: 2026-07-02

## Context

Both Rust and Zig produce small, fast, single-binary language servers and were
candidates for this project. The server's value lies in CSV-specific features, not in
protocol plumbing, so the deciding factors are ecosystem maturity and long-term
maintenance cost.

## Decision

Rust, stable toolchain, edition 2024.

## Rationale

| Aspect | Rust | Zig |
|---|---|---|
| LSP libraries | `lsp-server` + `lsp-types`: maintained under rust-lang, power rust-analyzer | `zigtools/lsp-kit` generates types from the LSP meta-model but tracks a Zig 0.16-dev nightly; the JSON-RPC loop is hand-rolled |
| Toolchain | Stable; editions guarantee code keeps compiling | Pre-1.0; std breaks between 0.x releases — a recurring maintenance tax |
| JSON | `serde_json`: LSP types (de)serialize for free | `std.json` works, but the huge LSP type surface is manual or generated |
| Tooling | cargo test / clippy / rustfmt out of the box | good test runner, thinner ecosystem |
| Distribution | single static binary | single static binary (Zig's genuine strength — parity, not advantage) |
| Ecosystem fit | Helix itself is Rust; contributor overlap | — |

Zig's advantages (compile speed, language simplicity, cross-compilation) do not
outweigh building a long-lived tool on a pre-1.0 toolchain while also hand-rolling
protocol plumbing.

## Consequences

- We accept Rust compile times and the borrow checker's learning curve.
- We get battle-tested protocol crates, first-class testing, and a contributor pool
  aligned with the primary target editor.
