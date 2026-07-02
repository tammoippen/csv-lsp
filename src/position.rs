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

    /// Convert an LSP position into a byte offset, clamping per the spec.
    ///
    /// A line past the end of the document maps to the document end; a
    /// character past the end of its line maps to the line end (before the
    /// terminator); a character landing inside a multi-unit character (e.g.
    /// between UTF-16 surrogate halves) snaps back to that character's start.
    pub fn offset(&self, text: &str, pos: lsp_types::Position, enc: PositionEncoding) -> usize {
        let Some(&line_start) = self.line_starts.get(pos.line as usize) else {
            return text.len();
        };
        let content_end = self.line_content_end(text, pos.line as usize);
        let line_text = &text[line_start..content_end];
        let mut remaining = pos.character as usize;
        for (i, ch) in line_text.char_indices() {
            let width = match enc {
                PositionEncoding::Utf8 => ch.len_utf8(),
                PositionEncoding::Utf16 => ch.len_utf16(),
                PositionEncoding::Utf32 => 1,
            };
            if remaining < width {
                return line_start + i;
            }
            remaining -= width;
        }
        content_end
    }

    /// End of a line's content: the offset of its terminator, or `text.len()`
    /// for the last line.
    fn line_content_end(&self, text: &str, line: usize) -> usize {
        let Some(&next_start) = self.line_starts.get(line + 1) else {
            return text.len();
        };
        let bytes = text.as_bytes();
        if next_start >= 2 && bytes[next_start - 2] == b'\r' && bytes[next_start - 1] == b'\n' {
            next_start - 2
        } else {
            next_start - 1
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

    #[test]
    fn offset_round_trips_positions_in_every_encoding() {
        let index = LineIndex::new(MIXED);
        for enc in [
            PositionEncoding::Utf8,
            PositionEncoding::Utf16,
            PositionEncoding::Utf32,
        ] {
            for (offset, _) in MIXED.char_indices() {
                let position = index.position(MIXED, offset, enc);
                assert_eq!(index.offset(MIXED, position, enc), offset, "{enc:?}");
            }
        }
    }

    #[test]
    fn offset_clamps_line_past_the_end_to_document_end() {
        let index = LineIndex::new("ab\ncd");
        assert_eq!(
            index.offset("ab\ncd", pos(99, 0), PositionEncoding::Utf8),
            5
        );
    }

    #[test]
    fn offset_clamps_character_to_the_line_content_end() {
        let text = "ab\r\ncd";
        let index = LineIndex::new(text);
        // Character 99 on line 0 stops before the \r\n terminator.
        assert_eq!(index.offset(text, pos(0, 99), PositionEncoding::Utf8), 2);
        assert_eq!(index.offset(text, pos(1, 99), PositionEncoding::Utf8), 6);
    }

    #[test]
    fn offset_snaps_utf16_surrogate_halves_to_the_char_start() {
        let text = "😀x";
        let index = LineIndex::new(text);
        // Character 1 lands between the surrogate halves of 😀 (units 0..2).
        assert_eq!(index.offset(text, pos(0, 1), PositionEncoding::Utf16), 0);
        assert_eq!(index.offset(text, pos(0, 2), PositionEncoding::Utf16), 4);
    }
}
