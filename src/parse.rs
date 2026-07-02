//! The error-tolerant CSV parser and its span-based data model.
//!
//! For M0 this module only defines [`Span`]; the parser itself lands in M1
//! (see `docs/plan/m1-parser-and-diagnostics.md`).

/// A half-open byte range `start..end` into the document text.
///
/// Invariant: `start <= end`, and both offsets lie on `char` boundaries of
/// the text they were produced from (the parser only splits at ASCII bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// First byte of the range.
    pub start: usize,
    /// One past the last byte of the range.
    pub end: usize,
}

impl Span {
    /// Create a span; panics in debug builds when `start > end`.
    pub fn new(start: usize, end: usize) -> Self {
        debug_assert!(start <= end, "inverted span {start}..{end}");
        Span { start, end }
    }

    /// Number of bytes covered.
    pub fn len(self) -> usize {
        self.end - self.start
    }

    /// True when the span covers no bytes.
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// True when `offset` lies within the half-open range.
    pub fn contains(self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }

    /// True when the two spans share at least one byte.
    pub fn overlaps(self, other: Span) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// The text covered by this span.
    pub fn slice(self, text: &str) -> &str {
        &text[self.start..self.end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_returns_the_covered_text() {
        assert_eq!(Span::new(3, 5).slice("id,name"), "na");
    }

    #[test]
    fn len_and_is_empty() {
        assert_eq!(Span::new(3, 5).len(), 2);
        assert!(!Span::new(3, 5).is_empty());
        assert!(Span::new(3, 3).is_empty());
    }

    #[test]
    fn contains_is_half_open() {
        let span = Span::new(3, 5);
        assert!(!span.contains(2));
        assert!(span.contains(3));
        assert!(span.contains(4));
        assert!(!span.contains(5));
    }

    #[test]
    fn overlaps_requires_a_shared_byte() {
        let span = Span::new(3, 5);
        assert!(span.overlaps(Span::new(4, 9)));
        assert!(span.overlaps(Span::new(0, 4)));
        assert!(!span.overlaps(Span::new(5, 9))); // touching is not overlapping
        assert!(!span.overlaps(Span::new(0, 3)));
    }
}
