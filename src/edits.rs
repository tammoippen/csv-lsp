//! Turning a full re-render into a minimal edit.
//!
//! Whole-document replacements work but scroll the viewport and move the
//! cursor in some clients; trimming the identical outer bytes yields one
//! small replacement instead — and `[]` for identical texts, which makes
//! formatting idempotent at the protocol level.

use crate::parse::Span;

/// The smallest single replacement turning `old` into `new`: the common
/// prefix and suffix are trimmed (snapped to `char` boundaries). Empty for
/// identical inputs.
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
    debug_assert!(new.is_char_boundary(new.len() - suffix));

    let span = Span::new(prefix, old.len() - suffix);
    let replacement = new[prefix..new.len() - suffix].to_owned();
    vec![(span, replacement)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference implementation: apply the edits to `old`.
    fn apply(old: &str, edits: &[(Span, String)]) -> String {
        let mut result = old.to_owned();
        for (span, replacement) in edits.iter().rev() {
            result.replace_range(span.start..span.end, replacement);
        }
        result
    }

    #[test]
    fn identical_texts_need_no_edit() {
        assert_eq!(minimize("a,b\n", "a,b\n"), Vec::new());
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
