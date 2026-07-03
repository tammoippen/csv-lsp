# Space+h turns the column under the cursor into one selection per cell (empty cells
# become bare cursors); c edits every cell at once.
# Run from the repository root:  vhs docs/gifs/tapes/select-column.tape
Output docs/gifs/select-column.gif
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
Type "cp docs/gifs/samples/highlight.csv /tmp/demo.csv && hx /tmp/demo.csv"
Enter
Sleep 3s
Show

Sleep 1.5s
Type "/bolt"
Enter
Sleep 800ms

# Select every cell of the "name" column — note the bare cursor in the empty cell.
Type " h"
Sleep 2.5s

# Change all cells at once with ordinary multi-cursor editing.
Type "c"
Sleep 500ms
Type "part"
Sleep 800ms
Escape
Sleep 2s
Type "u"
Sleep 2s

Hide
Type ":q!"
Enter
