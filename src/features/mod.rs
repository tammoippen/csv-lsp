//! The feature framework: diagnostic rules (and, from M2 on, code-action
//! providers) collected in a [`Registry`].
//!
//! **Adding a feature = one new module here + one line in
//! [`Registry::standard`].** Everything works on byte spans over the parsed
//! [`Table`]; the server converts to LSP types at its boundary only.

pub mod parse_errors;
pub mod ragged_rows;

use crate::parse::{Span, Table};

/// A diagnostic in crate-internal form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    /// The offending bytes (zero-width spans are legal and render as a
    /// caret / end-of-line marker in editors).
    pub span: Span,
    /// How loud the editor should be about it.
    pub severity: Severity,
    /// Stable machine-readable code (e.g. `row-missing-cells`), also shown
    /// to users next to the message.
    pub code: &'static str,
    /// Human-readable: states the fact and the expectation, lowercase, no
    /// trailing period.
    pub message: String,
    /// Optional structured payload, echoed back by clients in code-action
    /// requests. Never *relied* upon — providers recompute from the table.
    pub data: Option<serde_json::Value>,
}

/// Diagnostic severity, mapped to `lsp_types::DiagnosticSeverity` at the
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The file's structure is broken.
    Error,
    /// Suspicious but parseable (e.g. a stray quote).
    Warning,
    /// Informational.
    Info,
    /// Editor hint.
    Hint,
}

/// A check over the parsed table producing diagnostics.
pub trait DiagnosticRule {
    /// Stable rule name (for logs).
    fn name(&self) -> &'static str;
    /// Run the rule. Must not panic on any table the parser can produce.
    fn check(&self, text: &str, table: &Table) -> Vec<Diag>;
}

/// All registered features.
pub struct Registry {
    rules: Vec<Box<dyn DiagnosticRule>>,
}

impl Registry {
    /// The standard feature set — the single registration point.
    pub fn standard() -> Self {
        Registry {
            rules: vec![
                Box::new(parse_errors::ParseErrors),
                Box::new(ragged_rows::RaggedRows),
            ],
        }
    }

    /// All diagnostics for a parsed document, in rule order.
    pub fn diagnostics(&self, text: &str, table: &Table) -> Vec<Diag> {
        self.rules
            .iter()
            .flat_map(|rule| rule.check(text, table))
            .collect()
    }
}
