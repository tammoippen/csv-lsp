//! Quickfix: pad short rows with empty cells (trailing delimiters).
//!
//! `x,y` under a 4-column header becomes `x,y,,` — two inserted delimiters
//! are two empty cells. The insert lands at the row end, after any
//! alignment padding (the parser treats that padding as part of the last
//! cell).

use lsp_types::CodeActionKind;

use crate::features::ragged_rows::{ShortRow, missing_cells_diag, short_rows};
use crate::features::{Action, ActionContext, ActionProvider};
use crate::parse::{Span, Table};

/// Per-row quickfixes for short rows under the cursor, plus a whole-file
/// `source.fixAll` that pads every short row at once.
pub struct PadRows;

impl ActionProvider for PadRows {
    fn name(&self) -> &'static str {
        "pad-rows"
    }

    fn actions(&self, ctx: &ActionContext) -> Vec<Action> {
        let table = &ctx.doc.table;
        let shorts = short_rows(table);
        let delimiter = (table.dialect.delimiter() as char).to_string();

        let mut actions = Vec::new();
        for &short in &shorts {
            if !ctx.intersects(table.rows[short.row].span) {
                continue;
            }
            let noun = if short.missing == 1 { "cell" } else { "cells" };
            actions.push(Action {
                title: format!("Pad row with {} empty {noun}", short.missing),
                kind: CodeActionKind::QUICKFIX,
                edits: vec![pad_edit(table, short, &delimiter)],
                fixes: vec![missing_cells_diag(table, short)],
                is_preferred: true,
            });
        }
        // Whole-file repair, offered wherever the cursor is (row order keeps
        // the edits non-overlapping).
        if !shorts.is_empty() {
            actions.push(Action {
                title: format!("Pad all short rows ({})", shorts.len()),
                kind: CodeActionKind::SOURCE_FIX_ALL,
                edits: shorts
                    .iter()
                    .map(|&short| pad_edit(table, short, &delimiter))
                    .collect(),
                fixes: shorts
                    .iter()
                    .map(|&short| missing_cells_diag(table, short))
                    .collect(),
                is_preferred: false,
            });
        }
        actions
    }
}

/// The insert that completes a short row: N delimiters at the row end.
fn pad_edit(table: &Table, short: ShortRow, delimiter: &str) -> (Span, String) {
    let end = table.rows[short.row].span.end;
    (Span::new(end, end), delimiter.repeat(short.missing))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::testutil::{ctx_at, doc};

    #[test]
    fn cursor_on_a_short_row_offers_the_pad_quickfix() {
        let doc = doc("a,b,c\n1,2\nx\n");
        let offset = doc.text.find("1,2").unwrap();
        let ctx = ctx_at(&doc, offset);
        let actions = PadRows.actions(&ctx);

        let action = actions
            .iter()
            .find(|action| action.kind == CodeActionKind::QUICKFIX)
            .expect("a quickfix for the short row under the cursor");
        assert_eq!(action.title, "Pad row with 1 empty cell");
        assert_eq!(action.kind, CodeActionKind::QUICKFIX);
        assert!(action.is_preferred);

        let (span, text) = &action.edits[0];
        assert!(span.is_empty());
        assert_eq!(span.start, doc.text.find("1,2").unwrap() + 3); // row end
        assert_eq!(text, ",");

        assert_eq!(action.fixes.len(), 1);
        assert_eq!(action.fixes[0].code, "row-missing-cells");
    }

    #[test]
    fn several_missing_cells_pad_in_one_edit() {
        let doc = doc("a,b,c\nx\n");
        let ctx = ctx_at(&doc, doc.text.len() - 2); // on the x row
        let actions = PadRows.actions(&ctx);
        assert_eq!(actions[0].kind, CodeActionKind::QUICKFIX);
        assert_eq!(actions[0].title, "Pad row with 2 empty cells");
        assert_eq!(actions[0].edits[0].1, ",,");
    }

    #[test]
    fn the_insert_uses_the_documents_delimiter() {
        let doc = crate::document::Document::new(
            "file:///t.tsv".parse().unwrap(),
            "tsv",
            1,
            "a\tb\n1\n".to_owned(),
        );
        let ctx = ctx_at(&doc, doc.text.len() - 1);
        let actions = PadRows.actions(&ctx);
        assert_eq!(actions[0].edits[0].1, "\t");
    }

    #[test]
    fn complete_rows_and_the_header_get_no_quickfix() {
        let doc = doc("a,b,c\n1,2\n");
        // Cursor in the (complete) header row.
        let actions = PadRows.actions(&ctx_at(&doc, 0));
        assert!(
            actions
                .iter()
                .all(|action| action.kind != CodeActionKind::QUICKFIX)
        );
    }

    #[test]
    fn clean_files_offer_nothing() {
        let doc = doc("a,b\n1,2\n");
        assert_eq!(PadRows.actions(&ctx_at(&doc, 0)), Vec::new());
    }

    #[test]
    fn fix_all_pads_every_short_row_from_anywhere() {
        let doc = doc("a,b,c\n1,2\nx\n");
        // Cursor in the header — far away from both short rows.
        let actions = PadRows.actions(&ctx_at(&doc, 0));

        assert_eq!(actions.len(), 1);
        let fix_all = &actions[0];
        assert_eq!(fix_all.title, "Pad all short rows (2)");
        assert_eq!(fix_all.kind, CodeActionKind::SOURCE_FIX_ALL);
        assert!(!fix_all.is_preferred);
        assert_eq!(fix_all.edits.len(), 2);
        assert_eq!(fix_all.edits[0].1, ",");
        assert_eq!(fix_all.edits[1].1, ",,");
        // In document order, so a client can apply them naively.
        assert!(fix_all.edits[0].0.start < fix_all.edits[1].0.start);
        assert_eq!(fix_all.fixes.len(), 2);
    }

    #[test]
    fn cursor_on_a_short_row_offers_quickfix_and_fix_all() {
        let doc = doc("a,b,c\n1,2\nx\n");
        let actions = PadRows.actions(&ctx_at(&doc, doc.text.find("1,2").unwrap()));
        let kinds: Vec<_> = actions.iter().map(|action| action.kind.clone()).collect();
        assert_eq!(
            kinds,
            [CodeActionKind::QUICKFIX, CodeActionKind::SOURCE_FIX_ALL]
        );
    }
}
