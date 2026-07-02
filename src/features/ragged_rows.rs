//! Checks every row's cell count against the header's column contract.

use std::collections::HashSet;

use serde_json::json;

use crate::features::{Diag, DiagnosticRule, Severity};
use crate::parse::{Span, Table};

/// Rows with fewer or more cells than the header (= first non-blank row).
///
/// Blank rows are separators and rows overlapping a parse error are already
/// broken — both are skipped so one unclosed quote does not cascade into a
/// wall of ragged-row errors.
pub struct RaggedRows;

impl DiagnosticRule for RaggedRows {
    fn name(&self) -> &'static str {
        "ragged-rows"
    }

    fn check(&self, _text: &str, table: &Table) -> Vec<Diag> {
        let (Some(expected), Some(header_index)) = (table.expected_columns(), table.header_index())
        else {
            return Vec::new();
        };
        let error_rows: HashSet<usize> = table.errors.iter().map(|error| error.row).collect();

        let mut diags = Vec::new();
        for (index, row) in table.rows.iter().enumerate() {
            if index <= header_index || row.is_blank() || error_rows.contains(&index) {
                continue;
            }
            let count = row.cells.len();
            let noun = if count == 1 { "cell" } else { "cells" };
            if count < expected {
                diags.push(Diag {
                    // Zero-width at the row end: "cells are missing here" —
                    // renders as an end-of-line marker in editors.
                    span: Span::new(row.span.end, row.span.end),
                    severity: Severity::Error,
                    code: "row-missing-cells",
                    message: format!("row has {count} {noun}, expected {expected}"),
                    data: Some(json!({ "row": index, "missing": expected - count })),
                });
            } else if count > expected {
                diags.push(Diag {
                    span: Span::new(row.cells[expected].span.start, row.span.end),
                    severity: Severity::Error,
                    code: "row-extra-cells",
                    message: format!("row has {count} {noun}, expected {expected}"),
                    data: Some(json!({ "row": index, "extra": count - expected })),
                });
            }
        }
        diags
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::Dialect;
    use crate::parse::parse;

    fn check(text: &str) -> Vec<Diag> {
        RaggedRows.check(text, &parse(text, Dialect::Csv))
    }

    #[test]
    fn short_rows_get_a_zero_width_diagnostic_at_their_end() {
        let text = "a,b,c\n1,2\n";
        let diags = check(text);

        assert_eq!(diags.len(), 1);
        let diag = &diags[0];
        assert_eq!(diag.code, "row-missing-cells");
        assert_eq!(diag.severity, Severity::Error);
        assert!(diag.span.is_empty());
        assert_eq!(diag.span.start, text.len() - 1); // end of row 1, before \n
        assert_eq!(diag.message, "row has 2 cells, expected 3");
        assert_eq!(diag.data, Some(json!({ "row": 1, "missing": 1 })));
    }

    #[test]
    fn singular_cell_count_reads_naturally() {
        let diags = check("a,b\n1\n");
        assert_eq!(diags[0].message, "row has 1 cell, expected 2");
    }

    #[test]
    fn long_rows_span_their_extra_cells() {
        let text = "a,b\n1,2,3,4\n";
        let diags = check(text);

        assert_eq!(diags.len(), 1);
        let diag = &diags[0];
        assert_eq!(diag.code, "row-extra-cells");
        assert_eq!(diag.span.slice(text), "3,4");
        assert_eq!(diag.message, "row has 4 cells, expected 2");
        assert_eq!(diag.data, Some(json!({ "row": 1, "extra": 2 })));
    }

    #[test]
    fn blank_rows_are_not_ragged() {
        assert_eq!(check("a,b\n\n1,2\n"), Vec::new());
    }

    #[test]
    fn rows_with_parse_errors_are_skipped() {
        // Row 1 has an unclosed quote — reporting it as ragged would be
        // noise on top of the quoting error.
        assert_eq!(check("a,b\n\"x\n"), Vec::new());
    }

    #[test]
    fn headerless_and_header_only_files_have_no_contract() {
        assert_eq!(check(""), Vec::new());
        assert_eq!(check("\n\n"), Vec::new());
        assert_eq!(check("a,b,c\n"), Vec::new());
    }

    #[test]
    fn leading_blank_lines_do_not_shift_the_contract() {
        // Header is the first non-blank row.
        let diags = check("\na,b,c\n1,2\n");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "row-missing-cells");
    }
}
