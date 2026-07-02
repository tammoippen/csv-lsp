//! Align columns: pad cells with spaces so the delimiters line up under
//! the header. Exposed twice — as `textDocument/formatting` (`:format` in
//! Helix) and as an `Align columns` source action — over one code path.

use lsp_types::CodeActionKind;

use crate::document::Document;
use crate::edits::minimize;
use crate::features::{Action, ActionContext, ActionProvider};
use crate::parse::Span;
use crate::render::{RenderOptions, column_widths, render};

/// Whole-document alignment as a minimal edit; empty when the document is
/// already aligned (which makes formatting idempotent).
pub fn align_edits(doc: &Document) -> Vec<(Span, String)> {
    let widths = column_widths(&doc.text, &doc.table);
    let aligned = render(
        &doc.text,
        &doc.table,
        &RenderOptions::aligned_for(&doc.table, widths),
    );
    minimize(&doc.text, &aligned)
}

/// The `Align columns` source action.
pub struct AlignColumns;

impl ActionProvider for AlignColumns {
    fn name(&self) -> &'static str {
        "align-columns"
    }

    fn actions(&self, ctx: &ActionContext) -> Vec<Action> {
        let edits = align_edits(ctx.doc);
        if edits.is_empty() {
            // Already aligned — no no-op entry in the picker.
            return Vec::new();
        }
        vec![Action {
            title: "Align columns".to_owned(),
            kind: CodeActionKind::SOURCE,
            edits,
            command: None,
            dialect_change: None,
            fixes: Vec::new(),
            is_preferred: false,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::testutil::{ctx_at, doc};

    #[test]
    fn align_edits_produce_the_aligned_document() {
        let doc = doc("id,name\n1,x\n");
        let edits = align_edits(&doc);
        assert_eq!(edits.len(), 1);

        let mut text = doc.text.clone();
        let (span, replacement) = &edits[0];
        text.replace_range(span.start..span.end, replacement);
        assert_eq!(text, "id,name\n1 ,x\n");
    }

    #[test]
    fn aligned_documents_need_no_edits_and_offer_no_action() {
        let doc = doc("id,name\n1 ,x\n");
        assert_eq!(align_edits(&doc), Vec::new());
        assert_eq!(AlignColumns.actions(&ctx_at(&doc, 0)), Vec::new());
    }

    #[test]
    fn unaligned_documents_offer_the_source_action() {
        let doc = doc("id,name\n1,x\n");
        let actions = AlignColumns.actions(&ctx_at(&doc, 0));
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Align columns");
        assert_eq!(actions[0].kind, CodeActionKind::SOURCE);
    }
}
