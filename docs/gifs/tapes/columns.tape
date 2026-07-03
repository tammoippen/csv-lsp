# Add an empty column right of "id", then delete it again — a byte-identical round trip.
# Run from the repository root:  vhs docs/gifs/tapes/columns.tape
# Menu selections are by position (Down counts) and match Helix's kind-sorted menu
# (docs/features.html#anatomy); adjust the counts if your Helix version sorts differently.
Output docs/gifs/columns.gif
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
Type "cp docs/gifs/samples/columns.csv /tmp/demo.csv && hx /tmp/demo.csv"
Enter
Sleep 3s
Show

# Cursor starts on the "id" header cell.
Sleep 1.5s
Type " a"
Sleep 2s
# Entries: Quote cell / Quote column / Align / Reinterpret ×3 / Convert ×3 /
#          Add column left / Add column right / Delete column  → entry 11: Add right.
Down 10
Sleep 800ms
Enter
Sleep 2.5s

# Move into the new empty (headerless) column and delete it again.
Type "3l"
Sleep 800ms
Type " a"
Sleep 2s
# Same list for the headerless column → entry 12: Delete column #2.
Down 11
Sleep 800ms
Enter
Sleep 3s

Hide
Type ":q!"
Enter
