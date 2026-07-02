//! Text geometry: byte offsets ↔ LSP positions.
//!
//! Everything inside the server works in **byte offsets** into the document
//! text. This module is the single place where those offsets are converted to
//! and from LSP `(line, character)` positions, honoring the position encoding
//! negotiated with the client (see `docs/plan/m0-scaffold.md`).

/// The character-counting unit negotiated with the client during
/// `initialize`.
///
/// LSP positions are `(line, character)` where `character` counts code units
/// of this encoding — not bytes and not codepoints, unless negotiated so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionEncoding {
    /// Characters count UTF-8 bytes — preferred, offsets are already bytes.
    Utf8,
    /// Characters count UTF-16 code units — the protocol's mandatory default.
    Utf16,
    /// Characters count Unicode codepoints.
    Utf32,
}

impl PositionEncoding {
    /// Length of `s` in code units of this encoding.
    fn measure(self, s: &str) -> usize {
        match self {
            PositionEncoding::Utf8 => s.len(),
            PositionEncoding::Utf16 => s.chars().map(char::len_utf16).sum(),
            PositionEncoding::Utf32 => s.chars().count(),
        }
    }
}

/// Byte offsets of every line start, in ascending order.
///
/// Line breaks follow the LSP specification: `\n`, `\r\n` and lone `\r` all
/// terminate a line. Offset 0 is always present, so line `i` starts at
/// `line_starts[i]` and the vector is never empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    line_starts: Vec<usize>,
}

impl LineIndex {
    /// Index the line starts of `text`.
    pub fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut line_starts = vec![0];
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => {
                    line_starts.push(i + 1);
                    i += 1;
                }
                b'\r' => {
                    if bytes.get(i + 1) == Some(&b'\n') {
                        line_starts.push(i + 2);
                        i += 2;
                    } else {
                        line_starts.push(i + 1);
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        }
        LineIndex { line_starts }
    }

    /// Convert a byte offset into an LSP position.
    ///
    /// `offset` is clamped to `text.len()` and must lie on a `char` boundary
    /// (all spans produced by this crate do).
    pub fn position(
        &self,
        text: &str,
        offset: usize,
        enc: PositionEncoding,
    ) -> lsp_types::Position {
        let offset = offset.min(text.len());
        debug_assert!(text.is_char_boundary(offset));
        let line = self.line_starts.partition_point(|&start| start <= offset) - 1;
        let character = enc.measure(&text[self.line_starts[line]..offset]);
        lsp_types::Position {
            line: line as u32,
            character: character as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_has_one_line() {
        assert_eq!(LineIndex::new("").line_starts, vec![0]);
    }

    #[test]
    fn lf_starts_a_new_line() {
        assert_eq!(LineIndex::new("a\nb").line_starts, vec![0, 2]);
    }

    #[test]
    fn crlf_counts_as_one_break() {
        assert_eq!(LineIndex::new("a\r\nb").line_starts, vec![0, 3]);
    }

    #[test]
    fn lone_cr_is_a_line_break() {
        assert_eq!(LineIndex::new("a\rb").line_starts, vec![0, 2]);
    }

    #[test]
    fn trailing_newline_opens_an_empty_last_line() {
        assert_eq!(LineIndex::new("a\n").line_starts, vec![0, 2]);
    }

    /// Line 1 is `é😀,名`: é = 2 bytes / 1 utf-16 unit, 😀 = 4 bytes / 2
    /// utf-16 units (surrogate pair), 名 = 3 bytes / 1 unit / 2 display cells.
    const MIXED: &str = "id,x\né😀,名\n";

    fn pos(line: u32, character: u32) -> lsp_types::Position {
        lsp_types::Position { line, character }
    }

    #[test]
    fn position_counts_utf8_bytes() {
        let index = LineIndex::new(MIXED);
        let offset = MIXED.find('名').unwrap();
        assert_eq!(
            index.position(MIXED, offset, PositionEncoding::Utf8),
            pos(1, 7)
        );
    }

    #[test]
    fn position_counts_utf16_units() {
        let index = LineIndex::new(MIXED);
        let offset = MIXED.find('名').unwrap();
        assert_eq!(
            index.position(MIXED, offset, PositionEncoding::Utf16),
            pos(1, 4)
        );
    }

    #[test]
    fn position_counts_utf32_codepoints() {
        let index = LineIndex::new(MIXED);
        let offset = MIXED.find('名').unwrap();
        assert_eq!(
            index.position(MIXED, offset, PositionEncoding::Utf32),
            pos(1, 3)
        );
    }

    #[test]
    fn position_at_text_end_is_start_of_trailing_line() {
        let index = LineIndex::new(MIXED);
        assert_eq!(
            index.position(MIXED, MIXED.len(), PositionEncoding::Utf8),
            pos(2, 0)
        );
    }

    #[test]
    fn position_clamps_offsets_past_the_end() {
        let index = LineIndex::new(MIXED);
        assert_eq!(
            index.position(MIXED, 999, PositionEncoding::Utf16),
            pos(2, 0)
        );
    }
}
