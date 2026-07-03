//! Convert the document to a different dialect: rewrite the delimiters and
//! adapt quoting (see `docs/features.html#convert`).
//!
//! Conversion emits the compact form (padding is layout for the *old*
//! column widths). Files with parse errors offer no conversions — their
//! rows would pass through verbatim with old delimiters, producing a
//! mixed-dialect file; fix quoting first.

use lsp_types::CodeActionKind;

use crate::dialect::Dialect;
use crate::document::Document;
use crate::edits::minimize;
use crate::features::{Action, ActionContext, ActionProvider};
use crate::parse::Span;
use crate::render::{QuotePolicy, RenderOptions, render};

/// `Convert to …` for every non-current dialect.
pub struct ConvertDialect;

/// The conversion as a minimal edit; empty when converting changes nothing
/// (e.g. single-column files).
pub fn convert_edits(doc: &Document, target: Dialect) -> Vec<(Span, String)> {
    let opts = RenderOptions {
        dialect: target,
        quote_policy: QuotePolicy::PreserveOrRequired,
        ..RenderOptions::compact_for(&doc.table)
    };
    let converted = render(&doc.text, &doc.table, &opts);
    minimize(&doc.text, &converted)
}

impl ActionProvider for ConvertDialect {
    fn name(&self) -> &'static str {
        "convert-dialect"
    }

    fn actions(&self, ctx: &ActionContext) -> Vec<Action> {
        if !ctx.doc.table.errors.is_empty() {
            return Vec::new();
        }
        Dialect::ALL
            .into_iter()
            .filter(|&target| target != ctx.doc.dialect)
            .filter_map(|target| {
                let edits = convert_edits(ctx.doc, target);
                if edits.is_empty() {
                    return None;
                }
                Some(Action {
                    title: format!("Convert to {}", target.name()),
                    kind: CodeActionKind::SOURCE,
                    edits,
                    command: None,
                    dialect_change: Some(target),
                    fixes: Vec::new(),
                    is_preferred: false,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edits::apply;
    use crate::features::testutil::{ctx_at, doc};

    #[test]
    fn offers_conversions_to_the_non_current_dialects() {
        let doc = doc("a,b\n1,2\n");
        let actions = ConvertDialect.actions(&ctx_at(&doc, 0));

        let titles: Vec<_> = actions.iter().map(|action| action.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Convert to TSV", "Convert to SSV", "Convert to PSV"]
        );
        assert_eq!(actions[0].dialect_change, Some(Dialect::Tsv));
        assert_eq!(
            apply(&doc.text, &actions[0].edits),
            "a\tb\n1\t2\n" // delimiters swapped, nothing else
        );
    }

    #[test]
    fn conversion_protects_cells_containing_the_new_delimiter() {
        let doc = crate::document::Document::new(
            "file:///t/preise.ssv".parse().unwrap(),
            "ssv",
            1,
            "artikel;preis\nbolzen;1,50\n".to_owned(),
        );
        let actions = ConvertDialect.actions(&ctx_at(&doc, 0));
        let to_csv = actions
            .iter()
            .find(|action| action.title == "Convert to CSV")
            .expect("csv conversion offered");
        assert_eq!(
            apply(&doc.text, &to_csv.edits),
            "artikel,preis\nbolzen,\"1,50\"\n"
        );
    }

    #[test]
    fn conversion_to_psv_quotes_cells_containing_pipes() {
        let doc = doc("cmd,note\nls,a|b\n");
        let actions = ConvertDialect.actions(&ctx_at(&doc, 0));
        let to_psv = actions
            .iter()
            .find(|action| action.title == "Convert to PSV")
            .expect("psv conversion offered");
        assert_eq!(
            apply(&doc.text, &to_psv.edits),
            "cmd|note\nls|\"a|b\"\n" // the piped cell gains quotes
        );
    }

    #[test]
    fn files_with_parse_errors_offer_no_conversions() {
        let doc = doc("a,b\n5\" bolt,x\n"); // stray quote
        assert_eq!(ConvertDialect.actions(&ctx_at(&doc, 0)), Vec::new());
    }

    #[test]
    fn single_column_files_have_nothing_to_convert() {
        let doc = doc("a\nb\n");
        assert_eq!(ConvertDialect.actions(&ctx_at(&doc, 0)), Vec::new());
    }
}
