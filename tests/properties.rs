//! Property-based invariant suite (ADR 0004) — the Hypothesis workflow in
//! Rust, powered by `proptest`.
//!
//! Strategies assemble documents from the parser's *interesting* bytes
//! (delimiters, quotes, `""` escapes, all three terminators, padding, BOM,
//! multibyte text) plus plain Unicode noise, and assert the documented
//! invariants from `docs/architecture.md` over thousands of generated
//! inputs: the parser is total and span-sound, renders round-trip and
//! preserve values, edits apply, positions convert exactly in every
//! encoding, and feature edits honor the LSP contract. Counterexamples
//! shrink to minimal reproducers and persist as seeds in
//! `tests/*.proptest-regressions` (commit those files). Deeper runs:
//! `PROPTEST_CASES=10000 cargo test`.

use csv_lsp::dialect::Dialect;
use csv_lsp::document::Document;
use csv_lsp::edits::{apply, minimize};
use csv_lsp::features::{ActionContext, Registry, columns};
use csv_lsp::parse::{ParseErrorKind, Span, Table, parse};
use csv_lsp::position::{LineIndex, PositionEncoding};
use csv_lsp::render::{QuotePolicy, RenderOptions, column_widths, render};
use lsp_types::CodeActionKind;
use proptest::prelude::*;

static ENCODINGS: [PositionEncoding; 3] = [
    PositionEncoding::Utf8,
    PositionEncoding::Utf16,
    PositionEncoding::Utf32,
];

static DIALECTS: [Dialect; 4] = Dialect::ALL;

/// Weighted-by-repetition vocabulary: uniform random strings almost never
/// contain a quote or a delimiter, so fragment assembly is what reaches the
/// deep parser states (quoting, escapes, recovery).
static FRAGMENTS: &[&str] = &[
    ",", ",", ",", ";", ";", "\t", "\t", "|", "|", // delimiters of all dialects
    "\"", "\"", "\"", "\"\"", // quotes and escaped quotes
    "\n", "\n", "\r\n", "\r", // all three terminators
    " ", " ", "\u{feff}", // padding and the BOM (also mid-text)
    "a", "a", "b", "x9", "é", "😀", "名", "\0", // content, multibyte included
];

fn csv_shaped() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(FRAGMENTS), 0..64).prop_map(|parts| parts.concat())
}

/// Mostly CSV-shaped, sometimes arbitrary Unicode noise.
fn wild_text() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => csv_shaped(),
        1 => any::<String>(),
    ]
}

fn dialect() -> impl Strategy<Value = Dialect> {
    prop::sample::select(&DIALECTS[..])
}

/// A document that parses without errors under the paired dialect. Every
/// parse error requires a quote byte, so stripping the quotes repairs any
/// broken input constructively — no rejection budget, any case count.
fn clean_text_and_dialect() -> impl Strategy<Value = (String, Dialect)> {
    (wild_text(), dialect()).prop_map(|(text, from)| {
        if parse(&text, from).errors.is_empty() {
            (text, from)
        } else {
            (text.replace('"', ""), from)
        }
    })
}

fn encoding() -> impl Strategy<Value = PositionEncoding> {
    prop::sample::select(&ENCODINGS[..])
}

/// A `char` boundary that LSP positions cannot express: between the `\r`
/// and `\n` of a CRLF break (characters clamp to the line's content end).
fn between_crlf(text: &str, offset: usize) -> bool {
    offset > 0
        && text.as_bytes().get(offset - 1) == Some(&b'\r')
        && text.as_bytes().get(offset) == Some(&b'\n')
}

fn assert_span_sound(text: &str, span: Span) {
    assert!(span.start <= span.end, "inverted span {span:?} in {text:?}");
    assert!(
        span.end <= text.len(),
        "span {span:?} out of bounds in {text:?}"
    );
    assert!(
        text.is_char_boundary(span.start) && text.is_char_boundary(span.end),
        "span {span:?} splits a char in {text:?}"
    );
}

/// Both span ends survive the trip to an LSP position and back in every
/// encoding — what a conforming client will actually address.
fn assert_span_addressable(text: &str, index: &LineIndex, span: Span) {
    for enc in ENCODINGS {
        for offset in [span.start, span.end] {
            let position = index.position(text, offset, enc);
            assert_eq!(
                index.offset(text, position, enc),
                offset,
                "offset {offset} of span {span:?} in {text:?} is not \
                 client-addressable under {enc:?} (became {position:?})"
            );
        }
    }
}

/// Every structural invariant documented in `docs/architecture.md`.
fn assert_table_sound(text: &str, table: &Table) {
    let bom_len = if table.has_bom { "\u{feff}".len() } else { 0 };
    if let Some(first) = table.rows.first() {
        assert_eq!(
            first.span.start, bom_len,
            "first row after the BOM in {text:?}"
        );
    } else {
        assert_eq!(text.len(), bom_len, "content must produce rows in {text:?}");
        assert!(!table.ends_with_newline);
    }
    // Rows tile the text: exactly one terminator between adjacent rows and
    // after the last row iff `ends_with_newline`.
    for pair in table.rows.windows(2) {
        assert_one_terminator(text, Span::new(pair[0].span.end, pair[1].span.start));
    }
    if let Some(last) = table.rows.last() {
        if table.ends_with_newline {
            assert_one_terminator(text, Span::new(last.span.end, text.len()));
        } else {
            assert_eq!(
                last.span.end,
                text.len(),
                "unterminated last row ends at EOF"
            );
        }
    }

    let error_rows: std::collections::HashSet<usize> =
        table.errors.iter().map(|error| error.row).collect();
    for (index, row) in table.rows.iter().enumerate() {
        assert_span_sound(text, row.span);
        assert!(!row.cells.is_empty(), "row without cells in {text:?}");
        // Cells tile the row with exactly one delimiter between neighbors.
        assert_eq!(row.cells[0].span.start, row.span.start);
        assert_eq!(row.cells.last().unwrap().span.end, row.span.end);
        for pair in row.cells.windows(2) {
            assert_eq!(
                pair[0].span.end + 1,
                pair[1].span.start,
                "adjacent cells must straddle one delimiter in {text:?}"
            );
            assert_eq!(text.as_bytes()[pair[0].span.end], table.dialect.delimiter());
        }
        for cell in &row.cells {
            assert_span_sound(text, cell.span);
            assert_span_sound(text, cell.content_span);
            assert!(
                cell.span.start <= cell.content_span.start
                    && cell.content_span.end <= cell.span.end,
                "content span escapes its cell in {text:?}"
            );
            // Leading padding is always spaces; the trailing range holds
            // the garbage bytes of a text-after-quote error, so the
            // spaces-only guarantee applies to clean rows (the only rows
            // the renderer ever reformats).
            let leading = &text[cell.span.start..cell.content_span.start];
            assert!(
                leading.bytes().all(|b| b == b' '),
                "leading padding must be ASCII spaces in {text:?}"
            );
            let trailing = &text[cell.content_span.end..cell.span.end];
            assert!(
                error_rows.contains(&index) || trailing.bytes().all(|b| b == b' '),
                "trailing padding of clean cells must be ASCII spaces in {text:?}"
            );
            let _ = cell.value(text); // decoding must never panic
        }
        // Blank rows are exactly the all-space rows.
        assert_eq!(
            row.is_blank(),
            row.span.slice(text).bytes().all(|b| b == b' '),
            "blankness misclassified for {:?} in {text:?}",
            row.span.slice(text)
        );
    }

    for error in &table.errors {
        assert_span_sound(text, error.span);
        assert!(
            error.row < table.rows.len(),
            "error row out of range in {text:?}"
        );
        let row = &table.rows[error.row];
        assert!(
            row.span.start <= error.span.start && error.span.end <= row.span.end,
            "error span escapes its row in {text:?}"
        );
    }
}

fn assert_one_terminator(text: &str, gap: Span) {
    assert_span_sound(text, gap);
    let gap = gap.slice(text);
    assert!(
        matches!(gap, "\n" | "\r\n" | "\r"),
        "rows must be separated by exactly one terminator, got {gap:?} in {text:?}"
    );
}

/// The observable content of a parse: per row its blankness and decoded
/// cell values, plus the error kinds and the rows they sit on.
type Shape = (Vec<(bool, Vec<String>)>, Vec<(ParseErrorKind, usize)>);

fn shape(text: &str, table: &Table) -> Shape {
    (
        table
            .rows
            .iter()
            .map(|row| {
                let values = row
                    .cells
                    .iter()
                    .map(|cell| cell.value(text).into_owned())
                    .collect();
                (row.is_blank(), values)
            })
            .collect(),
        table
            .errors
            .iter()
            .map(|error| (error.kind, error.row))
            .collect(),
    )
}

/// What a padding-stripping render can preserve of `shape`: a *final*
/// blank row without a trailing terminator loses its text form entirely
/// once its spaces are stripped (an empty unterminated last line is
/// indistinguishable from a trailing newline), so it drops out of the
/// expectation. Every other row survives one-to-one.
fn render_expectation(text: &str, table: &Table) -> Shape {
    let (mut rows, errors) = shape(text, table);
    if !table.ends_with_newline
        && let Some((blank, _)) = rows.last()
        && *blank
    {
        rows.pop();
    }
    (rows, errors)
}

fn compact_text(text: &str, dialect: Dialect) -> String {
    let table = parse(text, dialect);
    render(text, &table, &RenderOptions::compact_for(&table))
}

fn align_text(text: &str, dialect: Dialect) -> String {
    let table = parse(text, dialect);
    let widths = column_widths(text, &table);
    render(text, &table, &RenderOptions::aligned_for(&table, widths))
}

/// A document whose dialect is pinned via the language id.
fn doc_for(dialect: Dialect, text: String) -> Document {
    let language_id = dialect.name().to_ascii_lowercase();
    Document::new("file:///t/data".parse().unwrap(), &language_id, 1, text)
}

proptest! {
    // Default persistence probes for lib.rs/main.rs and silently gives up
    // inside integration-test targets; `WithSource` writes the regression
    // seeds next to this file, where they can be committed.
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(
            proptest::test_runner::FileFailurePersistence::WithSource("proptest-regressions"),
        )),
        ..ProptestConfig::default()
    })]

    #[test]
    fn the_parser_is_total_and_structurally_sound(
        text in wild_text(),
        dialect in dialect(),
    ) {
        let table = parse(&text, dialect);
        assert_table_sound(&text, &table);

        // Cursor resolution is total for any offset, in bounds or not.
        for offset in 0..=text.len() + 1 {
            if let Some((row, column)) = table.cell_at(offset) {
                prop_assert_eq!(table.row_at(offset), Some(row));
                let span = table.rows[row].cells[column].span;
                prop_assert!(span.start <= offset && offset <= span.end);
            }
        }
    }

    #[test]
    fn align_and_compact_round_trip(text in wild_text(), dialect in dialect()) {
        let aligned = align_text(&text, dialect);
        let compacted = compact_text(&text, dialect);
        prop_assert_eq!(&align_text(&aligned, dialect), &aligned, "align is not idempotent");
        prop_assert_eq!(&compact_text(&compacted, dialect), &compacted, "compact is not idempotent");
        prop_assert_eq!(&compact_text(&aligned, dialect), &compacted, "compact does not undo align");
    }

    #[test]
    fn rendering_preserves_values_blankness_and_errors(
        text in wild_text(),
        dialect in dialect(),
    ) {
        let before = render_expectation(&text, &parse(&text, dialect));
        for rendered in [compact_text(&text, dialect), align_text(&text, dialect)] {
            let reparsed = parse(&rendered, dialect);
            assert_table_sound(&rendered, &reparsed);
            prop_assert_eq!(shape(&rendered, &reparsed), before.clone(), "for {:?}", rendered);
        }
    }

    #[test]
    fn dialect_conversion_is_lossless_on_clean_tables(
        (text, from) in clean_text_and_dialect(),
        to in dialect(),
    ) {
        let table = parse(&text, from);
        prop_assert!(table.errors.is_empty());
        let opts = RenderOptions {
            dialect: to,
            quote_policy: QuotePolicy::PreserveOrRequired,
            ..RenderOptions::compact_for(&table)
        };
        let converted = render(&text, &table, &opts);
        let reparsed = parse(&converted, to);
        assert_table_sound(&converted, &reparsed);
        prop_assert!(reparsed.errors.is_empty(), "conversion broke quoting: {converted:?}");
        prop_assert_eq!(
            shape(&converted, &reparsed).0,
            render_expectation(&text, &table).0,
            "conversion changed cell values: {:?}", converted
        );
    }

    #[test]
    fn minimize_yields_one_applicable_addressable_edit(
        old in wild_text(),
        new in wild_text(),
    ) {
        let edits = minimize(&old, &new);
        prop_assert!(edits.len() <= 1);
        prop_assert_eq!(edits.is_empty(), old == new);
        let index = LineIndex::new(&old);
        for &(span, _) in &edits {
            assert_span_sound(&old, span);
            assert_span_addressable(&old, &index, span);
        }
        prop_assert_eq!(apply(&old, &edits), new);
    }

    #[test]
    fn positions_round_trip_at_every_addressable_offset(
        text in wild_text(),
        enc in encoding(),
    ) {
        let index = LineIndex::new(&text);
        for offset in text.char_indices().map(|(i, _)| i).chain([text.len()]) {
            if between_crlf(&text, offset) {
                continue; // not expressible as an LSP position
            }
            let position = index.position(&text, offset, enc);
            prop_assert_eq!(index.offset(&text, position, enc), offset, "{:?}", enc);
        }
    }

    #[test]
    fn hostile_client_positions_clamp_cleanly(
        text in wild_text(),
        enc in encoding(),
        line in prop_oneof![3 => 0u32..10, 1 => any::<u32>()],
        character in prop_oneof![3 => 0u32..64, 1 => any::<u32>()],
    ) {
        let index = LineIndex::new(&text);
        let offset = index.offset(&text, lsp_types::Position { line, character }, enc);
        prop_assert!(offset <= text.len());
        prop_assert!(text.is_char_boundary(offset));
        // The spec clamps characters to the line's content end, so a clamped
        // offset must itself be addressable.
        prop_assert!(!between_crlf(&text, offset));
    }

    #[test]
    fn dialect_detection_is_total(text in prop_oneof![wild_text(), any::<String>()]) {
        let _ = Dialect::sniff(&text);
        let _ = Dialect::from_path(&text);
        let _ = Dialect::from_language_id(&text);
    }

    #[test]
    fn feature_actions_honor_the_lsp_edit_contract(
        text in wild_text(),
        language_id in prop::sample::select(&["csv", "tsv", "ssv", "psv", "plaintext", ""][..]),
        (line_a, char_a) in (0u32..10, 0u32..48),
        (line_b, char_b) in (0u32..10, 0u32..48),
        only_mask in proptest::option::of(0u8..32),
    ) {
        let doc = Document::new(
            "file:///t/data".parse().unwrap(),
            language_id,
            1,
            text,
        );
        // The requested range exactly as the server derives it from client
        // positions (reversed ranges included — the server normalizes).
        let a = doc.line_index.offset(
            &doc.text,
            lsp_types::Position { line: line_a, character: char_a },
            PositionEncoding::Utf16,
        );
        let b = doc.line_index.offset(
            &doc.text,
            lsp_types::Position { line: line_b, character: char_b },
            PositionEncoding::Utf16,
        );
        let range = Span::new(a.min(b), a.max(b));
        let advertised = [
            CodeActionKind::QUICKFIX,
            CodeActionKind::SOURCE,
            CodeActionKind::SOURCE_FIX_ALL,
            CodeActionKind::REFACTOR,
            CodeActionKind::REFACTOR_REWRITE,
        ];
        let only: Option<Vec<CodeActionKind>> = only_mask.map(|mask| {
            advertised
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, kind)| kind.clone())
                .collect()
        });
        let registry = Registry::standard();

        for diag in registry.diagnostics(&doc.text, &doc.table) {
            assert_span_sound(&doc.text, diag.span);
            assert_span_addressable(&doc.text, &doc.line_index, diag.span);
        }

        let ctx = ActionContext {
            doc: &doc,
            range,
            client_diagnostics: &[],
            only: only.as_deref(),
        };
        for action in registry.actions(&ctx) {
            prop_assert!(
                !action.edits.is_empty() || action.command.is_some(),
                "{:?} is a no-op action", action.title
            );
            let mut previous_end = 0;
            for &(span, _) in &action.edits {
                assert_span_sound(&doc.text, span);
                assert_span_addressable(&doc.text, &doc.line_index, span);
                prop_assert!(
                    previous_end <= span.start,
                    "{:?} edits overlap or are out of document order", action.title
                );
                previous_end = span.end;
            }
            // Applying the action must never panic, and the outcome must be
            // a sound parse under the dialect the document ends up in.
            let applied = apply(&doc.text, &action.edits);
            let dialect = action.dialect_change.unwrap_or(doc.dialect);
            assert_table_sound(&applied, &parse(&applied, dialect));
        }

        // The document-highlight payload obeys the same span contract.
        if let Some((_, column)) = doc.table.cell_at(range.start) {
            for span in columns::column_content_spans(&doc.table, column) {
                assert_span_sound(&doc.text, span);
                assert_span_addressable(&doc.text, &doc.line_index, span);
            }
        }
    }

    #[test]
    fn pad_fix_all_repairs_every_short_row(text in wild_text(), dialect in dialect()) {
        let doc = doc_for(dialect, text);
        let registry = Registry::standard();
        let ctx = ActionContext {
            doc: &doc,
            range: Span::new(0, 0),
            client_diagnostics: &[],
            only: None,
        };
        let Some(fix_all) = registry
            .actions(&ctx)
            .into_iter()
            .find(|action| action.title.starts_with("Pad all short rows"))
        else {
            return Ok(());
        };
        let applied = apply(&doc.text, &fix_all.edits);
        let repaired = parse(&applied, doc.dialect);
        let missing: Vec<_> = registry
            .diagnostics(&applied, &repaired)
            .into_iter()
            .filter(|diag| diag.code == "row-missing-cells")
            .collect();
        prop_assert!(missing.is_empty(), "fixAll left short rows in {applied:?}: {missing:?}");
    }

    #[test]
    fn formatting_is_idempotent_at_the_edit_level(text in wild_text(), dialect in dialect()) {
        let doc = doc_for(dialect, text);
        let edits = csv_lsp::features::align::align_edits(&doc);
        let aligned = apply(&doc.text, &edits);
        let doc = doc_for(dialect, aligned);
        prop_assert_eq!(
            csv_lsp::features::align::align_edits(&doc),
            Vec::new(),
            "formatting an already formatted document must be a no-op"
        );
    }
}
