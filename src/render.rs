//! Rendering a parsed table back to text: the shared engine behind align,
//! compact and (future) dialect transforms.
//!
//! Rows overlapping a parse error are emitted **verbatim** — the renderer
//! never reformats what the parser could not fully understand.

use std::collections::HashSet;

use unicode_width::UnicodeWidthStr;

use crate::parse::Table;

/// Display width of every column (Unicode width, UAX #11), measured over
/// the content spans of clean rows. Blank rows and rows with parse errors
/// contribute nothing (they are passed through verbatim when rendering).
pub fn column_widths(text: &str, table: &Table) -> Vec<usize> {
    let error_rows: HashSet<usize> = table.errors.iter().map(|error| error.row).collect();
    let mut widths = Vec::new();
    for (index, row) in table.rows.iter().enumerate() {
        if row.is_blank() || error_rows.contains(&index) {
            continue;
        }
        for (column, cell) in row.cells.iter().enumerate() {
            let width = cell.content_span.slice(text).width();
            if column == widths.len() {
                widths.push(width);
            } else {
                widths[column] = widths[column].max(width);
            }
        }
    }
    widths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::Dialect;
    use crate::parse::parse;

    fn widths(text: &str) -> Vec<usize> {
        column_widths(text, &parse(text, Dialect::Csv))
    }

    #[test]
    fn widths_measure_display_cells_not_bytes() {
        // héllo: 5 display cells despite 6 bytes; 名前: 4 cells from 2 chars.
        assert_eq!(widths("id,name\n1,héllo\n999,名前\n"), [3, 5]);
    }

    #[test]
    fn quoted_cells_measure_with_their_quotes() {
        assert_eq!(widths("id,x\n1,\"a,b\"\n"), [2, 5]);
    }

    #[test]
    fn blank_and_error_rows_contribute_nothing() {
        assert_eq!(widths("a,b\n\n\"broken bar,baz\n"), [1, 1]);
    }

    #[test]
    fn ragged_long_rows_extend_the_column_list() {
        assert_eq!(widths("a,b\n1,22,333\n"), [1, 2, 3]);
    }

    #[test]
    fn empty_documents_have_no_columns() {
        assert_eq!(widths(""), Vec::<usize>::new());
        assert_eq!(widths("\n\n"), Vec::<usize>::new());
    }
}
