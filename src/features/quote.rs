//! Quote a cell or a whole column: wrap unquoted cells in RFC 4180 quotes
//! via `encode_cell(force_quote: true)`.
//!
//! Only `Unquoted` cells on clean rows qualify — blank rows have nothing to
//! quote, parse-error rows are never rewritten, and already-quoted cells
//! would be no-ops the picker must not show. The edit replaces the
//! *content* span, so alignment padding stays outside the new quotes.

use std::collections::HashSet;

use lsp_types::CodeActionKind;

use crate::document::Document;
use crate::features::{Action, ActionContext, ActionProvider};
use crate::parse::{Quoting, Span, Table};
use crate::render::encode_cell;

/// Cursor-anchored quoting refactors.
pub struct QuoteCells;

impl ActionProvider for QuoteCells {
    fn name(&self) -> &'static str {
        "quote-cells"
    }

    fn actions(&self, ctx: &ActionContext) -> Vec<Action> {
        let table = &ctx.doc.table;
        let Some((row, column)) = ctx.cell_at_cursor() else {
            return Vec::new();
        };
        let error_rows: HashSet<usize> = table.errors.iter().map(|error| error.row).collect();
        let mut actions = Vec::new();

        if quotable(table, &error_rows, row, column) {
            actions.push(Action {
                title: "Quote cell".to_owned(),
                kind: CodeActionKind::REFACTOR_REWRITE,
                edits: vec![quote_edit(ctx.doc, row, column)],
                command: None,
                dialect_change: None,
                fixes: Vec::new(),
                is_preferred: false,
            });
        }

        // Quote column: every quotable cell of the cursor's column, header
        // included, in row order (keeps the edits non-overlapping).
        let edits: Vec<(Span, String)> = (0..table.rows.len())
            .filter(|&candidate| quotable(table, &error_rows, candidate, column))
            .map(|candidate| quote_edit(ctx.doc, candidate, column))
            .collect();
        if !edits.is_empty() {
            actions.push(Action {
                title: format!(
                    "Quote column {}",
                    column_title(&ctx.doc.text, table, column)
                ),
                kind: CodeActionKind::REFACTOR_REWRITE,
                edits,
                command: None,
                dialect_change: None,
                fixes: Vec::new(),
                is_preferred: false,
            });
        }
        actions
    }
}

/// `"header"` (truncated for the picker) or `#N` (1-based, as users count)
/// when the header lacks the column — shared with the column-edit actions.
pub fn column_title(text: &str, table: &Table, column: usize) -> String {
    const MAX_CHARS: usize = 24;
    let header_value = table
        .header()
        .and_then(|header| header.cells.get(column))
        .map(|cell| cell.value(text));
    match header_value {
        Some(value) if !value.is_empty() => {
            let mut name: String = value.chars().take(MAX_CHARS).collect();
            if value.chars().count() > MAX_CHARS {
                name.push('…');
            }
            format!("\"{name}\"")
        }
        _ => format!("#{}", column + 1),
    }
}

/// Whether the cell exists, sits on a clean non-blank row, and is unquoted.
fn quotable(table: &Table, error_rows: &HashSet<usize>, row: usize, column: usize) -> bool {
    let candidate = &table.rows[row];
    !candidate.is_blank()
        && !error_rows.contains(&row)
        && candidate
            .cells
            .get(column)
            .is_some_and(|cell| cell.quoting == Quoting::Unquoted)
}

/// Replace the cell's content span with its force-quoted encoding.
fn quote_edit(doc: &Document, row: usize, column: usize) -> (Span, String) {
    let cell = &doc.table.rows[row].cells[column];
    let value = cell.value(&doc.text);
    (cell.content_span, encode_cell(&value, doc.dialect, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edits::apply;
    use crate::features::testutil::{ctx_at, doc};

    fn quote_cell_at(text: &str, offset: usize) -> Option<String> {
        let doc = doc(text);
        let actions = QuoteCells.actions(&ctx_at(&doc, offset));
        let action = actions.iter().find(|action| action.title == "Quote cell")?;
        assert_eq!(action.kind, CodeActionKind::REFACTOR_REWRITE);
        Some(apply(&doc.text, &action.edits))
    }

    #[test]
    fn wraps_the_cell_under_the_cursor() {
        let text = "a,b\n";
        assert_eq!(
            quote_cell_at(text, text.find('b').unwrap()).as_deref(),
            Some("a,\"b\"\n")
        );
    }

    #[test]
    fn padding_stays_outside_the_quotes() {
        let text = "x, a \ny,b\n";
        assert_eq!(
            quote_cell_at(text, text.find(" a ").unwrap() + 1).as_deref(),
            Some("x, \"a\" \ny,b\n")
        );
    }

    #[test]
    fn empty_cells_become_empty_quoted_cells() {
        let text = "a,,c\n";
        assert_eq!(quote_cell_at(text, 2).as_deref(), Some("a,\"\",c\n"));
    }

    #[test]
    fn quoted_cells_offer_nothing() {
        let text = "a,\"b\"\n";
        assert_eq!(quote_cell_at(text, text.find("\"b").unwrap() + 1), None);
    }

    #[test]
    fn blank_and_error_rows_offer_nothing() {
        assert_eq!(quote_cell_at("a,b\n\n1,2\n", "a,b\n".len()), None); // blank row
        let text = "a,b\n5\" bolt,x\n";
        assert_eq!(quote_cell_at(text, text.find("bolt").unwrap()), None);
    }

    fn quote_column_at(text: &str, offset: usize) -> Option<(String, String)> {
        let doc = doc(text);
        let actions = QuoteCells.actions(&ctx_at(&doc, offset));
        let action = actions
            .iter()
            .find(|action| action.title.starts_with("Quote column"))?;
        assert_eq!(action.kind, CodeActionKind::REFACTOR_REWRITE);
        Some((action.title.clone(), apply(&doc.text, &action.edits)))
    }

    #[test]
    fn quote_column_wraps_only_the_unquoted_cells() {
        let text = "id,name\n1,\"x\"\n2,y\n";
        let (title, quoted) = quote_column_at(text, text.find("name").unwrap()).unwrap();
        assert_eq!(title, "Quote column \"name\"");
        assert_eq!(quoted, "id,\"name\"\n1,\"x\"\n2,\"y\"\n");
    }

    #[test]
    fn quote_column_skips_blank_and_error_rows() {
        let text = "a,b\n\n1,\"x\" y\n2,z\n";
        let (_, quoted) = quote_column_at(text, text.find('b').unwrap()).unwrap();
        // Column 1 only: header quoted, blank row untouched, the
        // text-after-quote row untouched, `z` quoted.
        assert_eq!(quoted, "a,\"b\"\n\n1,\"x\" y\n2,\"z\"\n");
    }

    #[test]
    fn quote_column_falls_back_to_a_numbered_title() {
        // Row 1 is longer than the header: column 3 has no header name.
        let text = "a,b\n1,2,3\n";
        let (title, _) = quote_column_at(text, text.find('3').unwrap()).unwrap();
        assert_eq!(title, "Quote column #3");
    }

    #[test]
    fn fully_quoted_columns_offer_no_column_action() {
        let text = "\"a\"\n\"b\"\n";
        assert_eq!(quote_column_at(text, 1), None);
    }
}
