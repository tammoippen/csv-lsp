//! Text geometry: byte offsets ↔ LSP positions.
//!
//! Everything inside the server works in **byte offsets** into the document
//! text. This module is the single place where those offsets are converted to
//! and from LSP `(line, character)` positions, honoring the position encoding
//! negotiated with the client (see `docs/plan/m0-scaffold.md`).

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
}
