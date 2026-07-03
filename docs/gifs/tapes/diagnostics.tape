# Live diagnostics: squiggles appear on short rows and vanish as rows are completed.
# Run from the repository root:  vhs docs/gifs/tapes/diagnostics.tape
# Assumes csv-lsp is installed and Helix is configured (docs/quickstart.html#helix).
Output docs/gifs/diagnostics.gif
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

# Two rows are short — their diagnostics are already visible at the line ends.
Sleep 2s
Type " d"
Sleep 2.5s
Escape
Sleep 1s

# Complete the `3,washer` row and watch its diagnostic disappear live.
Type "gg"
Sleep 500ms
Type "3j"
Sleep 500ms
Type "gl"
Sleep 500ms
Type "a"
Sleep 300ms
Type ",75,0.05"
Sleep 500ms
Escape
Sleep 3s

Hide
Type ":q!"
Enter
