//! The error-tolerant CSV parser and its span-based data model.
//!
//! [`parse`] is **total**: any input produces a [`Table`] (plus
//! [`ParseError`]s), never a failure. All spans are byte offsets into the
//! parsed text; the delimiter, quote and line-break bytes are ASCII and can
//! never occur inside a UTF-8 multibyte sequence, so every span boundary is
//! a `char` boundary. See `docs/parser.html` for the state machine and
//! recovery rules.

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

impl Row {
    /// A blank row (empty line or spaces only): exactly one unquoted cell
    /// with empty content. Blank rows are separators — no diagnostics, no
    /// part in column-count checks, rendered as empty lines.
    pub fn is_blank(&self) -> bool {
        self.cells.len() == 1
            && self.cells[0].quoting == Quoting::Unquoted
            && self.cells[0].content_span.is_empty()
    }
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

impl Table {
    /// The header: the first non-blank row. Its cell count is the column
    /// contract the rest of the file is checked against.
    pub fn header(&self) -> Option<&Row> {
        self.rows.iter().find(|row| !row.is_blank())
    }

    /// Index of the header row within `rows`.
    pub fn header_index(&self) -> Option<usize> {
        self.rows.iter().position(|row| !row.is_blank())
    }

    /// The expected column count (the header's cell count).
    pub fn expected_columns(&self) -> Option<usize> {
        self.header().map(|row| row.cells.len())
    }

    /// The row under `offset`, with an **inclusive** end: a cursor sitting
    /// at the end of a row (on its terminator) still belongs to it.
    pub fn row_at(&self, offset: usize) -> Option<usize> {
        let after = self.rows.partition_point(|row| row.span.start <= offset);
        let index = after.checked_sub(1)?;
        (offset <= self.rows[index].span.end).then_some(index)
    }

    /// The `(row, column)` under `offset`. A cursor on a delimiter resolves
    /// to the cell left of it; a cursor at the row end to the last cell.
    pub fn cell_at(&self, offset: usize) -> Option<(usize, usize)> {
        let row_index = self.row_at(offset)?;
        let cells = &self.rows[row_index].cells;
        let after = cells.partition_point(|cell| cell.span.start <= offset);
        let column = after.checked_sub(1)?;
        (offset <= cells[column].span.end).then_some((row_index, column))
    }
}

/// Parse `text` under `dialect`. Total: never fails, never panics.
pub fn parse(text: &str, dialect: Dialect) -> Table {
    const BOM: &str = "\u{feff}";
    let has_bom = text.starts_with(BOM);
    Parser {
        bytes: text.as_bytes(),
        delimiter: dialect.delimiter(),
        pos: if has_bom { BOM.len() } else { 0 },
        rows: Vec::new(),
        errors: Vec::new(),
        line_terminator: None,
        ends_with_newline: false,
    }
    .run(dialect, has_bom)
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
    fn run(mut self, dialect: Dialect, has_bom: bool) -> Table {
        while self.pos < self.bytes.len() {
            self.row();
        }
        Table {
            rows: self.rows,
            errors: self.errors,
            dialect,
            line_terminator: self.line_terminator.unwrap_or(LineTerminator::Lf),
            has_bom,
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

    /// Record a recovered error against the row currently being parsed
    /// (its index is what `rows.len()` will be once it is pushed).
    fn error(&mut self, kind: ParseErrorKind, start: usize, end: usize) {
        self.errors.push(ParseError {
            kind,
            span: Span::new(start, end),
            row: self.rows.len(),
        });
    }

    /// Consume the row terminator, if any, recording the file's first one.
    fn row_terminator(&mut self) -> bool {
        match self.bytes.get(self.pos) {
            Some(b'\n') => {
                self.line_terminator.get_or_insert(LineTerminator::Lf);
                self.pos += 1;
                true
            }
            Some(b'\r') => {
                if self.bytes.get(self.pos + 1) == Some(&b'\n') {
                    self.line_terminator.get_or_insert(LineTerminator::CrLf);
                    self.pos += 2;
                } else {
                    // Lone \r is a line break per the LSP spec; Lf family.
                    self.line_terminator.get_or_insert(LineTerminator::Lf);
                    self.pos += 1;
                }
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
                None | Some(b'\n' | b'\r') => break CellEnd::RowEnd,
                Some(&b) if b == self.delimiter => break CellEnd::Delimiter,
                Some(b' ') => self.pos += 1, // padding, unless content follows
                Some(b'"') => {
                    // Stray quote: report it, keep it as literal content.
                    self.error(ParseErrorKind::StrayQuote, self.pos, self.pos + 1);
                    self.pos += 1;
                    content_end = self.pos;
                }
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
                None => {
                    // Unclosed: report the opening quote, the rest of the
                    // file has become this cell's content.
                    self.error(
                        ParseErrorKind::UnclosedQuote,
                        content_start,
                        content_start + 1,
                    );
                    break self.pos;
                }
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
        let mut garbage: Option<Span> = None;
        let end = loop {
            match self.bytes.get(self.pos) {
                None | Some(b'\n' | b'\r') => break CellEnd::RowEnd,
                Some(&b) if b == self.delimiter => break CellEnd::Delimiter,
                Some(b' ') => self.pos += 1, // tolerated: our own align layout
                Some(_) => {
                    // Garbage after the closing quote: skip it, growing one
                    // error span (trailing spaces stay excluded).
                    let start = garbage.map_or(self.pos, |span| span.start);
                    self.pos += 1;
                    garbage = Some(Span::new(start, self.pos));
                }
            }
        };
        if let Some(span) = garbage {
            self.error(ParseErrorKind::TextAfterClosingQuote, span.start, span.end);
        }
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
    fn bom_is_recorded_and_excluded_from_spans() {
        let text = "\u{feff}a,b\n";
        let table = parse(text, Dialect::Csv);
        assert!(table.has_bom);
        assert_eq!(table.rows[0].cells[0].span.start, 3);
        assert_eq!(cell_slices(text, &table), [["a", "b"]]);
    }

    #[test]
    fn crlf_rows_record_the_terminator_and_stay_out_of_spans() {
        let text = "a,b\r\n1,2\r\n";
        let table = parse(text, Dialect::Csv);
        assert_eq!(table.line_terminator, LineTerminator::CrLf);
        assert_eq!(table.rows[0].span.slice(text), "a,b");
        assert_eq!(cell_slices(text, &table), [["a", "b"], ["1", "2"]]);
        assert!(table.ends_with_newline);
    }

    #[test]
    fn lone_cr_ends_a_row_as_lf_family() {
        let text = "a\rb\n";
        let table = parse(text, Dialect::Csv);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.line_terminator, LineTerminator::Lf);
    }

    #[test]
    fn quoted_cells_span_multiple_lines() {
        let text = "\"x\ny\",2\r\nnext\r\n";
        let table = parse(text, Dialect::Csv);
        assert_eq!(table.rows.len(), 2);
        assert!(table.errors.is_empty());
        let cell = &table.rows[0].cells[0];
        assert_eq!(cell.value(text), "x\ny");
        // The row span covers two editor lines; only the file's row
        // terminator (\r\n) is excluded.
        assert_eq!(table.rows[0].span.slice(text), "\"x\ny\",2");
        assert_eq!(table.line_terminator, LineTerminator::CrLf);
    }

    #[test]
    fn unclosed_quote_swallows_the_rest_and_reports_the_opening_quote() {
        let text = "a,\"bc\nd";
        let table = parse(text, Dialect::Csv);

        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].cells.len(), 2);
        assert_eq!(table.rows[0].cells[1].value(text), "bc\nd");
        // That trailing \n is cell content, not a row terminator.
        assert!(!table.ends_with_newline);

        assert_eq!(table.errors.len(), 1);
        let error = &table.errors[0];
        assert_eq!(error.kind, ParseErrorKind::UnclosedQuote);
        assert_eq!(error.span.slice(text), "\"");
        assert_eq!(error.span.start, 2);
        assert_eq!(error.row, 0);
    }

    #[test]
    fn stray_quote_stays_literal_and_is_reported() {
        let text = "5\" bolt,x\n";
        let table = parse(text, Dialect::Csv);

        assert_eq!(table.rows[0].cells[0].value(text), "5\" bolt");
        assert_eq!(table.errors.len(), 1);
        let error = &table.errors[0];
        assert_eq!(error.kind, ParseErrorKind::StrayQuote);
        assert_eq!(error.span.start, 1);
        assert_eq!(error.span.slice(text), "\"");
    }

    #[test]
    fn text_after_closing_quote_is_skipped_and_reported() {
        let text = "\"x\" y,z\n";
        let table = parse(text, Dialect::Csv);

        let cell = &table.rows[0].cells[0];
        assert_eq!(cell.value(text), "x");
        assert_eq!(cell.span.slice(text), "\"x\" y");
        assert_eq!(table.rows[0].cells[1].value(text), "z");

        assert_eq!(table.errors.len(), 1);
        let error = &table.errors[0];
        assert_eq!(error.kind, ParseErrorKind::TextAfterClosingQuote);
        assert_eq!(error.span.slice(text), "y");
        assert_eq!(error.row, 0);
    }

    #[test]
    fn blank_rows_are_separators_and_the_header_skips_them() {
        let text = "\na,b\n   \n1,2\n";
        let table = parse(text, Dialect::Csv);

        assert!(table.rows[0].is_blank());
        assert!(table.rows[2].is_blank()); // spaces only
        assert!(!table.rows[1].is_blank());

        assert_eq!(table.header_index(), Some(1));
        assert_eq!(table.header().unwrap().span.slice(text), "a,b");
        assert_eq!(table.expected_columns(), Some(2));
    }

    #[test]
    fn a_file_of_blank_rows_has_no_header() {
        let table = parse("\n\n", Dialect::Csv);
        assert_eq!(table.header_index(), None);
        assert_eq!(table.expected_columns(), None);
    }

    #[test]
    fn cell_at_resolves_cursor_positions() {
        let text = "ab,cd\nef,gh\n";
        let table = parse(text, Dialect::Csv);

        // Mid-cell.
        assert_eq!(table.cell_at(1), Some((0, 0)));
        assert_eq!(table.cell_at(4), Some((0, 1)));
        // On the delimiter (offset 2): the cell left of it.
        assert_eq!(table.cell_at(2), Some((0, 0)));
        // At the row end (on the terminator): the row's last cell.
        assert_eq!(table.cell_at(5), Some((0, 1)));
        // Start of the second row.
        assert_eq!(table.cell_at(6), Some((1, 0)));
        // Past the last row's inclusive end.
        assert_eq!(table.cell_at(12), None);
        assert_eq!(table.row_at(99), None);
    }

    #[test]
    fn cell_at_works_inside_multi_line_quoted_cells() {
        let text = "\"a\nb\",c\n";
        let table = parse(text, Dialect::Csv);
        // Offset 3 is the `b` on the second editor line — still row 0.
        assert_eq!(table.cell_at(3), Some((0, 0)));
        assert_eq!(table.cell_at(6), Some((0, 1)));
    }

    /// Malformed and adversarial snippets: the parser must stay total.
    const CORPUS: &[&str] = &[
        "",
        "\n",
        "\r",
        "\r\n",
        "\r\r\n",
        ",",
        ",,,",
        "\"",
        "\"\"",
        "\"\"\"",
        "\"\"\"\"",
        "a\"b\"c",
        "\"a\"b\"c\"",
        "\",\n\r",
        "\"\r\r\n",
        "\u{feff}",
        "\u{feff}\"",
        "  \"x\"  garbage  ,y",
        "é😀,\"名\n前\",ü\"",
        "\"😀",
        "a,b,c,d,e,f,g,h,i,j\n1\n",
        "x\0y,\0",
        " , , \n,,\n",
        "\"a\"\"",
        "quote\"in\"the\"middle\n\"and\", \"more\" x\n",
    ];

    #[test]
    fn the_parser_is_total_over_the_corpus() {
        for text in CORPUS {
            for dialect in Dialect::ALL {
                let table = parse(text, dialect);
                for row in &table.rows {
                    assert_char_boundaries(text, row.span);
                    assert!(!row.cells.is_empty());
                    for cell in &row.cells {
                        assert_char_boundaries(text, cell.span);
                        assert_char_boundaries(text, cell.content_span);
                        assert!(row.span.start <= cell.span.start);
                        assert!(cell.span.end <= row.span.end);
                        assert!(cell.span.start <= cell.content_span.start);
                        assert!(cell.content_span.end <= cell.span.end);
                        let _ = cell.value(text); // decoding must not panic
                    }
                }
                for error in &table.errors {
                    assert_char_boundaries(text, error.span);
                    assert!(error.row < table.rows.len(), "error row out of range");
                }
            }
        }
    }

    fn assert_char_boundaries(text: &str, span: Span) {
        assert!(span.start <= span.end, "inverted span in {text:?}");
        assert!(span.end <= text.len(), "span out of bounds in {text:?}");
        assert!(text.is_char_boundary(span.start), "bad start in {text:?}");
        assert!(text.is_char_boundary(span.end), "bad end in {text:?}");
        let _ = span.slice(text); // must not panic
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
        let psv = parse("a|b;c\n", Dialect::Psv);
        assert_eq!(cell_slices("a|b;c\n", &psv), [["a", "b;c"]]);
    }
}
