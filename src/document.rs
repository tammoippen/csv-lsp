//! Open-document state: text, version, dialect and derived indexes.
//!
//! The server advertises FULL document sync, so every change replaces the
//! whole text; derived state (line index, later the parse tree) is rebuilt
//! eagerly — CSV parsing is O(n) and cheap
//! (see `docs/contributing.html#performance`).

use std::collections::HashMap;

use lsp_types::Uri;

use crate::dialect::Dialect;
use crate::parse::{Table, parse};
use crate::position::LineIndex;

/// A single open document with its derived state.
#[derive(Debug)]
pub struct Document {
    /// The document's URI exactly as sent by the client.
    pub uri: Uri,
    /// Client-side version, bumped on every change.
    pub version: i32,
    /// The full document text.
    pub text: String,
    /// Resolved once at open; stable across edits (a half-typed document
    /// must not flip dialect under the user's cursor).
    pub dialect: Dialect,
    /// Line-start index for position conversion; rebuilt on every change.
    pub line_index: LineIndex,
    /// The parse result; rebuilt on every change.
    pub table: Table,
}

impl Document {
    /// Create a document, resolving the dialect with the precedence
    /// `languageId` → file extension → content sniffing → CSV.
    pub fn new(uri: Uri, language_id: &str, version: i32, text: String) -> Self {
        let dialect = Dialect::from_language_id(language_id)
            .or_else(|| Dialect::from_path(uri.as_str()))
            .or_else(|| Dialect::sniff(&text))
            .unwrap_or(Dialect::Csv);
        let line_index = LineIndex::new(&text);
        let table = parse(&text, dialect);
        Document {
            uri,
            version,
            text,
            dialect,
            line_index,
            table,
        }
    }

    /// Replace the text (FULL sync) and rebuild derived state.
    pub fn update(&mut self, version: i32, text: String) {
        self.version = version;
        self.line_index = LineIndex::new(&text);
        self.table = parse(&text, self.dialect);
        self.text = text;
    }
}

/// All currently open documents, keyed by URI.
///
/// Keys are the URI's string form: `lsp_types::Uri` is treated as opaque
/// throughout the crate to stay insulated from its churn
/// (see `docs/contributing.html#decisions`).
#[derive(Debug, Default)]
pub struct Store {
    docs: HashMap<String, Document>,
}

impl Store {
    /// Track a newly opened document.
    pub fn open(&mut self, uri: Uri, language_id: &str, version: i32, text: String) -> &Document {
        let key = uri.as_str().to_owned();
        let doc = Document::new(uri, language_id, version, text);
        self.docs.entry(key).insert_entry(doc).into_mut()
    }

    /// Apply a FULL-sync change. Returns `None` for unknown documents
    /// (a client protocol error we tolerate). `dialect_override` is set
    /// when the change is a dialect conversion the server offered — the
    /// document re-parses under the new dialect.
    pub fn change(
        &mut self,
        uri: &Uri,
        version: i32,
        text: String,
        dialect_override: Option<Dialect>,
    ) -> Option<&Document> {
        let doc = self.docs.get_mut(uri.as_str())?;
        if let Some(dialect) = dialect_override {
            doc.dialect = dialect;
        }
        doc.update(version, text);
        Some(doc)
    }

    /// Re-parse a document under a different dialect (the
    /// `csv-lsp.setDialect` command); text and version are untouched.
    pub fn set_dialect(&mut self, uri: &Uri, dialect: Dialect) -> Option<&Document> {
        let doc = self.docs.get_mut(uri.as_str())?;
        doc.dialect = dialect;
        doc.table = parse(&doc.text, dialect);
        Some(doc)
    }

    /// Forget a closed document.
    pub fn close(&mut self, uri: &Uri) {
        self.docs.remove(uri.as_str());
    }

    /// Look up an open document.
    pub fn get(&self, uri: &Uri) -> Option<&Document> {
        self.docs.get(uri.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(s: &str) -> Uri {
        s.parse().unwrap()
    }

    #[test]
    fn language_id_outranks_extension() {
        let doc = Document::new(uri("file:///d/x.csv"), "tsv", 1, "a\tb\n".into());
        assert_eq!(doc.dialect, Dialect::Tsv);
    }

    #[test]
    fn extension_outranks_sniffing() {
        let doc = Document::new(uri("file:///d/x.ssv"), "plaintext", 1, "a,b\n".into());
        assert_eq!(doc.dialect, Dialect::Ssv);
    }

    #[test]
    fn sniffing_is_the_content_fallback() {
        let doc = Document::new(uri("file:///d/x.txt"), "plaintext", 1, "a;b\n".into());
        assert_eq!(doc.dialect, Dialect::Ssv);
    }

    #[test]
    fn csv_is_the_final_default() {
        let doc = Document::new(uri("file:///d/data"), "plaintext", 1, "plain\n".into());
        assert_eq!(doc.dialect, Dialect::Csv);
    }

    #[test]
    fn store_tracks_the_document_lifecycle() {
        let mut store = Store::default();
        let u = uri("file:///d/x.csv");
        store.open(u.clone(), "csv", 1, "a,b\n".into());
        assert_eq!(store.get(&u).unwrap().version, 1);

        let doc = store.change(&u, 2, "a,b,c\n".into(), None).unwrap();
        assert_eq!(doc.version, 2);
        assert_eq!(doc.text, "a,b,c\n");

        store.close(&u);
        assert!(store.get(&u).is_none());
    }

    #[test]
    fn change_rebuilds_the_line_index() {
        let mut store = Store::default();
        let u = uri("file:///d/x.csv");
        store.open(u.clone(), "csv", 1, "ab\n".into());
        let doc = store.change(&u, 2, "a\nb\n".into(), None).unwrap();
        let pos = doc
            .line_index
            .position(&doc.text, 2, crate::position::PositionEncoding::Utf8);
        assert_eq!(pos.line, 1);
    }

    #[test]
    fn change_with_override_flips_the_dialect() {
        let mut store = Store::default();
        let u = uri("file:///d/x.csv");
        store.open(u.clone(), "csv", 1, "a,b\n".into());
        let doc = store
            .change(&u, 2, "a\tb\n".into(), Some(Dialect::Tsv))
            .unwrap();
        assert_eq!(doc.dialect, Dialect::Tsv);
        assert_eq!(doc.table.rows[0].cells.len(), 2);
    }

    #[test]
    fn set_dialect_reparses_in_place() {
        let mut store = Store::default();
        let u = uri("file:///d/mislabeled.csv");
        store.open(u.clone(), "csv", 1, "a;b\n".into());
        assert_eq!(store.get(&u).unwrap().table.rows[0].cells.len(), 1);

        let doc = store.set_dialect(&u, Dialect::Ssv).unwrap();
        assert_eq!(doc.dialect, Dialect::Ssv);
        assert_eq!(doc.table.rows[0].cells.len(), 2);
        assert_eq!(doc.version, 1); // untouched

        assert!(
            store
                .set_dialect(&uri("file:///nope"), Dialect::Tsv)
                .is_none()
        );
    }

    #[test]
    fn change_on_unknown_document_is_tolerated() {
        let mut store = Store::default();
        assert!(
            store
                .change(&uri("file:///nope.csv"), 2, String::new(), None)
                .is_none()
        );
    }
}
