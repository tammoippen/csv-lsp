# Feature GIFs

The GIFs referenced by `docs/features.html` are **recorded, not committed** —
they are reproducible artifacts.

- `tapes/` — one [VHS](https://github.com/charmbracelet/vhs) script per GIF;
  the single source of truth for what each recording shows.
- `samples/` — the CSV files the tapes open. Tapes copy them to `/tmp` and
  quit Helix with `:q!`, so the samples are never modified by a recording.
- `*.gif` — the outputs land here (referenced as `gifs/<name>.gif` by the
  features page). Commit them once recorded if you want the docs to show
  them for everyone.

## Recording

Prerequisites: `csv-lsp` installed, Helix configured per
`docs/quickstart.html#helix` (verify with `hx --health csv`), and `vhs`
installed (`brew install vhs`).

From the **repository root**:

```sh
vhs docs/gifs/tapes/align-compact.tape          # one
for t in docs/gifs/tapes/*.tape; do vhs "$t"; done   # all
```

Full instructions, per-GIF manual scripts, and polish tips:
`docs/features.html#recording`.

Note: tapes select code-action menu entries by position (`Down` counts),
matching the server's deterministic action order. If your Helix version
sorts the menu differently, adjust the counts — each selection is marked
with a comment naming the entry it expects.
