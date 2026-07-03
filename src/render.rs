//! Rendering a parsed table back to text: the shared engine behind align,
//! compact and (future) dialect transforms.
//!
//! Rows overlapping a parse error are emitted **verbatim** — the renderer
//! never reformats what the parser could not fully understand.

use std::borrow::Cow;
use std::collections::HashSet;

use unicode_width::UnicodeWidthStr;

use crate::dialect::Dialect;
use crate::parse::{LineTerminator, Table};

/// What [`render`] should produce.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Target delimiter (differs from the table's for dialect transforms).
    pub dialect: Dialect,
    /// `Some(column display widths)`: pad cells so delimiters line up.
    /// `None`: compact, no padding.
    pub align: Option<Vec<usize>>,
    /// How cell content is emitted.
    pub quote_policy: QuotePolicy,
    /// Row terminator to emit (mixed-terminator files normalize to it).
    pub line_terminator: LineTerminator,
    /// Re-emit the BOM the file started with.
    pub include_bom: bool,
    /// Reproduce the trailing newline.
    pub final_newline: bool,
}

impl RenderOptions {
    /// Compact rendering preserving the table's dialect, terminator, BOM
    /// and final newline.
    pub fn compact_for(table: &Table) -> Self {
        RenderOptions {
            dialect: table.dialect,
            align: None,
            quote_policy: QuotePolicy::Preserve,
            line_terminator: table.line_terminator,
            include_bom: table.has_bom,
            final_newline: table.ends_with_newline,
        }
    }

    /// Aligned rendering with the given column widths, everything else
    /// preserved.
    pub fn aligned_for(table: &Table, widths: Vec<usize>) -> Self {
        RenderOptions {
            align: Some(widths),
            ..Self::compact_for(table)
        }
    }
}

/// How cell content leaves the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotePolicy {
    /// Emit each cell's content bytes verbatim — maximally lossless; align
    /// and compact are pure whitespace transforms under this policy (with
    /// one exception under every policy: a first cell whose content starts
    /// with U+FEFF is quoted so it cannot be re-read as a file BOM).
    Preserve,
    /// Re-encode each *decoded* value, quoting only where the target
    /// dialect requires it (normalizes quoting).
    Required,
    /// Dialect conversion: quoted cells stay verbatim (already protected);
    /// unquoted cells gain quotes only when their content contains the
    /// *target* delimiter. Clean unquoted cells can never contain quotes or
    /// newlines, so this is complete for files without parse errors.
    PreserveOrRequired,
}

/// Render the table back to text under `opts`.
pub fn render(text: &str, table: &Table, opts: &RenderOptions) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    if opts.include_bom {
        out.push('\u{feff}');
    }
    let terminator = opts.line_terminator.as_str();
    let delimiter = opts.dialect.delimiter() as char;
    let error_rows: HashSet<usize> = table.errors.iter().map(|error| error.row).collect();

    for (index, row) in table.rows.iter().enumerate() {
        if index > 0 {
            out.push_str(terminator);
        }
        if error_rows.contains(&index) {
            // Never reformat what the parser could not fully understand.
            out.push_str(row.span.slice(text));
            continue;
        }
        if row.is_blank() {
            continue; // an empty line
        }
        for (column, cell) in row.cells.iter().enumerate() {
            if column > 0 {
                out.push(delimiter);
            }
            let mut content: Cow<'_, str> = match opts.quote_policy {
                QuotePolicy::Preserve => Cow::Borrowed(cell.content_span.slice(text)),
                QuotePolicy::Required => {
                    Cow::Owned(encode_cell(&cell.value(text), opts.dialect, false))
                }
                QuotePolicy::PreserveOrRequired => {
                    let content = cell.content_span.slice(text);
                    if cell.quoting == crate::parse::Quoting::Unquoted
                        && content.contains(delimiter)
                    {
                        Cow::Owned(encode_cell(content, opts.dialect, false))
                    } else {
                        Cow::Borrowed(content)
                    }
                }
            };
            // A first cell whose content would put U+FEFF at byte 0 of the
            // output gets re-read as a file-level BOM by any parser — the
            // value would silently lose the character. Quotes keep it data.
            if out.is_empty() && content.starts_with('\u{feff}') {
                content = Cow::Owned(encode_cell(&cell.value(text), opts.dialect, true));
            }
            out.push_str(&content);
            // Pad only when another cell follows: last cells stay unpadded,
            // so no row ever gains trailing whitespace.
            if let Some(widths) = &opts.align
                && column + 1 < row.cells.len()
            {
                let width = widths.get(column).copied().unwrap_or(0);
                let padding = width.saturating_sub(content.width());
                for _ in 0..padding {
                    out.push(' ');
                }
            }
        }
    }
    if opts.final_newline && !table.rows.is_empty() {
        out.push_str(terminator);
    }
    out
}

/// RFC 4180-encode a decoded value for `dialect`: quote when it contains
/// the delimiter, a quote or a line break (or when forced), doubling any
/// embedded quotes.
pub fn encode_cell(value: &str, dialect: Dialect, force_quote: bool) -> String {
    let delimiter = dialect.delimiter() as char;
    let needs_quotes = force_quote
        || value.contains(delimiter)
        || value.contains('"')
        || value.contains('\n')
        || value.contains('\r');
    if needs_quotes {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

/// Display width of every column (Unicode width, UAX #11), measured over
/// the content spans of clean rows. Blank rows and rows with parse errors
/// contribute nothing (they are passed through verbatim when rendering).
pub fn column_widths(text: &str, table: &Table) -> Vec<usize> {
    let error_rows: HashSet<usize> = table.errors.iter().map(|error| error.row).collect();
    let mut widths = Vec::new();
    for (index, row) in table.rows.iter().enumerate() {
        if row.is_blank() || error_rows.contains(&index) {
            continue;
        }
        for (column, cell) in row.cells.iter().enumerate() {
            let content = cell.content_span.slice(text);
            let mut width = content.width();
            // Mirror the renderer: a first cell whose content starts with
            // U+FEFF gains quotes (it would otherwise be re-read as a file
            // BOM), so it measures two display cells wider.
            if index == 0 && column == 0 && !table.has_bom && content.starts_with('\u{feff}') {
                width += 2;
            }
            if column == widths.len() {
                widths.push(width);
            } else {
                widths[column] = widths[column].max(width);
            }
        }
    }
    widths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::Dialect;
    use crate::parse::parse;

    fn widths(text: &str) -> Vec<usize> {
        column_widths(text, &parse(text, Dialect::Csv))
    }

    #[test]
    fn widths_measure_display_cells_not_bytes() {
        // héllo: 5 display cells despite 6 bytes; 名前: 4 cells from 2 chars.
        assert_eq!(widths("id,name\n1,héllo\n999,名前\n"), [3, 5]);
    }

    #[test]
    fn quoted_cells_measure_with_their_quotes() {
        assert_eq!(widths("id,x\n1,\"a,b\"\n"), [2, 5]);
    }

    #[test]
    fn blank_and_error_rows_contribute_nothing() {
        assert_eq!(widths("a,b\n\n\"broken bar,baz\n"), [1, 1]);
    }

    #[test]
    fn ragged_long_rows_extend_the_column_list() {
        assert_eq!(widths("a,b\n1,22,333\n"), [1, 2, 3]);
    }

    #[test]
    fn empty_documents_have_no_columns() {
        assert_eq!(widths(""), Vec::<usize>::new());
        assert_eq!(widths("\n\n"), Vec::<usize>::new());
    }

    fn compact(text: &str) -> String {
        let table = parse(text, Dialect::Csv);
        render(text, &table, &RenderOptions::compact_for(&table))
    }

    #[test]
    fn compact_strips_padding_around_all_cell_kinds() {
        assert_eq!(compact(" a , \"q\" ,c \n1,2,3\n"), "a,\"q\",c\n1,2,3\n");
    }

    #[test]
    fn compact_keeps_blank_lines_as_separators() {
        assert_eq!(compact("a,b\n\n1,2\n"), "a,b\n\n1,2\n");
    }

    #[test]
    fn compact_passes_error_rows_through_verbatim() {
        assert_eq!(compact("a,b\n\"x\" z , w\n"), "a,b\n\"x\" z , w\n");
    }

    #[test]
    fn compact_preserves_bom_and_crlf() {
        assert_eq!(compact("\u{feff}a , b\r\n"), "\u{feff}a,b\r\n");
    }

    #[test]
    fn compact_preserves_a_missing_final_newline() {
        assert_eq!(compact("a , b"), "a,b");
        assert_eq!(compact(""), "");
    }

    #[test]
    fn a_cell_value_starting_with_a_bom_is_quoted_to_stay_data() {
        // The ZWNBSP is *content* here (the file does not start with it).
        // Stripping the padding would put it at byte 0, where every parser
        // reads it as a file-level BOM — quoting keeps it in the value.
        assert_eq!(compact(" \u{feff}x,y\n"), "\"\u{feff}x\",y\n");
        // With a real file BOM ahead of it, the content BOM is safe.
        assert_eq!(compact("\u{feff} \u{feff}x,y\n"), "\u{feff}\u{feff}x,y\n");
    }

    #[test]
    fn align_widths_account_for_the_bom_quoting() {
        // The first cell gains quotes when aligned, so the column must be
        // measured at its quoted width or align would not be idempotent.
        assert_eq!(align(" \u{feff}a,x\nbb,y\n"), "\"\u{feff}a\",x\nbb ,y\n");
    }

    #[test]
    fn quoted_interiors_are_never_touched() {
        // Padding *inside* quotes is content, not layout.
        assert_eq!(compact("\" a , b \",c\n"), "\" a , b \",c\n");
    }

    fn align(text: &str) -> String {
        let table = parse(text, Dialect::Csv);
        let widths = column_widths(text, &table);
        render(text, &table, &RenderOptions::aligned_for(&table, widths))
    }

    #[test]
    fn align_pads_delimiters_into_columns() {
        assert_eq!(
            align("id,name,qty\n1,\"a,b\",3\n20,x,400\n"),
            "id,name ,qty\n1 ,\"a,b\",3\n20,x    ,400\n"
        );
    }

    #[test]
    fn align_pads_by_display_width() {
        assert_eq!(align("a,héllo,x\nbb,名前,y\n"), "a ,héllo,x\nbb,名前 ,y\n");
    }

    #[test]
    fn align_never_pads_a_rows_last_cell() {
        // Short row: its last cell has cells following in *other* rows but
        // none in its own — no trailing spaces anywhere.
        assert_eq!(align("aaa,b,c\n1\n"), "aaa,b,c\n1\n");
    }

    #[test]
    fn align_skips_blank_and_error_rows() {
        assert_eq!(
            align("aaa,b\n\n1,2\n\"x y,z\n"),
            "aaa,b\n\n1  ,2\n\"x y,z\n"
        );
    }

    /// Inputs exercising every renderer path, used for the round-trip
    /// properties below.
    const WILD: &[&str] = &[
        "",
        "\n",
        "a",
        "a,b\r\n1,2\r\n",
        " a , b \n1,\"q\" ,2\n",
        "\u{feff}x;y\n",
        "id,name,qty\n1,\"a,b\",3\n20,x,400\n",
        "a,héllo\nbb,名前\n",
        "h1,h2\n\n\"multi\nline\",2\n",
        "broken \"row,1\nclean,2\n",
        "\"unclosed,to eof",
        " , ,\n,,\n",
    ];

    #[test]
    fn align_is_idempotent() {
        for text in WILD {
            let once = align(text);
            assert_eq!(align(&once), once, "align not idempotent for {text:?}");
        }
    }

    #[test]
    fn compact_undoes_align() {
        for text in WILD {
            assert_eq!(
                compact(&align(text)),
                compact(text),
                "compact(align) != compact for {text:?}"
            );
        }
    }

    #[test]
    fn compact_is_idempotent() {
        for text in WILD {
            let once = compact(text);
            assert_eq!(compact(&once), once, "compact not idempotent for {text:?}");
        }
    }

    fn convert(text: &str, from: Dialect, to: Dialect) -> String {
        let table = parse(text, from);
        let opts = RenderOptions {
            dialect: to,
            quote_policy: QuotePolicy::PreserveOrRequired,
            ..RenderOptions::compact_for(&table)
        };
        render(text, &table, &opts)
    }

    #[test]
    fn conversion_keeps_quoted_cells_verbatim() {
        assert_eq!(
            convert("\"a,b\",x\n", Dialect::Csv, Dialect::Tsv),
            "\"a,b\"\tx\n"
        );
    }

    #[test]
    fn conversion_quotes_cells_containing_the_target_delimiter() {
        // A tab is plain content under CSV but must be protected in TSV.
        assert_eq!(
            convert("a\tb,x\n", Dialect::Csv, Dialect::Tsv),
            "\"a\tb\"\tx\n"
        );
        // German decimals: content commas need quoting in CSV.
        assert_eq!(
            convert("artikel;preis\nbolzen;1,50\n", Dialect::Ssv, Dialect::Csv),
            "artikel,preis\nbolzen,\"1,50\"\n"
        );
    }

    #[test]
    fn conversion_leaves_plain_cells_byte_identical() {
        assert_eq!(
            convert("a,b\n1,2\n", Dialect::Csv, Dialect::Ssv),
            "a;b\n1;2\n"
        );
    }

    #[test]
    fn encode_cell_quotes_only_when_needed() {
        assert_eq!(encode_cell("plain", Dialect::Csv, false), "plain");
        assert_eq!(encode_cell("a,b", Dialect::Csv, false), "\"a,b\"");
        assert_eq!(encode_cell("a,b", Dialect::Tsv, false), "a,b");
        assert_eq!(encode_cell("a\tb", Dialect::Tsv, false), "\"a\tb\"");
        assert_eq!(
            encode_cell("say \"hi\"", Dialect::Csv, false),
            "\"say \"\"hi\"\"\""
        );
        assert_eq!(
            encode_cell("two\nlines", Dialect::Csv, false),
            "\"two\nlines\""
        );
        assert_eq!(encode_cell("forced", Dialect::Csv, true), "\"forced\"");
    }
}
