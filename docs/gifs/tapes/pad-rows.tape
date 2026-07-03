# Pad quickfix on one row, then "Pad all short rows" from the header.
# Run from the repository root:  vhs docs/gifs/tapes/pad-rows.tape
# Menu selections are by position (Down counts) and match Helix's kind-sorted menu
# (docs/features.html#anatomy); adjust the counts if your Helix version sorts differently.
Output docs/gifs/pad-rows.gif
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
Type "cp docs/gifs/samples/inventory.csv /tmp/demo.csv && hx /tmp/demo.csv"
Enter
Sleep 3s
Show

# Cursor onto the short `2,nut,250` row.
Sleep 1.5s
Type "gg"
Type "2j"
Sleep 1s
Type " a"
Sleep 2s
# First entry: "Pad row with 1 empty cell" (the preferred quickfix).
Enter
Sleep 2.5s

# From the header, repair the rest of the file at once.
Type "gg"
Sleep 800ms
Type " a"
Sleep 2s
# Entries: Quote cell / Quote column / Pad all short rows (1) / …  → entry 3
# (no quickfix on a complete row; the quote refactors sort first).
Down 2
Enter
Sleep 3s

Hide
Type ":q!"
Enter
