//! Surfaces the parser's recovered quoting errors as diagnostics.

use crate::features::{Diag, DiagnosticRule, Severity};
use crate::parse::{ParseErrorKind, Table};

/// Maps [`Table::errors`] one-to-one to diagnostics.
pub struct ParseErrors;

impl DiagnosticRule for ParseErrors {
    fn name(&self) -> &'static str {
        "parse-errors"
    }

    fn check(&self, _text: &str, table: &Table) -> Vec<Diag> {
        table
            .errors
            .iter()
            .map(|error| {
                let (code, severity, message) = match error.kind {
                    ParseErrorKind::UnclosedQuote => (
                        "unclosed-quote",
                        Severity::Error,
                        "quoted cell is never closed, the rest of the file became this cell",
                    ),
                    // Extremely common in real data (`5" bolt`) — warn only.
                    ParseErrorKind::StrayQuote => (
                        "stray-quote",
                        Severity::Warning,
                        "unexpected quote inside an unquoted cell, quote the whole cell or double the quote",
                    ),
                    ParseErrorKind::TextAfterClosingQuote => (
                        "text-after-quote",
                        Severity::Error,
                        "unexpected text after the closing quote",
                    ),
                };
                Diag {
                    span: error.span,
                    severity,
                    code,
                    message: message.to_owned(),
                    data: None,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::Dialect;
    use crate::parse::parse;

    #[test]
    fn quoting_errors_become_diagnostics_with_spans_and_codes() {
        let text = "5\" bolt,\"x\" y\n\"open";
        let table = parse(text, Dialect::Csv);
        let diags = ParseErrors.check(text, &table);

        assert_eq!(diags.len(), 3);

        assert_eq!(diags[0].code, "stray-quote");
        assert_eq!(diags[0].severity, Severity::Warning);
        assert_eq!(diags[0].span.slice(text), "\"");
        assert_eq!(diags[0].span.start, 1);

        assert_eq!(diags[1].code, "text-after-quote");
        assert_eq!(diags[1].severity, Severity::Error);
        assert_eq!(diags[1].span.slice(text), "y");

        assert_eq!(diags[2].code, "unclosed-quote");
        assert_eq!(diags[2].severity, Severity::Error);
        assert_eq!(diags[2].span.start, text.find("\"open").unwrap());
    }

    #[test]
    fn the_standard_registry_runs_this_rule() {
        let text = "a\"b\n";
        let table = parse(text, Dialect::Csv);
        let diags = crate::features::Registry::standard().diagnostics(text, &table);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "stray-quote");
    }
}
