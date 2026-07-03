//! csv-lsp binary: speaks the Language Server Protocol over stdio.
//!
//! All logic lives in the library (see `docs/contributing.html#layout`); this
//! shim only owns the process boundary. stdout carries protocol frames
//! exclusively — human-facing output goes to stderr.

use std::process::ExitCode;

fn main() -> ExitCode {
    let (connection, io_threads) = lsp_server::Connection::stdio();
    let served = csv_lsp::server::run(connection);
    let joined = io_threads.join().map_err(csv_lsp::server::BoxError::from);
    match served.and(joined) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("[csv-lsp] fatal: {err}");
            ExitCode::FAILURE
        }
    }
}
