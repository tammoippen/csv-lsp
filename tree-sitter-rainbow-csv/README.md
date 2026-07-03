# tree-sitter-rainbow-csv

A tree-sitter grammar for delimiter-separated files (CSV, TSV, SSV, PSV)
whose parse trees carry the column position: cells are named
`first` … `seventh`, cycling every seven columns. An editor highlight query
that maps those seven names to colors — 11 lines, see
[`queries/highlights.scm`](queries/highlights.scm) — gives rainbow columns.

It is a rewrite of [weartist/rainbow-csv-tree-sitter] (MIT), the grammar
Helix ships for its built-in `csv` language, keeping its node names and
captures (existing themes and queries keep working) while fixing what made
it derail:

| | upstream (rev `d3dbf91`, pinned by Helix) | this grammar |
|---|---|---|
| empty cells (`a,b,,,`) | parse error; error recovery merges rows, neighboring rows change color | clean parse; non-empty cells keep their true column color |
| more than 7 columns | parse error after the 7th | cycle wraps: 8th column = `first` again |
| tab as delimiter (TSV) | not a delimiter — whole row is one cell | supported |
| `""` escapes in quoted cells | parse error | supported (RFC 4180) |
| missing final newline | parse error on the last row | tolerated |

Delimiters are `,` `;` `|` and tab — any of them, in any file. That is the
same trade-off the upstream grammar makes for `,` `;` `|`: no per-file
delimiter detection (tree-sitter has no such notion), so a comma inside an
unquoted SSV cell still starts a new color. Quote such cells, or accept the
cosmetics; csv-lsp's diagnostics always use the file's real dialect and are
unaffected.

Blank lines are rows without cells. Alignment padding (spaces) and the CR
of CRLF line endings are insignificant. Quoted cells may contain
delimiters, line breaks and doubled quotes.

## Using it in Helix

See the [main README](../README.md#helix-setup) — point the `csv` grammar
at this directory, reuse it for the `tsv`/`ssv`/`psv` languages, and wire
up the queries.

## Regenerating

`src/` is generated (ABI 14, the version Helix consumes) and committed so
editors can build the parser without the tree-sitter CLI:

```sh
cd tree-sitter-rainbow-csv
tree-sitter generate --abi 14
```

Validate changes against a corpus with `tree-sitter parse` — empty cells,
rows of only delimiters, >7 columns, all four delimiters, quoted cells with
escapes/newlines, CRLF, and a file without a final newline should all parse
without a single `ERROR` node.

## License

MIT, like the rest of this repository. Node names, captures and the
delimiter set follow [weartist/rainbow-csv-tree-sitter] (MIT License,
Copyright (c) 2024 Hans).

[weartist/rainbow-csv-tree-sitter]: https://github.com/weartist/rainbow-csv-tree-sitter
