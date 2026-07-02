//! The error-tolerant CSV parser and its span-based data model.
//!
//! [`parse`] is **total**: any input produces a [`Table`] (plus
//! [`ParseError`]s), never a failure. All spans are byte offsets into the
//! parsed text; the delimiter, quote and line-break bytes are ASCII and can
//! never occur inside a UTF-8 multibyte sequence, so every span boundary is
//! a `char` boundary. See `docs/plan/m1-parser-and-diagnostics.md` for the
//! state machine and recovery rules.

use std::borrow::Cow;

use crate::dialect::Dialect;

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

/// How a cell was written in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quoting {
    /// Bare content, ended by a delimiter or row terminator.
    Unquoted,
    /// Wrapped in double quotes (may contain delimiters and newlines).
    Quoted,
}

/// The row terminator style of a file: the first terminator seen wins and
/// is reused when re-rendering. A lone `\r` counts as the `Lf` family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineTerminator {
    /// `\n`
    Lf,
    /// `\r\n`
    CrLf,
}

impl LineTerminator {
    /// The terminator's text form.
    pub fn as_str(self) -> &'static str {
        match self {
            LineTerminator::Lf => "\n",
            LineTerminator::CrLf => "\r\n",
        }
    }
}

/// A parsing problem, reported with an exact span instead of aborting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// What went wrong.
    pub kind: ParseErrorKind,
    /// The offending bytes (see each kind for the exact policy).
    pub span: Span,
    /// Index into `Table::rows` of the row being parsed.
    pub row: usize,
}

/// The kinds of quoting damage the parser recovers from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// A quoted cell never sees its closing quote; the span is the opening
    /// quote and the rest of the file becomes the cell's content.
    UnclosedQuote,
    /// A `"` inside an unquoted cell; the span is that byte and the quote
    /// is kept as literal content.
    StrayQuote,
    /// Non-space bytes between a closing quote and the next delimiter; the
    /// span is the garbage run (trailing spaces trimmed) and the quoted
    /// value is kept.
    TextAfterClosingQuote,
}

/// A parsed cell.
///
/// `content_span ⊆ span`: the difference is alignment padding (ASCII
/// spaces). For quoted cells the content span *includes* the quotes. An
/// all-padding cell has a zero-width content span at `span.start` (padding
/// counts as trailing, consistent with left-aligned columns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// Full extent, excluding delimiters, including padding.
    pub span: Span,
    /// Padding-trimmed extent.
    pub content_span: Span,
    /// Whether the source wrapped the cell in quotes.
    pub quoting: Quoting,
    /// True when `""` escapes occurred (decoding must allocate).
    pub has_escaped_quotes: bool,
}

impl Cell {
    /// The decoded value: padding trimmed, quotes stripped, `""` unescaped.
    /// Borrows from `text` unless unescaping forces an allocation.
    pub fn value<'t>(&self, text: &'t str) -> Cow<'t, str> {
        let content = self.content_span.slice(text);
        match self.quoting {
            Quoting::Unquoted => Cow::Borrowed(content),
            Quoting::Quoted => {
                // An unclosed cell has no closing quote to strip; both
                // strips are therefore optional.
                let inner = content.strip_prefix('"').unwrap_or(content);
                let inner = inner.strip_suffix('"').unwrap_or(inner);
                if self.has_escaped_quotes {
                    Cow::Owned(inner.replace("\"\"", "\""))
                } else {
                    Cow::Borrowed(inner)
                }
            }
        }
    }
}

/// A parsed row. The span excludes the row terminator but — for rows with
/// multi-line quoted cells — may cover several editor lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// From the first cell's start to the last cell's end.
    pub span: Span,
    /// The row's cells, in order; never empty.
    pub cells: Vec<Cell>,
}

/// The parse result: always produced, never fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// All rows, blank ones included.
    pub rows: Vec<Row>,
    /// Recovered problems, in source order.
    pub errors: Vec<ParseError>,
    /// The dialect the text was parsed under.
    pub dialect: Dialect,
    /// First row terminator seen (`Lf` for files without terminators).
    pub line_terminator: LineTerminator,
    /// Whether the text starts with a UTF-8 byte-order mark (spans never
    /// include it; re-rendering re-emits it).
    pub has_bom: bool,
    /// Whether the text ends with a row terminator (no phantom empty row is
    /// produced for it).
    pub ends_with_newline: bool,
}

/// Parse `text` under `dialect`. Total: never fails, never panics.
pub fn parse(text: &str, dialect: Dialect) -> Table {
    Parser {
        bytes: text.as_bytes(),
        delimiter: dialect.delimiter(),
        pos: 0,
        rows: Vec::new(),
        errors: Vec::new(),
        line_terminator: None,
        ends_with_newline: false,
    }
    .run(dialect)
}

/// How a cell ended, deciding whether another cell follows in the row.
enum CellEnd {
    Delimiter,
    RowEnd,
}

struct Parser<'t> {
    bytes: &'t [u8],
    delimiter: u8,
    pos: usize,
    rows: Vec<Row>,
    errors: Vec<ParseError>,
    line_terminator: Option<LineTerminator>,
    ends_with_newline: bool,
}

impl Parser<'_> {
    fn run(mut self, dialect: Dialect) -> Table {
        while self.pos < self.bytes.len() {
            self.row();
        }
        Table {
            rows: self.rows,
            errors: self.errors,
            dialect,
            line_terminator: self.line_terminator.unwrap_or(LineTerminator::Lf),
            has_bom: false,
            ends_with_newline: self.ends_with_newline,
        }
    }

    fn row(&mut self) {
        let start = self.pos;
        let mut cells = Vec::new();
        loop {
            let (cell, end) = self.cell();
            cells.push(cell);
            match end {
                CellEnd::Delimiter => {
                    self.pos += 1; // consume the delimiter, next cell follows
                }
                CellEnd::RowEnd => break,
            }
        }
        let span = Span::new(start, self.pos);
        let terminated = self.row_terminator();
        self.ends_with_newline = terminated && self.pos == self.bytes.len();
        self.rows.push(Row { span, cells });
    }

    /// Consume the row terminator, if any, recording the file's first one.
    fn row_terminator(&mut self) -> bool {
        match self.bytes.get(self.pos) {
            Some(b'\n') => {
                self.line_terminator.get_or_insert(LineTerminator::Lf);
                self.pos += 1;
                true
            }
            _ => false,
        }
    }

    /// Parse one cell: `CellStart → InUnquoted`, stopping (without
    /// consuming) at the row terminator or EOF, and reporting whether a
    /// delimiter follows.
    fn cell(&mut self) -> (Cell, CellEnd) {
        let span_start = self.pos;
        // CellStart: consume leading padding.
        while self.bytes.get(self.pos) == Some(&b' ') {
            self.pos += 1;
        }
        if self.bytes.get(self.pos) == Some(&b'"') {
            return self.quoted_cell(span_start);
        }
        let content_start = self.pos;
        let mut content_end = self.pos;
        let end = loop {
            match self.bytes.get(self.pos) {
                None | Some(b'\n') => break CellEnd::RowEnd,
                Some(&b) if b == self.delimiter => break CellEnd::Delimiter,
                Some(b' ') => self.pos += 1, // padding, unless content follows
                Some(_) => {
                    self.pos += 1;
                    content_end = self.pos;
                }
            }
        };
        let content_span = if content_end > content_start {
            Span::new(content_start, content_end)
        } else {
            // All padding: zero-width content at the span *start* — the
            // spaces count as trailing padding (left-aligned columns).
            Span::new(span_start, span_start)
        };
        let cell = Cell {
            span: Span::new(span_start, self.pos),
            content_span,
            quoting: Quoting::Unquoted,
            has_escaped_quotes: false,
        };
        (cell, end)
    }

    /// `InQuoted → InAfterQuoted`: the content span includes the quotes;
    /// delimiters and line breaks inside them are content.
    fn quoted_cell(&mut self, span_start: usize) -> (Cell, CellEnd) {
        let content_start = self.pos;
        self.pos += 1; // opening quote
        let mut has_escaped_quotes = false;
        let content_end = loop {
            match self.bytes.get(self.pos) {
                // Unclosed quote: the error report lands in a later cycle.
                None => break self.pos,
                Some(b'"') => {
                    if self.bytes.get(self.pos + 1) == Some(&b'"') {
                        has_escaped_quotes = true;
                        self.pos += 2; // escaped pair is content
                    } else {
                        self.pos += 1; // closing quote
                        break self.pos;
                    }
                }
                Some(_) => self.pos += 1, // incl. delimiters and newlines
            }
        };
        // InAfterQuoted: alignment padding until delimiter / row end.
        let end = loop {
            match self.bytes.get(self.pos) {
                None | Some(b'\n') => break CellEnd::RowEnd,
                Some(&b) if b == self.delimiter => break CellEnd::Delimiter,
                Some(b' ') => self.pos += 1, // tolerated: our own align layout
                Some(_) => self.pos += 1,    // garbage: error lands in a later cycle
            }
        };
        let cell = Cell {
            span: Span::new(span_start, self.pos),
            content_span: Span::new(content_start, content_end),
            quoting: Quoting::Quoted,
            has_escaped_quotes,
        };
        (cell, end)
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

    /// Slices of every cell's span, per row — failures read as text.
    fn cell_slices<'t>(text: &'t str, table: &Table) -> Vec<Vec<&'t str>> {
        table
            .rows
            .iter()
            .map(|row| row.cells.iter().map(|cell| cell.span.slice(text)).collect())
            .collect()
    }

    /// Slices of every cell's content span, per row.
    fn content_slices<'t>(text: &'t str, table: &Table) -> Vec<Vec<&'t str>> {
        table
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| cell.content_span.slice(text))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn padding_is_trimmed_from_content_spans() {
        let text = " a , bb ,c \n";
        let table = parse(text, Dialect::Csv);
        assert_eq!(cell_slices(text, &table), [[" a ", " bb ", "c "]]);
        assert_eq!(content_slices(text, &table), [["a", "bb", "c"]]);
    }

    #[test]
    fn interior_spaces_are_content() {
        let text = "a b,c\n";
        let table = parse(text, Dialect::Csv);
        assert_eq!(content_slices(text, &table), [["a b", "c"]]);
    }

    #[test]
    fn all_padding_cell_has_zero_width_content_at_its_start() {
        let text = "x,  ,y\n";
        let table = parse(text, Dialect::Csv);
        let padded = &table.rows[0].cells[1];
        assert_eq!(padded.span.slice(text), "  ");
        assert!(padded.content_span.is_empty());
        assert_eq!(padded.content_span.start, padded.span.start);
    }

    #[test]
    fn quoted_cells_may_contain_delimiters() {
        let text = "\"a,b\",c\n";
        let table = parse(text, Dialect::Csv);
        assert_eq!(cell_slices(text, &table), [["\"a,b\"", "c"]]);
        let quoted = &table.rows[0].cells[0];
        assert_eq!(quoted.quoting, Quoting::Quoted);
        assert_eq!(quoted.content_span.slice(text), "\"a,b\"");
        assert!(matches!(quoted.value(text), Cow::Borrowed("a,b")));
    }

    #[test]
    fn escaped_quotes_decode_with_allocation() {
        let text = "\"x\"\"y\"\n";
        let table = parse(text, Dialect::Csv);
        let cell = &table.rows[0].cells[0];
        assert!(cell.has_escaped_quotes);
        assert!(matches!(cell.value(text), Cow::Owned(_)));
        assert_eq!(cell.value(text), "x\"y");
    }

    #[test]
    fn padding_around_quoted_cells_is_tolerated() {
        let text = " \"q\" ,z\n";
        let table = parse(text, Dialect::Csv);
        assert!(table.errors.is_empty());
        let cell = &table.rows[0].cells[0];
        assert_eq!(cell.span.slice(text), " \"q\" ");
        assert_eq!(cell.content_span.slice(text), "\"q\"");
        assert_eq!(cell.value(text), "q");
    }

    #[test]
    fn empty_quoted_cell_decodes_to_empty() {
        let text = "\"\",x\n";
        let table = parse(text, Dialect::Csv);
        assert_eq!(table.rows[0].cells[0].value(text), "");
    }

    #[test]
    fn parses_unquoted_cells_with_exact_spans() {
        let text = "a,b,cc\n1,,2\n";
        let table = parse(text, Dialect::Csv);

        assert_eq!(
            cell_slices(text, &table),
            [["a", "b", "cc"], ["1", "", "2"]]
        );
        assert_eq!(table.rows[0].span.slice(text), "a,b,cc");
        assert_eq!(table.rows[1].span.slice(text), "1,,2");
        assert!(table.errors.is_empty());
        assert!(table.ends_with_newline);

        let empty = &table.rows[1].cells[1];
        assert!(empty.span.is_empty());
        assert_eq!(empty.span.start, "a,b,cc\n1,".len());
    }

    #[test]
    fn last_row_without_terminator_is_kept() {
        let table = parse("a", Dialect::Csv);
        assert_eq!(table.rows.len(), 1);
        assert!(!table.ends_with_newline);
    }

    #[test]
    fn empty_text_has_no_rows() {
        let table = parse("", Dialect::Csv);
        assert!(table.rows.is_empty());
        assert!(!table.ends_with_newline);
    }

    #[test]
    fn delimiter_follows_the_dialect() {
        let text = "a;b\tc\n";
        let ssv = parse(text, Dialect::Ssv);
        assert_eq!(cell_slices(text, &ssv), [["a", "b\tc"]]);
        let tsv = parse(text, Dialect::Tsv);
        assert_eq!(cell_slices(text, &tsv), [["a;b", "c"]]);
    }
}
