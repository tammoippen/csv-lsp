# A mislabeled file: semicolon content under a .csv name. Reinterpret as SSV makes the
# diagnostics sane; Convert to CSV makes the content match the extension (quoting adapts).
# Run from the repository root:  vhs docs/gifs/tapes/reinterpret-convert.tape
# Menu selections are by position (Down counts) and match Helix's kind-sorted menu
# (docs/features.html#anatomy); adjust the counts if your Helix version sorts differently.
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

# The diagnostics are garbage: under CSV rules the decimal commas split cells, so
# every row reports "row has 2 cells, expected 1" — wrong dialect, confidently wrong.
Sleep 2.5s
Type " a"
Sleep 2s
# Entries: Quote cell / Quote column / Align / Reinterpret as TSV / as SSV / as PSV /
# Convert ×3 / …  → entry 5: Reinterpret as SSV.
Down 4
Sleep 800ms
Enter
Sleep 2.5s

# Sane view now: only the short `scheibe;0,10` row is flagged. Complete it.
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
# Entries: Quote cell / Quote column "menge" / Align / Reinterpret as CSV / as TSV /
# as PSV / Convert to CSV / …  → entry 7: Convert to CSV ("1,50" gains quotes).
Down 6
Sleep 800ms
Enter
Sleep 3s

Hide
Type ":q!"
Enter
