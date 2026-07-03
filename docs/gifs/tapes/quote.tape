# Quote the cell under the cursor, then the whole column; quoted cells are skipped
# and a fully-quoted column offers nothing.
# Run from the repository root:  vhs docs/gifs/tapes/quote.tape
# Menu positions assume the server's deterministic action order (docs/features.html#anatomy);
# adjust the Down counts if your Helix version sorts the menu differently.
Output docs/gifs/quote.gif
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
Type "cp docs/gifs/samples/quotes.csv /tmp/demo.csv && hx /tmp/demo.csv"
Enter
Sleep 3s
Show

Sleep 1.5s
Type "/needs"
Enter
Sleep 800ms
Type " a"
Sleep 2s
# Entries: Align / Reinterpret ×3 / Convert ×3 / Quote cell / Quote column "note" / … → entry 8.
Down 7
Sleep 800ms
Enter
Sleep 2.5s

Type " a"
Sleep 2s
# "Quote cell" is gone for the now-quoted cell; same position is "Quote column \"note\"".
Down 7
Sleep 800ms
Enter
Sleep 2.5s

# Everything in the column is quoted — the quote actions have disappeared.
Type " a"
Sleep 2.5s
Escape
Sleep 1.5s

Hide
Type ":q!"
Enter
