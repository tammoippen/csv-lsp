//! The feature framework: diagnostic rules (and, from M2 on, code-action
//! providers) collected in a [`Registry`].
//!
//! **Adding a feature = one new module here + one line in
//! [`Registry::standard`].** Everything works on byte spans over the parsed
//! [`Table`]; the server converts to LSP types at its boundary only.

pub mod align;
pub mod compact;
pub mod pad_rows;
pub mod parse_errors;
pub mod ragged_rows;
pub mod reinterpret;
pub mod transform;

use lsp_types::CodeActionKind;

use crate::dialect::Dialect;
use crate::document::Document;
use crate::parse::{Span, Table};

/// A diagnostic in crate-internal form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    /// The offending bytes (zero-width spans are legal and render as a
    /// caret / end-of-line marker in editors).
    pub span: Span,
    /// How loud the editor should be about it.
    pub severity: Severity,
    /// Stable machine-readable code (e.g. `row-missing-cells`), also shown
    /// to users next to the message.
    pub code: &'static str,
    /// Human-readable: states the fact and the expectation, lowercase, no
    /// trailing period.
    pub message: String,
    /// Optional structured payload, echoed back by clients in code-action
    /// requests. Never *relied* upon — providers recompute from the table.
    pub data: Option<serde_json::Value>,
}

/// Diagnostic severity, mapped to `lsp_types::DiagnosticSeverity` at the
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The file's structure is broken.
    Error,
    /// Suspicious but parseable (e.g. a stray quote).
    Warning,
    /// Informational.
    Info,
    /// Editor hint.
    Hint,
}

/// A check over the parsed table producing diagnostics.
pub trait DiagnosticRule {
    /// Stable rule name (for logs).
    fn name(&self) -> &'static str;
    /// Run the rule. Must not panic on any table the parser can produce.
    fn check(&self, text: &str, table: &Table) -> Vec<Diag>;
}

/// Everything an action provider may consider.
pub struct ActionContext<'a> {
    /// The document: text, dialect, parse result, line index.
    pub doc: &'a Document,
    /// The requested range as a byte span (zero-width for a bare cursor).
    pub range: Span,
    /// The client's diagnostics overlapping the range — response linkage
    /// only, never an input: providers recompute from the table.
    pub client_diagnostics: &'a [lsp_types::Diagnostic],
    /// The client's kind filter; applied centrally by the registry.
    pub only: Option<&'a [CodeActionKind]>,
}

impl ActionContext<'_> {
    /// The `(row, column)` under the cursor (= the range start), following
    /// [`Table::cell_at`]'s conventions (delimiter → left cell, row end
    /// inclusive).
    pub fn cell_at_cursor(&self) -> Option<(usize, usize)> {
        self.doc.table.cell_at(self.range.start)
    }

    /// The column under the cursor.
    pub fn column_at_cursor(&self) -> Option<usize> {
        self.cell_at_cursor().map(|(_, column)| column)
    }

    /// Inclusive-touch intersection with the requested range: a zero-width
    /// cursor sitting exactly at a span's end still counts.
    pub fn intersects(&self, span: Span) -> bool {
        span.start <= self.range.end && self.range.start <= span.end
    }
}

/// A command the server executes on itself when the user picks the action
/// (`workspace/executeCommand`) — the channel for actions that change server
/// state instead of document text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerCommand {
    /// Re-parse the document under a different dialect.
    SetDialect {
        /// The dialect to switch to.
        dialect: Dialect,
    },
}

/// A code action in crate-internal form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// Shown verbatim in the editor's picker.
    pub title: String,
    /// LSP kind (`quickfix`, `source`, …); drives client-side grouping.
    pub kind: CodeActionKind,
    /// Replacements (span → new text): non-overlapping, in document order.
    /// Empty only for pure command actions — a no-op text action must
    /// simply not be offered.
    pub edits: Vec<(Span, String)>,
    /// Server-side effect executed when the user picks the action.
    pub command: Option<ServerCommand>,
    /// The dialect the document will be in once the edits are applied —
    /// the server watches for the matching `didChange` and flips the
    /// document's dialect so diagnostics stay coherent after a conversion.
    pub dialect_change: Option<Dialect>,
    /// The diagnostics this action fixes, for editor linkage.
    pub fixes: Vec<Diag>,
    /// Marks the editor's default choice.
    pub is_preferred: bool,
}

/// A source of code actions.
pub trait ActionProvider {
    /// Stable provider name (for logs).
    fn name(&self) -> &'static str;
    /// Actions applicable to the context. Applicability is recomputed from
    /// the parsed table — never from the client's diagnostics.
    fn actions(&self, ctx: &ActionContext) -> Vec<Action>;
}

/// All registered features.
pub struct Registry {
    rules: Vec<Box<dyn DiagnosticRule>>,
    providers: Vec<Box<dyn ActionProvider>>,
}

impl Registry {
    /// The standard feature set — the single registration point.
    pub fn standard() -> Self {
        Registry {
            rules: vec![
                Box::new(parse_errors::ParseErrors),
                Box::new(ragged_rows::RaggedRows),
            ],
            providers: vec![
                Box::new(pad_rows::PadRows),
                Box::new(align::AlignColumns),
                Box::new(compact::CompactColumns),
                Box::new(reinterpret::ReinterpretDialect),
                Box::new(transform::ConvertDialect),
            ],
        }
    }

    /// All diagnostics for a parsed document, in rule order.
    pub fn diagnostics(&self, text: &str, table: &Table) -> Vec<Diag> {
        self.rules
            .iter()
            .flat_map(|rule| rule.check(text, table))
            .collect()
    }

    /// All applicable actions, in provider order, honoring the client's
    /// `only` kind filter.
    pub fn actions(&self, ctx: &ActionContext) -> Vec<Action> {
        self.providers
            .iter()
            .flat_map(|provider| provider.actions(ctx))
            .filter(|action| kind_matches(ctx.only, &action.kind))
            .collect()
    }
}

/// Dotted-prefix kind matching per the LSP spec: `source` matches
/// `source.fixAll`, but `quickfix` does not match `quickfixes`.
fn kind_matches(only: Option<&[CodeActionKind]>, kind: &CodeActionKind) -> bool {
    let Some(only) = only else {
        return true;
    };
    only.iter().any(|allowed| {
        let allowed = allowed.as_str();
        let kind = kind.as_str();
        kind == allowed
            || (kind.starts_with(allowed) && kind.as_bytes().get(allowed.len()) == Some(&b'.'))
    })
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    /// A CSV document for feature tests.
    pub(crate) fn doc(text: &str) -> Document {
        Document::new("file:///t.csv".parse().unwrap(), "csv", 1, text.to_owned())
    }

    /// A context with a bare cursor at `offset`.
    pub(crate) fn ctx_at(doc: &Document, offset: usize) -> ActionContext<'_> {
        ActionContext {
            doc,
            range: Span::new(offset, offset),
            client_diagnostics: &[],
            only: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::{ctx_at, doc};

    #[test]
    fn cursor_resolves_to_cells_through_the_context() {
        let doc = doc("ab,cd\nef,gh\n");
        assert_eq!(ctx_at(&doc, 4).cell_at_cursor(), Some((0, 1)));
        assert_eq!(ctx_at(&doc, 2).column_at_cursor(), Some(0)); // on the delimiter
        assert_eq!(ctx_at(&doc, 5).cell_at_cursor(), Some((0, 1))); // at the row end
        assert_eq!(ctx_at(&doc, 99).cell_at_cursor(), None);
    }

    #[test]
    fn cursor_resolves_inside_multi_line_quoted_cells() {
        let doc = doc("\"a\nb\",c\n");
        assert_eq!(ctx_at(&doc, 3).cell_at_cursor(), Some((0, 0)));
    }

    #[test]
    fn intersection_counts_touching_spans() {
        let doc = doc("ab,cd\nef,gh\n");
        let row0 = doc.table.rows[0].span; // 0..5
        assert!(ctx_at(&doc, 5).intersects(row0)); // cursor at the row end
        assert!(ctx_at(&doc, 0).intersects(row0));
        assert!(!ctx_at(&doc, 6).intersects(row0)); // next row's start
    }

    #[test]
    fn only_filter_uses_dotted_prefix_semantics() {
        use lsp_types::CodeActionKind;

        use super::kind_matches;

        // No filter: everything passes.
        assert!(kind_matches(None, &CodeActionKind::QUICKFIX));
        // Exact match.
        assert!(kind_matches(
            Some(&[CodeActionKind::SOURCE_FIX_ALL]),
            &CodeActionKind::SOURCE_FIX_ALL
        ));
        // Prefix match down the dotted hierarchy.
        assert!(kind_matches(
            Some(&[CodeActionKind::SOURCE]),
            &CodeActionKind::SOURCE_FIX_ALL
        ));
        // But not the other way around, and not on string prefixes.
        assert!(!kind_matches(
            Some(&[CodeActionKind::SOURCE_FIX_ALL]),
            &CodeActionKind::SOURCE
        ));
        assert!(!kind_matches(
            Some(&[CodeActionKind::new("quick")]),
            &CodeActionKind::QUICKFIX
        ));
        // A quickfix disappears under a source-only filter.
        assert!(!kind_matches(
            Some(&[CodeActionKind::SOURCE]),
            &CodeActionKind::QUICKFIX
        ));
        // An empty filter hides everything.
        assert!(!kind_matches(Some(&[]), &CodeActionKind::QUICKFIX));
    }

    #[test]
    fn the_registry_applies_the_only_filter() {
        use lsp_types::CodeActionKind;

        let doc = doc("a,b,c\n1,2\n");
        let registry = super::Registry::standard();

        let offset = doc.text.find("1,2").unwrap();
        let mut ctx = ctx_at(&doc, offset);
        let kinds = |actions: &[super::Action]| -> Vec<_> {
            actions.iter().map(|a| a.kind.clone()).collect()
        };

        let unfiltered = registry.actions(&ctx);
        assert!(kinds(&unfiltered).contains(&CodeActionKind::QUICKFIX));
        assert!(kinds(&unfiltered).contains(&CodeActionKind::SOURCE_FIX_ALL));

        let only = [CodeActionKind::SOURCE];
        ctx.only = Some(&only);
        let actions = registry.actions(&ctx);
        assert!(!actions.is_empty());
        // Prefix semantics: plain source AND source.fixAll survive; the
        // quickfix does not.
        assert!(kinds(&actions).contains(&CodeActionKind::SOURCE_FIX_ALL));
        assert!(!kinds(&actions).contains(&CodeActionKind::QUICKFIX));

        let only = [CodeActionKind::QUICKFIX];
        ctx.only = Some(&only);
        let actions = registry.actions(&ctx);
        assert_eq!(kinds(&actions), [CodeActionKind::QUICKFIX]);
    }
}
