//! Dialect (delimiter) handling and detection.
//!
//! The supported dialects differ only in their delimiter byte; quoting
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
    /// Pipe-separated (`|`) — common in database dumps and log exports.
    Psv,
}

impl Dialect {
    /// Every dialect, in detection-priority order (ties in [`Self::sniff`]
    /// and action listings follow this order).
    pub const ALL: [Dialect; 4] = [Dialect::Csv, Dialect::Tsv, Dialect::Ssv, Dialect::Psv];

    /// Human-readable dialect name for action titles and messages.
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Csv => "CSV",
            Dialect::Tsv => "TSV",
            Dialect::Ssv => "SSV",
            Dialect::Psv => "PSV",
        }
    }

    /// The delimiter byte of this dialect.
    pub fn delimiter(self) -> u8 {
        match self {
            Dialect::Csv => b',',
            Dialect::Tsv => b'\t',
            Dialect::Ssv => b';',
            Dialect::Psv => b'|',
        }
    }

    /// Map an LSP `languageId` (e.g. from `didOpen`) to a dialect.
    pub fn from_language_id(id: &str) -> Option<Self> {
        match id.to_ascii_lowercase().as_str() {
            "csv" => Some(Dialect::Csv),
            "tsv" => Some(Dialect::Tsv),
            "ssv" => Some(Dialect::Ssv),
            "psv" => Some(Dialect::Psv),
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
            "psv" => Some(Dialect::Psv),
            _ => None,
        }
    }

    /// Guess the dialect by counting candidate delimiters (outside quotes) in
    /// the first non-blank line. Last resort after `languageId` and file
    /// extension; ties are biased towards CSV (documented in the README).
    pub fn sniff(text: &str) -> Option<Self> {
        let line = text.lines().find(|line| !line.trim().is_empty())?;
        let mut counts = [0usize; Self::ALL.len()];
        let mut in_quotes = false;
        for byte in line.bytes() {
            if byte == b'"' {
                in_quotes = !in_quotes;
            } else if !in_quotes {
                for (count, dialect) in counts.iter_mut().zip(Self::ALL) {
                    if byte == dialect.delimiter() {
                        *count += 1;
                    }
                }
            }
        }
        // `ALL` is ordered so that ties fall to the earlier (more common)
        // dialect.
        let mut best = None;
        let mut best_count = 0;
        for (count, dialect) in counts.into_iter().zip(Self::ALL) {
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
        assert_eq!(Dialect::Psv.name(), "PSV");
    }

    #[test]
    fn delimiters_per_dialect() {
        assert_eq!(Dialect::Csv.delimiter(), b',');
        assert_eq!(Dialect::Tsv.delimiter(), b'\t');
        assert_eq!(Dialect::Ssv.delimiter(), b';');
        assert_eq!(Dialect::Psv.delimiter(), b'|');
    }

    #[test]
    fn all_lists_every_dialect_once() {
        for dialect in [Dialect::Csv, Dialect::Tsv, Dialect::Ssv, Dialect::Psv] {
            assert_eq!(Dialect::ALL.iter().filter(|&&d| d == dialect).count(), 1);
        }
    }

    #[test]
    fn language_ids_map_case_insensitively() {
        assert_eq!(Dialect::from_language_id("csv"), Some(Dialect::Csv));
        assert_eq!(Dialect::from_language_id("TSV"), Some(Dialect::Tsv));
        assert_eq!(Dialect::from_language_id("ssv"), Some(Dialect::Ssv));
        assert_eq!(Dialect::from_language_id("psv"), Some(Dialect::Psv));
        assert_eq!(Dialect::from_language_id("plaintext"), None);
    }

    #[test]
    fn extensions_map_from_paths_and_uris() {
        assert_eq!(Dialect::from_path("data.csv"), Some(Dialect::Csv));
        assert_eq!(Dialect::from_path("/a/b/x.TSV"), Some(Dialect::Tsv));
        assert_eq!(Dialect::from_path("file:///w/x.tab"), Some(Dialect::Tsv));
        assert_eq!(Dialect::from_path("x.ssv"), Some(Dialect::Ssv));
        assert_eq!(Dialect::from_path("dump.psv"), Some(Dialect::Psv));
        assert_eq!(Dialect::from_path("notes.txt"), None);
        assert_eq!(Dialect::from_path("no_extension"), None);
        assert_eq!(Dialect::from_path("/dotted.dir/no_extension"), None);
    }

    #[test]
    fn sniff_picks_the_most_frequent_delimiter() {
        assert_eq!(Dialect::sniff("a,b,c\n"), Some(Dialect::Csv));
        assert_eq!(Dialect::sniff("a\tb\tc\n"), Some(Dialect::Tsv));
        assert_eq!(Dialect::sniff("a;b;c\n"), Some(Dialect::Ssv));
        assert_eq!(Dialect::sniff("a|b|c\n"), Some(Dialect::Psv));
    }

    #[test]
    fn sniff_skips_leading_blank_lines() {
        assert_eq!(Dialect::sniff("\n  \na;b\n"), Some(Dialect::Ssv));
    }

    #[test]
    fn sniff_ignores_delimiters_inside_quotes() {
        assert_eq!(Dialect::sniff("\"a,b\";c\n"), Some(Dialect::Ssv));
        assert_eq!(Dialect::sniff("\"a;b\"|c\n"), Some(Dialect::Psv));
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
        assert_eq!(Dialect::sniff("a,b|c\n"), Some(Dialect::Csv));
        assert_eq!(Dialect::sniff("a;b|c\n"), Some(Dialect::Ssv));
    }
}
