//! Structural column editing at the cursor: insert an empty column left or
//! right of the current one, or delete it across the whole file.
//!
//! Edits cover clean rows that *have* the column — blank rows, parse-error
//! rows and shorter rows are skipped (the latter stay ragged and keep their
//! pad quickfix). Because the header row is edited too, the column contract
//! shifts coherently: clean files stay clean.

use std::collections::HashSet;

use lsp_types::CodeActionKind;

use crate::features::quote::column_title;
use crate::features::{Action, ActionContext, ActionProvider};
use crate::parse::{Row, Span, Table};

/// `Add column left/right of …` and `Delete column …` at the cursor.
pub struct ColumnEdits;

impl ActionProvider for ColumnEdits {
    fn name(&self) -> &'static str {
        "column-edits"
    }

    fn actions(&self, ctx: &ActionContext) -> Vec<Action> {
        let table = &ctx.doc.table;
        let Some(column) = ctx.column_at_cursor() else {
            return Vec::new();
        };
        let delimiter = (table.dialect.delimiter() as char).to_string();
        let title = column_title(&ctx.doc.text, table, column);
        let error_rows: HashSet<usize> = table.errors.iter().map(|error| error.row).collect();

        // One delimiter at a cell boundary = one new empty cell: before any
        // leading padding (left) or after any trailing padding (right).
        let add_left: Vec<(Span, String)> = editable_rows(table, &error_rows, column)
            .map(|row| {
                let at = row.cells[column].span.start;
                (Span::new(at, at), delimiter.clone())
            })
            .collect();
        if add_left.is_empty() {
            // No clean row actually has this column.
            return Vec::new();
        }
        let add_right: Vec<(Span, String)> = editable_rows(table, &error_rows, column)
            .map(|row| {
                let at = row.cells[column].span.end;
                (Span::new(at, at), delimiter.clone())
            })
            .collect();
        let delete: Vec<(Span, String)> = editable_rows(table, &error_rows, column)
            .map(|row| (delete_span(row, column), String::new()))
            .collect();

        let action = |title: String, edits: Vec<(Span, String)>| Action {
            title,
            kind: CodeActionKind::REFACTOR,
            edits,
            command: None,
            dialect_change: None,
            fixes: Vec::new(),
            is_preferred: false,
        };
        vec![
            action(format!("Add column left of {title}"), add_left),
            action(format!("Add column right of {title}"), add_right),
            action(format!("Delete column {title}"), delete),
        ]
    }
}

/// The bytes deleting the column removes from a row: the cell plus one
/// adjacent delimiter — the preceding one for inner columns, the following
/// one for the first; single-cell rows just lose their content (the row
/// becomes a blank line).
fn delete_span(row: &Row, column: usize) -> Span {
    if column > 0 {
        Span::new(row.cells[column - 1].span.end, row.cells[column].span.end)
    } else if row.cells.len() > 1 {
        Span::new(row.cells[0].span.start, row.cells[1].span.start)
    } else {
        row.cells[0].span
    }
}

/// The content spans of a column across clean rows (header included), in
/// document order — the payload behind `textDocument/documentHighlight`,
/// which Helix turns into a per-cell multi-selection (`Space+h`), giving
/// column selection.
pub fn column_content_spans(table: &Table, column: usize) -> Vec<Span> {
    let error_rows: HashSet<usize> = table.errors.iter().map(|error| error.row).collect();
    editable_rows(table, &error_rows, column)
        .map(|row| row.cells[column].content_span)
        .collect()
}

/// Clean rows that have the column, in document order.
fn editable_rows<'t>(
    table: &'t Table,
    error_rows: &'t HashSet<usize>,
    column: usize,
) -> impl Iterator<Item = &'t Row> {
    table
        .rows
        .iter()
        .enumerate()
        .filter_map(move |(index, row)| {
            (!row.is_blank() && !error_rows.contains(&index) && row.cells.len() > column)
                .then_some(row)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edits::apply;
    use crate::features::testutil::{ctx_at, doc};

    fn action_applied(text: &str, offset: usize, title: &str) -> Option<String> {
        let doc = doc(text);
        let actions = ColumnEdits.actions(&ctx_at(&doc, offset));
        let action = actions.iter().find(|action| action.title == title)?;
        assert_eq!(action.kind, CodeActionKind::REFACTOR);
        Some(apply(&doc.text, &action.edits))
    }

    #[test]
    fn adds_an_empty_column_on_both_sides() {
        let text = "a,b\n1,2\n";
        let at_b = text.find('b').unwrap();
        assert_eq!(
            action_applied(text, at_b, "Add column left of \"b\"").as_deref(),
            Some("a,,b\n1,,2\n")
        );
        assert_eq!(
            action_applied(text, at_b, "Add column right of \"b\"").as_deref(),
            Some("a,b,\n1,2,\n")
        );
    }

    #[test]
    fn the_first_column_works_on_both_sides() {
        let text = "a,b\n1,2\n";
        assert_eq!(
            action_applied(text, 0, "Add column left of \"a\"").as_deref(),
            Some(",a,b\n,1,2\n")
        );
        assert_eq!(
            action_applied(text, 0, "Add column right of \"a\"").as_deref(),
            Some("a,,b\n1,,2\n")
        );
    }

    #[test]
    fn insertion_respects_alignment_padding() {
        let text = "aa, b \ncc, d \n";
        let offset = text.find(" b ").unwrap() + 1;
        assert_eq!(
            action_applied(text, offset, "Add column left of \"b\"").as_deref(),
            Some("aa,, b \ncc,, d \n")
        );
        assert_eq!(
            action_applied(text, offset, "Add column right of \"b\"").as_deref(),
            Some("aa, b ,\ncc, d ,\n")
        );
    }

    #[test]
    fn short_blank_and_error_rows_are_untouched() {
        let text = "a,b\n\n1,2\nx\n5\" z,w\n";
        let at_b = text.find('b').unwrap();
        assert_eq!(
            action_applied(text, at_b, "Add column left of \"b\"").as_deref(),
            Some("a,,b\n\n1,,2\nx\n5\" z,w\n")
        );
    }

    #[test]
    fn a_cursor_on_the_delimiter_edits_the_left_cell() {
        let text = "a,b\n1,2\n";
        // Offset 1 is the comma: column 0.
        assert_eq!(
            action_applied(text, 1, "Add column right of \"a\"").as_deref(),
            Some("a,,b\n1,,2\n")
        );
    }

    #[test]
    fn deleting_an_inner_column_takes_the_preceding_delimiter() {
        let text = "a,b,c\n1,2,3\n";
        assert_eq!(
            action_applied(text, text.find('b').unwrap(), "Delete column \"b\"").as_deref(),
            Some("a,c\n1,3\n")
        );
    }

    #[test]
    fn deleting_the_first_column_takes_the_following_delimiter() {
        let text = "a,b,c\n1,2,3\n";
        assert_eq!(
            action_applied(text, 0, "Delete column \"a\"").as_deref(),
            Some("b,c\n2,3\n")
        );
    }

    #[test]
    fn quoted_cells_are_deleted_wholesale() {
        let text = "x,\"a,b\"\ny,z\n";
        let offset = text.find("\"a").unwrap();
        assert_eq!(
            action_applied(text, offset, "Delete column \"a,b\"").as_deref(),
            Some("x\ny\n")
        );
    }

    #[test]
    fn deleting_the_only_column_leaves_blank_lines() {
        let text = "a\nb\n";
        assert_eq!(
            action_applied(text, 0, "Delete column \"a\"").as_deref(),
            Some("\n\n")
        );
    }

    #[test]
    fn column_content_spans_cover_clean_rows_only() {
        let text = "id,name\n1,x\n\n2,\n5\" b,y\nz\n";
        let doc = doc(text);
        let spans = column_content_spans(&doc.table, 1);

        // Header + `x` + the empty cell; the blank row, the stray-quote row
        // and the short `z` row contribute nothing.
        let slices: Vec<_> = spans.iter().map(|span| span.slice(text)).collect();
        assert_eq!(slices, ["name", "x", ""]);
        assert!(spans[2].is_empty());
        assert_eq!(spans[2].start, text.find("2,").unwrap() + 2);
    }

    #[test]
    fn short_rows_keep_their_text_on_delete() {
        let text = "a,b\n1\n";
        assert_eq!(
            action_applied(text, text.find('b').unwrap(), "Delete column \"b\"").as_deref(),
            Some("a\n1\n")
        );
    }
}
