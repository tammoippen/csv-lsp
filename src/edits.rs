//! Turning a full re-render into a minimal edit.
//!
//! Whole-document replacements work but scroll the viewport and move the
//! cursor in some clients; trimming the identical outer bytes yields one
//! small replacement instead — and `[]` for identical texts, which makes
//! formatting idempotent at the protocol level.

use crate::parse::Span;

/// The smallest single replacement turning `old` into `new`: the common
/// prefix and suffix are trimmed (snapped to `char` boundaries and off the
/// middle of CRLF breaks). Empty for identical inputs.
pub fn minimize(old: &str, new: &str) -> Vec<(Span, String)> {
    if old == new {
        return Vec::new();
    }
    let old_bytes = old.as_bytes();
    let new_bytes = new.as_bytes();

    let mut prefix = old_bytes
        .iter()
        .zip(new_bytes)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(prefix) {
        prefix -= 1;
    }
    // An LSP position cannot point between the `\r` and `\n` of a CRLF
    // break (characters clamp to the line's *content* end, before the
    // terminator), so a client would misplace such an edit boundary.
    // Widen the edit by one byte instead.
    if splits_crlf(old_bytes, prefix) {
        prefix -= 1;
    }
    // Identical prefix bytes ⇒ the boundary holds in `new` as well.
    debug_assert!(new.is_char_boundary(prefix));

    // The suffix must not reach into the prefix on either side.
    let max_suffix = old.len().min(new.len()) - prefix;
    let mut suffix = old_bytes
        .iter()
        .rev()
        .zip(new_bytes.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix) {
        suffix -= 1;
    }
    if splits_crlf(old_bytes, old.len() - suffix) {
        suffix -= 1;
    }
    debug_assert!(new.is_char_boundary(new.len() - suffix));

    let span = Span::new(prefix, old.len() - suffix);
    let replacement = new[prefix..new.len() - suffix].to_owned();
    vec![(span, replacement)]
}

/// True when `offset` points between the `\r` and `\n` of a CRLF break —
/// the one `char` boundary the LSP position model cannot express.
fn splits_crlf(bytes: &[u8], offset: usize) -> bool {
    offset > 0 && bytes[offset - 1] == b'\r' && bytes.get(offset) == Some(&b'\n')
}

/// Apply non-overlapping, document-ordered edits to `text` (the shape every
/// [`crate::features::Action`] carries). Splices back-to-front so earlier
/// offsets stay valid — the same thing an LSP client does with `TextEdit`s.
pub fn apply(text: &str, edits: &[(Span, String)]) -> String {
    let mut result = text.to_owned();
    for (span, replacement) in edits.iter().rev() {
        result.replace_range(span.start..span.end, replacement);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_texts_need_no_edit() {
        assert_eq!(minimize("a,b\n", "a,b\n"), Vec::new());
    }

    #[test]
    fn apply_splices_multiple_edits_in_document_order() {
        let edits = vec![
            (Span::new(0, 1), "X".to_owned()),
            (Span::new(2, 2), "!".to_owned()), // pure insertion
            (Span::new(3, 4), String::new()),  // deletion
        ];
        assert_eq!(apply("a,b,c", &edits), "X,!bc");
    }

    #[test]
    fn apply_with_no_edits_is_identity() {
        assert_eq!(apply("a,b\n", &[]), "a,b\n");
    }

    #[test]
    fn apply_round_trips_minimize() {
        for (old, new) in [
            ("a,bb,c\n", "a,b,c\n"),
            ("", "x"),
            ("aé,b\n", "aè,b\n"),
            ("same", "same"),
        ] {
            assert_eq!(apply(old, &minimize(old, new)), new);
        }
    }

    #[test]
    fn a_middle_change_yields_one_small_replacement() {
        let old = "a,bb,c\n";
        let new = "a,b,c\n";
        let edits = minimize(old, new);
        assert_eq!(edits.len(), 1);
        let (span, replacement) = &edits[0];
        // Exact bounds are an implementation detail (prefix/suffix greed);
        // the contract is: small, in the middle, and correct.
        assert!(span.start >= 2 && span.end <= 4, "span too wide: {span:?}");
        assert!(replacement.len() <= 1);
        assert_eq!(apply(old, &edits), new);
    }

    #[test]
    fn multibyte_boundaries_are_respected() {
        let old = "aé,b\n";
        let new = "aè,b\n";
        let edits = minimize(old, new);
        assert_eq!(apply(old, &edits), new);
        let (span, _) = &edits[0];
        assert!(old.is_char_boundary(span.start));
        assert!(old.is_char_boundary(span.end));
    }

    #[test]
    fn suffix_never_overlaps_the_prefix() {
        // "aa" → "a": prefix eats the first byte, the suffix must not
        // also claim it.
        let edits = minimize("aa", "a");
        assert_eq!(apply("aa", &edits), "a");

        let edits = minimize("a", "aa");
        assert_eq!(apply("a", &edits), "aa");
    }

    #[test]
    fn edit_boundaries_never_split_a_crlf_break() {
        // A boundary between `\r` and `\n` is a valid char boundary but not
        // a valid LSP position (characters clamp to the line content end),
        // so a conforming client would misplace the edit. The edit must
        // widen onto whole breaks instead.
        for (old, new) in [
            ("\r\n", "\r,"), // naive common prefix ends mid-break
            ("\r\n", "\n"),  // naive common suffix starts mid-break
            ("\r\r\n", "\n\n"),
            ("a\r\nb", "a\rb"),
        ] {
            let edits = minimize(old, new);
            assert_eq!(apply(old, &edits), new, "{old:?} -> {new:?}");
            let bytes = old.as_bytes();
            for (span, _) in &edits {
                for offset in [span.start, span.end] {
                    assert!(
                        !super::splits_crlf(bytes, offset),
                        "{old:?} -> {new:?} splits a CRLF at {offset}"
                    );
                }
            }
        }
    }

    #[test]
    fn pure_insertions_and_deletions_work_at_the_ends() {
        let edits = minimize("ab", "abc");
        assert_eq!(edits[0].0, Span::new(2, 2));
        assert_eq!(edits[0].1, "c");

        let edits = minimize("abc", "bc");
        assert_eq!(apply("abc", &edits), "bc");

        let edits = minimize("", "x");
        assert_eq!(apply("", &edits), "x");
    }
}
