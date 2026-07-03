# :format aligns the columns; the "Compact columns" action restores the compact form.
# Run from the repository root:  vhs docs/gifs/tapes/align-compact.tape
# This tape is shown annotated in docs/features.html#recording-anatomy — keep them in sync.
Output docs/gifs/align-compact.gif
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
Type "cp docs/gifs/samples/messy.csv /tmp/demo.csv && hx /tmp/demo.csv"
Enter
Sleep 3s
Show

Sleep 1.5s
Type ":format"
Sleep 500ms
Enter
Sleep 2.5s

Type " a"
Sleep 1.5s
# Third entry: "Compact columns" (the file is aligned and clean, so nothing outranks it).
Down 2
Enter
Sleep 2.5s

Hide
Type ":q!"
Enter
