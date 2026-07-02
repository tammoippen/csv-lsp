//! Dialect (delimiter) handling and detection.
//!
//! The three supported dialects differ only in their delimiter byte; quoting
//! rules are identical. Detection order (first hit wins) lives in
//! `Document`: LSP `languageId` → file extension → content sniffing → CSV.

/// A delimiter-separated dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// Comma-separated (`,`).
    Csv,
    /// Tab-separated (`\t`).
    Tsv,
    /// Semicolon-separated (`;`) — common in European locale exports where
    /// `,` is the decimal separator.
    Ssv,
}

impl Dialect {
    /// The delimiter byte of this dialect.
    pub fn delimiter(self) -> u8 {
        match self {
            Dialect::Csv => b',',
            Dialect::Tsv => b'\t',
            Dialect::Ssv => b';',
        }
    }

    /// Map an LSP `languageId` (e.g. from `didOpen`) to a dialect.
    pub fn from_language_id(id: &str) -> Option<Self> {
        match id.to_ascii_lowercase().as_str() {
            "csv" => Some(Dialect::Csv),
            "tsv" => Some(Dialect::Tsv),
            "ssv" => Some(Dialect::Ssv),
            _ => None,
        }
    }

    /// Map a path or URI string to a dialect via its file extension.
    pub fn from_path(path: &str) -> Option<Self> {
        let file_name = path.rsplit('/').next().unwrap_or(path);
        let (_, extension) = file_name.rsplit_once('.')?;
        match extension.to_ascii_lowercase().as_str() {
            "csv" => Some(Dialect::Csv),
            "tsv" | "tab" => Some(Dialect::Tsv),
            "ssv" => Some(Dialect::Ssv),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delimiters_per_dialect() {
        assert_eq!(Dialect::Csv.delimiter(), b',');
        assert_eq!(Dialect::Tsv.delimiter(), b'\t');
        assert_eq!(Dialect::Ssv.delimiter(), b';');
    }

    #[test]
    fn language_ids_map_case_insensitively() {
        assert_eq!(Dialect::from_language_id("csv"), Some(Dialect::Csv));
        assert_eq!(Dialect::from_language_id("TSV"), Some(Dialect::Tsv));
        assert_eq!(Dialect::from_language_id("ssv"), Some(Dialect::Ssv));
        assert_eq!(Dialect::from_language_id("plaintext"), None);
    }

    #[test]
    fn extensions_map_from_paths_and_uris() {
        assert_eq!(Dialect::from_path("data.csv"), Some(Dialect::Csv));
        assert_eq!(Dialect::from_path("/a/b/x.TSV"), Some(Dialect::Tsv));
        assert_eq!(Dialect::from_path("file:///w/x.tab"), Some(Dialect::Tsv));
        assert_eq!(Dialect::from_path("x.ssv"), Some(Dialect::Ssv));
        assert_eq!(Dialect::from_path("notes.txt"), None);
        assert_eq!(Dialect::from_path("no_extension"), None);
        assert_eq!(Dialect::from_path("/dotted.dir/no_extension"), None);
    }
}
