//! Reinterpret the document under a different dialect — zero text changes,
//! only how the server *parses* it. The escape hatch for files whose
//! extension lies (a `.csv` that is actually semicolon-separated).
//!
//! The action carries no edit; picking it makes the client call
//! `workspace/executeCommand` with [`crate::capabilities::SET_DIALECT_COMMAND`],
//! and the server flips the dialect, reparses and republishes diagnostics.
//! Session-scoped by design: on reopen the extension wins again — the durable
//! fixes are renaming the file or converting its content.

use lsp_types::CodeActionKind;

use crate::dialect::Dialect;
use crate::features::{Action, ActionContext, ActionProvider, ServerCommand};

/// `Reinterpret as …` for every non-current dialect.
pub struct ReinterpretDialect;

impl ActionProvider for ReinterpretDialect {
    fn name(&self) -> &'static str {
        "reinterpret-dialect"
    }

    fn actions(&self, ctx: &ActionContext) -> Vec<Action> {
        Dialect::ALL
            .into_iter()
            .filter(|&dialect| dialect != ctx.doc.dialect)
            .map(|dialect| Action {
                title: format!("Reinterpret as {}", dialect.name()),
                kind: CodeActionKind::SOURCE,
                edits: Vec::new(),
                command: Some(ServerCommand::SetDialect { dialect }),
                dialect_change: None,
                fixes: Vec::new(),
                is_preferred: false,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::testutil::{ctx_at, doc};

    #[test]
    fn offers_every_non_current_dialect() {
        let doc = doc("a,b\n"); // languageId csv
        let actions = ReinterpretDialect.actions(&ctx_at(&doc, 0));

        let titles: Vec<_> = actions.iter().map(|action| action.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Reinterpret as TSV",
                "Reinterpret as SSV",
                "Reinterpret as PSV"
            ]
        );
        for action in &actions {
            assert_eq!(action.kind, CodeActionKind::SOURCE);
            assert!(action.edits.is_empty());
        }
        assert_eq!(
            actions[1].command,
            Some(ServerCommand::SetDialect {
                dialect: Dialect::Ssv
            })
        );
    }

    #[test]
    fn the_current_dialect_reflects_prior_reinterpretation() {
        let doc = crate::document::Document::new(
            "file:///t/x.txt".parse().unwrap(),
            "ssv",
            1,
            "a;b\n".to_owned(),
        );
        let actions = ReinterpretDialect.actions(&ctx_at(&doc, 0));
        let titles: Vec<_> = actions.iter().map(|action| action.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Reinterpret as CSV",
                "Reinterpret as TSV",
                "Reinterpret as PSV"
            ]
        );
    }
}
