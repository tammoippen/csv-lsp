//! Compact columns: strip the alignment padding that `align` adds —
//! restoring the canonical, machine-friendly form of the file.

use lsp_types::CodeActionKind;

use crate::document::Document;
use crate::edits::minimize;
use crate::features::{Action, ActionContext, ActionProvider};
use crate::parse::Span;
use crate::render::{RenderOptions, render};

/// Whole-document compaction as a minimal edit; empty when the document
/// carries no padding.
pub fn compact_edits(doc: &Document) -> Vec<(Span, String)> {
    let compact = render(
        &doc.text,
        &doc.table,
        &RenderOptions::compact_for(&doc.table),
    );
    minimize(&doc.text, &compact)
}

/// The `Compact columns` source action.
pub struct CompactColumns;

impl ActionProvider for CompactColumns {
    fn name(&self) -> &'static str {
        "compact-columns"
    }

    fn actions(&self, ctx: &ActionContext) -> Vec<Action> {
        let edits = compact_edits(ctx.doc);
        if edits.is_empty() {
            return Vec::new();
        }
        vec![Action {
            title: "Compact columns".to_owned(),
            kind: CodeActionKind::SOURCE,
            edits,
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
    fn compact_edits_strip_the_padding() {
        let doc = doc("id,name\n1 ,x\n");
        let edits = compact_edits(&doc);

        let mut text = doc.text.clone();
        for (span, replacement) in edits.iter().rev() {
            text.replace_range(span.start..span.end, replacement);
        }
        assert_eq!(text, "id,name\n1,x\n");
    }

    #[test]
    fn compact_documents_offer_no_action() {
        let doc = doc("id,name\n1,x\n");
        assert_eq!(CompactColumns.actions(&ctx_at(&doc, 0)), Vec::new());
    }

    #[test]
    fn padded_documents_offer_the_source_action() {
        let doc = doc("id,name\n1 ,x\n");
        let actions = CompactColumns.actions(&ctx_at(&doc, 0));
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Compact columns");
        assert_eq!(actions[0].kind, CodeActionKind::SOURCE);
    }
}
