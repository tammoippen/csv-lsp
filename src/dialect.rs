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
    /// Human-readable dialect name for action titles and messages.
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Csv => "CSV",
            Dialect::Tsv => "TSV",
            Dialect::Ssv => "SSV",
        }
    }

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

    /// Guess the dialect by counting candidate delimiters (outside quotes) in
    /// the first non-blank line. Last resort after `languageId` and file
    /// extension; ties are biased towards CSV (documented in the README).
    pub fn sniff(text: &str) -> Option<Self> {
        let line = text.lines().find(|line| !line.trim().is_empty())?;
        let (mut commas, mut tabs, mut semicolons) = (0usize, 0usize, 0usize);
        let mut in_quotes = false;
        for byte in line.bytes() {
            match byte {
                b'"' => in_quotes = !in_quotes,
                b',' if !in_quotes => commas += 1,
                b'\t' if !in_quotes => tabs += 1,
                b';' if !in_quotes => semicolons += 1,
                _ => {}
            }
        }
        // Ordered so that ties fall to the earlier (more common) dialect.
        let counted = [
            (commas, Dialect::Csv),
            (tabs, Dialect::Tsv),
            (semicolons, Dialect::Ssv),
        ];
        let mut best = None;
        let mut best_count = 0;
        for (count, dialect) in counted {
            if count > best_count {
                best_count = count;
                best = Some(dialect);
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_read_like_users_expect() {
        assert_eq!(Dialect::Csv.name(), "CSV");
        assert_eq!(Dialect::Tsv.name(), "TSV");
        assert_eq!(Dialect::Ssv.name(), "SSV");
    }

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

    #[test]
    fn sniff_picks_the_most_frequent_delimiter() {
        assert_eq!(Dialect::sniff("a,b,c\n"), Some(Dialect::Csv));
        assert_eq!(Dialect::sniff("a\tb\tc\n"), Some(Dialect::Tsv));
        assert_eq!(Dialect::sniff("a;b;c\n"), Some(Dialect::Ssv));
    }

    #[test]
    fn sniff_skips_leading_blank_lines() {
        assert_eq!(Dialect::sniff("\n  \na;b\n"), Some(Dialect::Ssv));
    }

    #[test]
    fn sniff_ignores_delimiters_inside_quotes() {
        assert_eq!(Dialect::sniff("\"a,b\";c\n"), Some(Dialect::Ssv));
    }

    #[test]
    fn sniff_returns_none_without_delimiters() {
        assert_eq!(Dialect::sniff(""), None);
        assert_eq!(Dialect::sniff("   \n\n"), None);
        assert_eq!(Dialect::sniff("plain text\n"), None);
    }

    #[test]
    fn sniff_breaks_ties_towards_csv() {
        assert_eq!(Dialect::sniff("a,b;c\n"), Some(Dialect::Csv));
    }
}
