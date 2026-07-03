# A mislabeled file: semicolon content under a .csv name. Reinterpret as SSV makes the
# diagnostics sane; Convert to CSV makes the content match the extension (quoting adapts).
# Run from the repository root:  vhs docs/gifs/tapes/reinterpret-convert.tape
# Menu positions assume the server's deterministic action order (docs/features.html#anatomy);
# adjust the Down counts if your Helix version sorts the menu differently.
Output docs/gifs/reinterpret-convert.gif
Require hx
Require csv-lsp

Set Shell bash
Set FontSize 20
Set Width 1200
Set Height 660
Set Padding 12
Set Theme "Catppuccin Mocha"
Set TypingSpeed 60ms

Hide
Type "cp docs/gifs/samples/preise.csv /tmp/demo.csv && hx /tmp/demo.csv"
Enter
Sleep 3s
Show

# Note: no diagnostics — as CSV this parses as one column, the short row is invisible.
Sleep 2.5s
Type " a"
Sleep 2s
# Fifth Entries: Reinterpret as TSV / as SSV / as PSV / Convert to SSV / …  → pick entry 2.
Down 4
Sleep 800ms
Enter
Sleep 2.5s

# The short `scheibe;0,10` row is flagged now. Complete it.
Type "ge"
Sleep 500ms
Type "A"
Sleep 300ms
Type ";25"
Sleep 300ms
Escape
Sleep 2s

Type " a"
Sleep 2s
# Third Entry: 
Down 2
Sleep 800ms
Enter
Sleep 3s

Hide
Type ":q!"
Enter
