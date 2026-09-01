# Looks

For anyone who wants lanthorn to look like their own terminal, not somebody
else's theme — this page covers the styling surface, from the quick switches
to the full palette.

**Pick a theme.** Point `style.toml` at a [Ghostty](https://ghostty.org) theme
file, or use a built-in — `mono`, `high-contrast`, `tomorrow-night`. Set
nothing at all and lanthorn asks your terminal for its own foreground and
background colours and builds around those, so the status bar and dialogs sit
on your terminal's own page rather than a stranger's black one.

**Seven colours, one theme.** Everything lanthorn paints derives from seven
roles: `text` (body ink), `chrome` (ink on a UI surface — bars, panels, the
upper window), `line` (frames and rules), `accent` (highlights — selection,
the current room, tabs), `muted` (secondary text), `alert` (warnings), and
`heading` (titles). Set those seven and the whole app reads as one coherent
theme; reach further in — any individual selector, down to a single map
glyph — only if you want to.

**Edit it live.** On first run lanthorn seeds `~/.lanthorn/style.toml` fully
commented out, every selector already spelling its own default, grouped by
section — a working reference, not a blank page. Uncomment what you want to
change, save, and run `/reload-style` to see it live; a bad edit keeps the old
look and tells you why instead of crashing. Flip `watch_style = true` (or run
`/toggle-watch`) and every save reloads on its own. `config.toml` documents
itself the same way, and keeps documenting itself over time — a setting added
in a later release is appended to your file, commented, rather than needing a
fresh one.

**Per-game overrides.** Drop a `style.toml` next to a game's own saves (its
`.save` folder) to layer a look on top of the global theme for just that
story — it's reapplied every time that game opens. A `config.toml` can sit
beside it too, but it's deliberately tiny: at most a handful of keys such as
`honor_game_colours`, `show_map`, or `v6_render`, written for you the moment
you toggle one of those for that story. Leave a key out and the story
inherits your global setting, so there's no need to copy the whole file —
only the differences.

**The status bar, your words.** The `[statusbar]` section builds the line
from segments you assign to a left, center, or right cluster, each with its
own style. Templates take live placeholders — `{location}`, `{score}`,
`{moves}`, `{time}`, `{turns}`, `{title}` — so `Score: {score}  Moves:
{moves}` is a one-line rewrite away from the default.

**Your own keys.** The leader panel (default `Ctrl+P`) shows a reference of
frequent map-editing verbs, each bound to a mnemonic letter you can reassign
under `[[hotkeys.group]]`. Direct bindings — including the story picker's own
keys — live in `[keymap.global]` and its siblings as `"key" = "command
args"`; set `use_defaults = false` to clear the built-ins and start over.

**Fonts and glyphs.** The map draws with box-drawing and arrow characters any
font carries, plus one modern block — diagonal corner stubs — that not every
font covers yet. On first launch lanthorn shows two rows of glyphs and asks
which one your terminal draws cleanly, then writes the answer as presets
(`arrow_set`, `portal_icons`, `control_icons`) rather than dozens of
individual overrides; `/run-font-check` asks again whenever you change fonts,
and boxes or blank squares where glyphs should be just mean that block isn't
in your font. A patched [Nerd Font](https://www.nerdfonts.com) unlocks the
fancier `nerdfont` presets, but nothing requires one — the default look needs
no patched font at all. The portal markers default to shapes an ordinary
monospace face already carries — up, down, in, out as `↑ ↓ ◉ ◎` — and they
draw wherever a portal does: on the map, and in the command band's one-click
cluster beside the compass rose. Nerd Font presets, including a dedicated
stairway set, are there if you'd rather.

The full selector list, and every config key, lives in the reference tables
rather than repeated here: see [the style reference](../reference/style.md)
and [the config reference](../reference/config.md).

## Going deeper

- [Customization](../internals/customization.md) — how a setting is resolved, and every override surface
- [Missing or corrupted glyphs](../internals/glyphs.md) — the Unicode blocks the map's line art needs
- [Style reference](../reference/style.md) — every `style.toml` selector
- [Config reference](../reference/config.md) — every `config.toml` setting
