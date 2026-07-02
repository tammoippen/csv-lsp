//! csv-lsp binary: speaks the Language Server Protocol over stdio.
//!
//! All logic lives in the library (see `docs/architecture.md`); this shim only
//! owns the process boundary. stdout carries protocol frames exclusively —
//! human-facing output goes to stderr.

fn main() {
    // Wired to the server main loop later in M0 (cycle 9).
}
