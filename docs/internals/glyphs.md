# Missing or corrupted characters and glyphs

> For players, the short version is in [the guide](../guide/looks.md).

If your map is peppered with tofu boxes or question marks, your font is missing
some of the line art lanthorn draws with. Any mono-space Nerd Font carries the
lot: https://www.nerdfonts.com

Here is exactly what the map asks of your font, so you can check a favourite
before switching away from it:

| Range | Block | Used for |
|---|---|---|
| `U+2500`–`U+257F` | Box Drawing | room outlines, connector paths, junctions |
| `U+2580`–`U+259F` | Block Elements | panel fills, dividers, the half-block image renderer |
| `U+2190`–`U+2193`, `U+2196`–`U+2199` | Arrows | connector arrowheads, including the diagonals `↖↗↘↙` |
| `U+25B2`, `U+25B6`, `U+25BC`, `U+25C0`, `U+25CF` | Geometric Shapes | filled arrowheads, the note marker `●` |
| `U+2297`, `U+2299` | Misc. Mathematical | in/out portal icons `⊗ ⊙` |
| **`U+1FBA0`–`U+1FBA3`** | **Symbols for Legacy Computing** | **the diagonal corner stubs `🮠🮡🮢🮣`** |

Everything above the last row has been in Unicode for decades and is safe
essentially everywhere. **The half-diagonals are the one modern ask** — Symbols
for Legacy Computing arrived in Unicode 13 (2020), and plenty of otherwise
excellent fonts still don't cover it. If your diagonal *passages* come out blank
while everything else draws fine, that block is your culprit.

> **The fix, if your font is missing them.** Turn the stubs off and those
> connectors route orthogonally with plain box-drawing characters instead. The
> line is already in your `~/.lanthorn/style.toml`, commented out — uncomment it
> and set it to `false`:
>
> ```toml
> [map]
> diagonal_corners = false
> ```
>
> `reload-style` picks it up without restarting. Picking a font that covers the
> block works too, and keeps the nicer diagonals.

Style settings live in `~/.lanthorn/style.toml` (create it if absent — every
setting has a default, so it only needs the lines you change), and `reload-style`
applies edits without restarting. A per-game file at
`~/.lanthorn/saves/<story-filename>.save/style.toml` layers over the global one.
Styling belongs in `style.toml`, **not** `config.toml`; `[symbols]` in a config
file is a legacy location lanthorn will tell you to move.

Diagonal *arrowheads* are a different thing entirely — they live in the ancient
Arrows block, so if those are missing, something else is wrong. Individual glyphs
can also be swapped one at a time under `[symbols.overrides]`; see
[customization & configuration](customization.md).

Nerd Font glyphs themselves (Private Use Area) are strictly opt-in — you only
touch them if you choose a `nerdfont` preset for `arrow_set` or `portal_icons`.
The default look needs no patched font at all.

---
