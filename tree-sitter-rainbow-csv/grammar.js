// Rainbow-highlighting grammar for delimiter-separated files (CSV, TSV,
// SSV, PSV). A rewrite of weartist/rainbow-csv-tree-sitter (MIT) that
// keeps its interface — node types `first`…`seventh` carry the column
// position mod 7, so Helix's bundled csv highlight queries work as-is —
// while fixing its parse failures:
//  - empty cells parse cleanly (they simply produce no cell node, so
//    non-empty cells keep their true column position),
//  - columns beyond the 7th cycle back to `first` instead of erroring,
//  - tab is a delimiter too, so TSV files get rainbow columns,
//  - quoted cells support RFC 4180 `""` escapes.

const DELIM = () => choice(",", ";", "|", "\t");
// The character class is built from a JS string so the control characters
// land in the pattern literally (escape sequences inside a regex literal's
// character class are not reliably honored by tree-sitter's lexer).
const CELL = () => choice(/"([^"]|"")*"/, new RegExp("[^,;|\t\n\r]+"));

// Column i of the cycle: either a cell (optionally followed by a delimiter
// and the rest of the row), or — for an empty cell — just the delimiter and
// the rest. Hidden (`_`) rules splice their children into `row`.
function col(cell, next) {
  return ($) =>
    choice(
      seq($[cell], optional(seq(DELIM(), optional($[next])))),
      seq(DELIM(), optional($[next]))
    );
}

module.exports = grammar({
  name: "csv",

  // Not the default /\s/: tab is a delimiter and newline a row terminator,
  // so neither may be skippable. Spaces (alignment padding) and the CR of
  // CRLF line endings are the only insignificant whitespace.
  extras: () => [/[ \r]/],

  rules: {
    // The last line may lack the trailing newline.
    csv: ($) => seq(repeat($.row), optional($._col1)),

    // A blank line is a row without cells.
    row: ($) => choice("\n", seq($._col1, "\n")),

    _col1: col("first", "_col2"),
    _col2: col("second", "_col3"),
    _col3: col("third", "_col4"),
    _col4: col("fourth", "_col5"),
    _col5: col("fifth", "_col6"),
    _col6: col("sixth", "_col7"),
    _col7: col("seventh", "_col1"),

    first: () => CELL(),
    second: () => CELL(),
    third: () => CELL(),
    fourth: () => CELL(),
    fifth: () => CELL(),
    sixth: () => CELL(),
    seventh: () => CELL(),
  },
});
