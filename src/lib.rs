//! Library backing the csv-lsp language server.
//!
//! Everything below `main.rs` lives here so unit tests and the end-to-end
//! protocol tests in `tests/` can exercise it. See `docs/architecture.md` for
//! the module map and `docs/plan/` for the milestone plans.

pub mod capabilities;
pub mod dialect;
pub mod document;
pub mod features;
pub mod parse;
pub mod position;
pub mod render;
pub mod server;

/// Log a line to **stderr** when `CSV_LSP_LOG=1` is set.
///
/// stdout belongs to the LSP protocol and must never carry human output.
/// Helix surfaces stderr in its own log when started with `hx -v`.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        if std::env::var_os("CSV_LSP_LOG").is_some() {
            eprintln!("[csv-lsp] {}", format!($($arg)*));
        }
    };
}
