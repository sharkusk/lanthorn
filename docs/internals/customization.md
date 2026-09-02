# Customization & configuration

> For players, the short version is in [the guide](../guide/looks.md).

[← back to README](../../README.md)

## Customization

Almost every pixel lanthorn paints is yours to repaint. Colours, borders, box
glyphs, the status line, the keymap, even the easing curve on a scroll — all of
it lives in two plain TOML files you can edit and reload without leaving the
game. This page walks the knobs from the ones you'll reach for first to the ones
that let you rebuild the whole look from scratch.

### Styling model — roles, panels, and Glk styles
Set seven colours and you've themed the entire app. That's the payoff of
`style.toml`'s **role palette**: a handful of roots that everything else derives
from, so a coherent theme falls out of almost no typing — while power users can
still reach in and override any single selector by name.

- **7 roles** (`[roles]`) are the roots a theme actually sets: `text` (body ink),
  `chrome` (ink on a UI surface — bars/panels/upper window), `line`
  (lines, frames, rules, dividers), `accent` (highlights — links, selection,
  current room, tabs), `muted` (dim/secondary text), `alert` (warnings/errors),
  and `heading` (emphasized titles). Everything else is a **derivation** —
  `parent = "<role>"`
  plus an optional delta (fg/bg/bold/italic/underline/dim/reversed) — so a
  minimal theme that only touches `[roles]` still looks fully coherent. The
  emphasis keys are **three-state**: leave one out and you inherit whatever your
  parent had, write `bold = true` to add it, and write `bold = false` to take it
  *away* — so "like the thing above me, but not bold" is something you can
  actually say.
- **Panels vs. windows.** *Panels* are the frames lanthorn itself draws — the
  story pane, map, command band, debug inspector, and every dialog/overlay.
  *Windows* are the surfaces the story/VM generates (Glk buffer/grid/graphics
  windows, the v4+ upper window). Panels are host chrome and never honor game
  colors; windows do, subject to the resolution chain below. Every panel shares
  **one** border under `[panel]` instead of a per-panel selector: `panel.border`
  when unfocused, `panel.border:active` when it has focus (bold by default —
  today's cyan+bold focus highlight), `panel.background` for the body fill, and
  `panel.title` / `panel.tab` / `panel.tab:active` / `panel.tab_divider` /
  `panel.terminator_left` / `panel.terminator_right` for the title/tab strip
  inset in the top border, and `panel.control` (off) / `panel.control:lit` (on,
  the `alert` role) / `panel.control:hover` for the clickable toggle controls on
  the story pane's own frame — bracketed by the same terminator caps, and placed
  on the border nearest whatever each one switches (every framed pane — story, map, dialogs, the command
  band and inventory dock, the debug inspector's window tabs, the story-list info
  panel — renders through this one shared panel component and these same
  selectors). The story pane's strip text is the resolved adventure title,
  with the story's filename appended in parentheses when it differs from the
  title (e.g. `Journey: The Quest Begins (journey-r83-s890706.z6)`) — a bare
  filename with no known title (or a file already named after it) shows with
  no parenthetical. The title is the *same* one the story picker lists, drawn
  from the same metadata in the same order — a blorb's own iFiction record, then
  the fetched IFDB details cached beside your saves, then lanthorn's bundled
  title tables — so a game can't be *Anchorhead* in the library and `anchor` in
  the pane. The game's opening banner is consulted only after all of those, and
  the filename is the last resort it was always meant to be. A story mounted off
  a **disk image** always names its `.adf` there, however neatly the box-spelled
  filename matches the title: a floppy carries a different *release* of the game,
  and which one you're playing is exactly what the border should tell you.
  The strip's bracket caps and divider track the pane's border
  style by default (`┤ … ├` on single, `┫ … ┣` on thick, `╡ … ╞` on double);
  set `panel.terminator_left` / `panel.terminator_right` / `panel.tab_divider`
  to a `glyph` to override any of them. The map additionally sets
  its own canvas fill, `map.background`, since it isn't a Glk window.
  The one *window* frame lanthorn can draw itself is the box around a v4+ game's
  status/upper window, and it answers to `upper_window_border` in `[elements]` —
  the selector that colours it carries its shape too. It is **off by default**:
  the status line sits flush against the story, and the whole pane is the screen
  the game is told it has. Ask for a box with `style = "single"` (or
  `double`/`thick`/`rounded`), reach for `style_top` / `style_bottom` /
  `style_left` / `style_right` to rule one edge at a time, and remember that
  every side you turn on costs the story a row or a column. Don't reach for
  `[statusbar]`'s `border` for this — that frames lanthorn's own status bar, not
  the game's window.

  The **same selector draws the rule between two Glk windows** — the line you'd
  see under a Glulx game's status bar, or down the side of a split. Glk lets a
  game ask for that border, but `winmethod_Border` is the *default value* of the
  flag rather than a considered request, so honouring it drew a line under
  practically every Glulx game whether or not its author ever thought about one.
  Your theme decides instead, and it is off by default: no rule, and no gutter
  reserved for one either, so the windows sit flush. Turn it on the same way you
  turn on the status box — `style = "single"`, `double`, `thick` — and the rule
  is drawn in that style's own glyph (`─`, `═`, `━`), horizontal between stacked
  windows and vertical between side-by-side ones. `glyph_top` overrides the
  horizontal rule and `glyph_left` the vertical one, and the colour is the
  selector's own `fg`/`bg`. Two things still speak over your theme: a game that
  explicitly asks for `winmethod_NoBorder` never gets a rule, and a game that
  paints its *own* divider next to the gutter (Kerkerkruip does) suppresses ours
  rather than doubling the line. A game's window colours can recolour the rule
  too — but only while `honor_game_colours` is on, which is the setting that says
  the game's palette wins. Turn it off and your border colour is final.
- **The 11 standard Glk styles** — Normal, Emphasized, Preformatted, Header,
  Subheader, Alert, Note, BlockQuote, Input, User1, User2 — are first-class,
  addressable selectors under `[glk.buffer]` (text-buffer windows) and
  `[glk.grid]` (text-grid/status windows), each carrying fg/bg plus
  bold/italic/underline/reversed. Each style defaults to a role-derived look (a
  game that sets no styles renders identically to a role-only theme) but can be
  overridden per slot for full Glk fidelity. A text-buffer window's background
  is its `glk.buffer.normal.bg` (defaults to `text.bg`) — there's no separate
  background knob there. A **Glk text-grid window's ground is different**:
  `glk.grid.background` (default: reversed `chrome` — the same spelling
  `status_bar`/`help_bar` use) is the GROUND, the cells the game never wrote,
  kept apart from `glk.grid.normal` — the per-style colour a game paints INTO a
  cell it did write. That split exists because a grid window carries no border
  by default (the point above), so an unstyled one used to be visually
  indistinguishable from the terminal page it sat on — a mouse-driven menu with
  no visible extent at all. Reversing the ground instead of colouring it works
  on any terminal palette, the same reason the status bar is reverse video
  rather than a named colour. This ground is Glk-only: a Z-machine or Scott
  upper window still grounds on `upper_window`, unreversed, because those games
  paint their own reversal and a default one would double it up.
- **`[map]`** owns every map-domain selector: colors (`room`, `room_current`,
  `room_selected`, `connector`, `connector_distorted`, `connector_portal`,
  `shared_path`, `layer_cycle`, …) and the glyph-set presets that used to live
  in a standalone `[symbols]` section — `box_style` (rounded / thick / double /
  **solid** / **super-thick** / ascii / borderless), `arrow_set` (including Nerd
  Font Material Design families), `portal_icons` (including a 4-icon stairs
  set), `path_style` for cardinal (N/S/E/W) connectors, and a separate
  `portal_path_style` for vertical/portal (up/down/in/out) connectors so they
  can render distinctly (dotted by default). `control_icons` (plain | nerdfont)
  picks the glyphs for **every** border control — the story pane's cluster (the
  map and verb-panel toggles, the Guiding Light's mark, the word reveal's lamp,
  the return probe's footprint and the two v6 render switches), the map pane's
  own five (room numbers, centre, zoom out, zoom in, view) and the tooltip
  pointer, all off one key so a single answer dresses the whole interface.
  `plain` is shape-based — Geometric Shapes for the story pane, and marks an
  ASCII face is certain to carry for the map's (`#`, `¤`, `−`, `+`, `M`);
  `nerdfont` gives every state a named icon, every codepoint read from the font's own `post` table rather than
  guessed from a name, with each control's states drawn from a single icon
  family so a toggle changes shape without changing weight. `diagonal_corners = false` turns
  the half-diagonal corner stubs (🮠🮡🮢🮣) back into plain orthogonal exits, for
  fonts without Unicode 13 Legacy Computing coverage. Individual glyphs are
  overridden one slot at a time in a `[map.overrides]` table keyed by slot name
  — `"room.normal.tl" = "+"`, `"arrow.north" = "^"`, `"path.diag_ul" = "/"`.
  On a **first launch** lanthorn asks which of two glyph rows your terminal draws
  properly and writes the answer into this section for you — `arrow_set`,
  `portal_icons`, `control_icons` and the Guiding Light's `"gutter.assist"`
  together, plus `badge_icons` over in `[elements]`, which is where the story
  picker's row badges live. A patched font supplies all of them or none of them,
  so they are one answer rather than five. It has to ask: lanthorn writes
  characters and the font belongs to the terminal, and the nearest thing to a
  probe measures a glyph's *width*, which a missing-glyph box passes.
  Both rows also end with the four diagonal corner stubs, identically — those are
  Unicode 13 Legacy Computing rather than Nerd Font, so no answer here changes
  them and neither row can be the one that "fixes" them. They are shown so you
  can see whether your terminal draws them at all; if they come up as empty
  boxes, `diagonal_corners = false` is the one-line answer. It writes
  preset **names**, not the forty expanded overrides they stand for, so the
  section stays readable and a later improvement to a preset still reaches you.
  `/run-font-check` asks again — worth doing whenever you change terminal fonts —
  and so does `--font-check on`; `--font-check off` never asks. Esc means "the
  plain row", and is recorded, so the question does not come back every launch.
  (The settings screen used to carry a `font_check` row that ran it. It no longer
  does: that screen is Global Settings, every row on it holds a value the Save
  button writes to `config.toml`, and a row that merely *ran* something had its
  answer land in `style.toml` instead — outside anything Save or Cancel could
  speak for.)
  If that first launch happened to be piped, redirected or otherwise unable to
  show a dialog, the question is not lost: lanthorn notes that it is still owed
  and asks on the next launch that *can* ask. Ctrl-C is different — that is you
  dismissing it, so it does not come back on its own.
- **`[debug]`** holds the selectors particular to the debug inspector: `pc`, the
  four confidence tiers that shade how sure the disassembler is that a byte is
  really code, and `zstring`. The tier defaults read as a risk gradient —
  **blue** verified, **yellow** medium, **red** high-risk:
  - **`disasm_executed`** (blue) — the line's address has *ever* run. Ground
    truth; it wins over any static guess and stays blue for the rest of the
    session (cumulative coverage). Its `|` gutter mark is separate: it flags only
    the lines that ran during the *last* command, so the bar tracks the most
    recent turn while the colour accumulates.
  - **`disasm_rd`** (yellow) — hard-discovered code: reached by recursive descent
    from a constant call target or the initial PC, or later confirmed by execution.
  - **`disasm_soft`** (red) — a linear-scan guess that hasn't been verified yet —
    the "don't fully trust this" tier.
  - **`disasm_data`** — bytes shown as `.byte`, not decoded as code at all
    (muted; it's not a risk level).

  - **`zstring`** — the Memory view's decoded-text column: the story's own words
    printed beside the bytes that encode them, row for row. Story text rather
    than a confidence tier, so it takes `accent` and italics by default, to read
    as a gloss on the hex dump instead of more of it.

  Each tier carries both a line style and a gutter **`glyph`** (e.g.
  `disasm_executed`'s `|` mark; the others default to a blank space — set
  `disasm_soft = { glyph = "?" }` to flag guesses), so the colour and the mark
  are both themeable. The panel's frame/body/tabs come from the shared `[panel]`
  chrome above, and its opcode hover tooltip from the shared `[tooltip]` surface
  (below), not from `[debug]`.
- **Surfaces beyond `[panel]`.** Dialogs and tooltips are their own **surface**
  sections — a background + optional frame + the text on them — separate from
  `[panel]`. `[dialog]` styles the modal surface (`background`, its own `border`
  frame, `title`, `button` / `button:active`, `shadow`); `[tooltip]` styles every
  hover tooltip (`background` + an optional `border`, borderless by default).
  A tooltip is a **card lying on the page**, and none of the seven roles is one:
  only `chrome` carries a background at all, and `chrome` *is* the page — deriving
  the tip from it painted the card in exactly the colours it floats over, which is
  the same thing as not drawing it. `accent` fails the same way one level down,
  cyan ink with no fill behind it. So the tip borrows `dialog.list_selected`, the
  Black-on-Cyan highlight every menu already uses for the row you're on: a real
  surface, and one you've seen before. It takes that highlight's **colours but not
  its weight** — bold reads as "this one" on a single selected row and as a bold
  paragraph on a multi-line card — and retuning the highlight moves your tooltips
  with it. Set `background = { parent = "chrome" }` to get the old invisible
  behaviour back, or any `fg`/`bg` pair you prefer. The tip
  also grows a **pointer** aimed at the icon it explains, drawn in the box's own
  background so the two read as one shape; the box is centred on that icon so the
  pointer sits near its middle, sliding off-centre only when a pane edge shoves
  the box along. The pointer's glyphs follow `map`'s `control_icons` — a wedge on
  a patched font, a flat half-block tab otherwise — and each of its four cells is
  overridable under `[map.overrides]` as `"tip.up_left"`, `"tip.up_right"`,
  `"tip.down_left"`, `"tip.down_right"`. Keys
  in these sections are bare (`title = { parent = "accent" }`), like `[panel]`
  keys. The story picker's **IFDB search** modal (`/`) reuses this `[dialog]`
  chrome and adds five `[elements]` selectors for its contents: `ifdb_result` (a
  game/file row), `ifdb_result_selected` (the highlighted row, accent + bold +
  reversed), `ifdb_result_meta` (the rating/year tail and hint line),
  `ifdb_download_marker` (the ⭳ glyph on a download option), and
  `ifdb_attribution` (the "Results from IFDB" credit line). The **region prompt** —
  the modal that offers to give a set of rooms a layer of its own, and that picks
  between passages or destinations when `move-region` cannot settle one for itself —
  adds four more `[dialog]` keys: `region_prompt.body` (what is being asked),
  `region_prompt.rooms` (the dimmer line naming the rooms that would travel),
  and `region_prompt.option` / `region_prompt.option:chosen` (a choice row, and
  the one currently picked, which borrows the shared `list_selected` highlight).
  The **saves manager**
  adds two more for its Type column: `saves_portable` (accent, and its `glyph`
  supplies the `↗` mark on a save another interpreter can read) and
  `saves_host_only` (muted — a host snapshot that stays put).

### Everyday customization
Below the full role system sit the knobs most people actually touch — the small
switches that make lanthorn feel like yours without opening the whole registry.

- **Room numbers** — room id numbers are hidden by default (portal icons take the
  freed bottom row); flip them on with the `toggle-room-numbers` command,
  persisted via the `show_room_numbers` setting.
- **Color schemes** — recolor rooms, connectors, and chrome from a
  [Ghostty](https://ghostty.org) theme file or a built-in (mono / high-contrast /
  tomorrow-night), with per-role and per-selector overrides. Defaults to your
  terminal colors — genuinely so: with no scheme set, lanthorn asks the terminal
  for its own default foreground and background (OSC 10/11, at startup) and hands
  the answer to the `chrome` role, so the status bar, upper window and dialog
  surfaces sit on your terminal's page rather than a black one. A terminal that
  declines to answer, or answers only half, falls back to the built-in dark
  palette rather than mixing a real ink into a guessed page. On Windows the
  question is not asked at all — there is no non-blocking console read to hear
  the answer with, and an answer nobody reads is one the terminal types into
  your game instead — so Windows takes that same fallback. `print-colors` prints
  the active, resolved scheme to the transcript (`print-colors color` also renders
  each entry in its own color).
- **Configurable status bar** — the `[statusbar]` section builds the status line
  from templated segments assigned to a left / center / right cluster. Each
  segment can set its own style directly, or ride a role via `parent = "accent"`.
  Templates substitute live `{placeholder}` values — `{location}`, `{score}`,
  `{moves}`, `{time}`, `{turns}`, `{title}`, `{filter}` — so you can compose
  exactly the readout you want (e.g. `Score: {score}  Moves: {moves}`) instead
  of a fixed layout.
- **Animations** — the transcript glides to its new position on an easing curve
  instead of snapping there. Tune it under `[animation]` in `config.toml`
  (`enabled`, `easing`, `scroll_ms`), or set `enabled = false` (or `scroll_ms =
  0`) to have every scroll land instantly. The same section holds the story
  pane's auto-hiding scrollbar: `scrollbar_hide_ms` (default 1500) is how long
  the bar stays up after you scroll — `0` keeps it up permanently — and
  `scrollbar_fade_ms` (default 300) how long it takes to fade away, `0` for a
  clean pop. Its two colours are yours as well: `scrollbar` paints the thumb and
  `scrollbar_track` the channel it runs in, both as background fills rather than
  glyphs, so nothing crowds the prose beside them. Those two selectors dress
  every bar in the app, whichever way it runs — including the horizontal one
  under the debug inspector's Memory dump, which pans sideways rather than
  scrolling down and takes the same thumb and channel colours.
- **Transcript text styling** — color each transcript category independently via
  bare selectors — `transcript`, `transcript_input`, `transcript_meta`,
  `transcript_warning`, `transcript_system`, `transcript_crash` (`fg`/`bg`/
  `bold`/`italic`). Story lines also run through styling rules: built-in ones for
  the room-name **location** header (`transcript_location`) and bracketed
  **system** lines such as `[Your score just went up.]` (`transcript_system`),
  plus your own ordered `[[transcript.rule]]` regex rules in `style.toml` (e.g.
  paint every `grue` red). Those selectors carry the meta and warning lines'
  **colour**; the gutter **mark** beside them is a glyph override, in the same
  table and the same shape as the assist mark described below — `"gutter.meta"`
  (`▏` by default) and `"gutter.warning"` (`!`) under `[map.overrides]`.
  **Lanthorn's Guiding Light** — the help offered while you play — has three
  selectors of its own. `transcript_assist` and `transcript_assist_caution` are
  the two it speaks in, both parented on `alert` (your terminal's yellow slot, so it stays legible on
  a light page as well as a dark one) and separated by weight, the caution tone
  bold; `transcript_reveal` is the ink the **word reveal** lays over the story's
  own prose, parented on `accent` and underlined — underlined because it has to
  read against whatever colour the game already painted those words, and a
  foreground alone cannot promise that.
  What identifies an assist line on screen is not its words but the **mark**
  in their margin, `●` by default; the glyph is yours, under `[map.overrides]`:
  `"gutter.assist" = "●"`. Point it at a patched font's own lamp — U+F1A60,
  Nerd Fonts' `md-post_lamp` — if you have one installed. (Not `*`: Infocom
  games spend asterisks on footnotes.) An **exported** transcript is the one
  place the words appear instead, because a file has no margin and no colour:
  every assist line comes out of `/export-transcript` prefixed `Lanthorn: `.
  `/dump-terminal`'s report rides the same transcript styling,
  with two selectors of its own: `terminal_dump_heading` for its section headings
  and `terminal_dump_assumed` — parented on `alert` — for every line carrying a
  value lanthorn **guessed** rather than measured, or one it could not reach at
  all. That colour difference is the command's whole point, so it is worth
  keeping loud. On top of all that, the game's own **`set_text_style`**
  emphasis (bold / italic / reverse-video) is rendered per-span — a bold word
  inside a sentence shows just that word bold — layered over the category/rule
  colors and preserved across save/reload.
- **Tmux-style leader keymap**: a configurable prefix (default `Ctrl+P`) pops up
  a **reference panel** of frequent map-editing verbs, each on a **mnemonic
  single letter** — `t`idy, `a`nimate, `p`eel, `m`erge, `c`ycle-layer, `r`ename
  room, `n`otes, `d`elete connection, `e`dge relabel, `i`nventory, portal
  `l`abels, `v`erb menu, `+`/`-` zoom, `0` centre map, `s`ettings, `h`istory,
  reset `g`ame — grouped as
  Layout / Layers / Edit / View / Map / Session. Pressing a letter runs the command and
  returns to normal — one keypress, then the panel closes (any unbound key or
  `q`/`Esc` just closes it; `q` is deliberately left unassigned so it closes).
  The long tail (exports, pane resizing, `rename-layer`, `toggle-map`,
  `toggle-inspector`, `toggle-alignment`, …) lives in the `/` command palette
  below rather than the panel. A small always-active set stays live outside the
  panel and is advertised in the bottom hint bar: `Ctrl+S`/`Ctrl+R`
  (save/restore state), quit, and `Shift+Arrow` to pan the map — all of which
  work while you type, since the map never takes the keyboard. Tab appears there only while the debug inspector is open, which is
  the one thing it still steps through.
  Leader letters are set per group under `[[hotkeys.group]]` in
  `config.toml` (`commands = ["t tidy-map", …]`; a bare `"tidy-map"` auto-assigns
  the first free letter), and the letter's color is themeable via the
  `hotkey_key` style selector. Direct key bindings
  still live in `[keymap.global]`, `[keymap.map]` (reached only while the debug
  inspector holds the right-hand pane; it ships no defaults of its own),
  `[keymap.anim]`, and `[keymap.browser]` (the story picker — see below) as
  `"key" = "command args"` — the **key on the left**, the command it runs on the
  right, spelled the way the registry spells it (hyphenated: `save-state`,
  `zoom-map in`). Bind one command to two keys by writing two entries. Get the two
  sides the wrong way round and the entry is skipped with a warning at game start
  that says so and quotes the corrected line. Set `use_defaults = false` under
  `[keymap]` to clear the built-ins and define your own from scratch.

  Two things worth knowing before you pick a key. A binding for a command outside
  the always-available `direct` set only fires from the story prompt, not from map
  focus and not with a Ctrl modifier — that set is what "available without opening
  the leader panel" means. And while a story is waiting on a single keypress
  (menus, "press any key"), every *plain* key goes to the game; only Ctrl and Alt
  combos are held back for lanthorn. So a diagnostic you want reachable at any
  moment wants a Ctrl binding:

  ```toml
  [keymap.global]
  "ctrl+d" = "dump-windows"
  "ctrl+g" = "dump-cells"
  "ctrl+t" = "dump-terminal"
  ```
- **The story picker's keys are bindable too** — the screen you get when
  lanthorn is pointed at a directory used to be the one surface whose keys were
  not data: hardcoded match arms, and a footer hint typed out by hand beside
  them. They now go through the same registry as everything else, in their own
  `Browser` context, so every one of them can be moved:

  ```toml
  [keymap.browser]
  "p" = "play-story"
  "ctrl+f" = "search-ifdb"
  ```

  The commands are `move-selection <dx> <dy>`, `page-selection <n>`,
  `select-edge first|last`, `play-story`, `open-launch-options`,
  `toggle-info-panel`, `toggle-gallery`, `fetch-story`, `refresh-library`,
  `set-ifdb-url`, `search-ifdb`, `download-hints`, `sort-library`,
  `reverse-sort`, `find-story`, `parent-folder`, `quit-browser` and
  `cancel-browser`. They are a world of their
  own: a game command in `[keymap.browser]` is refused with a warning (there is
  no game yet for it to act on), and these do not appear in `/help` or the
  command palette, because the picker has no command line to type them into.
  The footer hint bar is *generated* from these bindings, so rebind `g` and the
  footer says so without anyone editing a string.
- **Command palette** — press `/` at an empty prompt (or `/` inside the leader
  panel) to open a fuzzy search over every command; its rows theme via five
  `[elements]` selectors: `palette_query` (the input line), `palette_name` (a
  command name), `palette_match` (the fuzzy-matched characters, accent + bold by
  default), `palette_desc` (the one-line help), and `palette_selected` (the
  highlighted row). Its frame reuses the shared `[dialog]` chrome.
- **Command band** — the band's own parts theme via three `[elements]`
  selectors: `band.column_header` / `band.column_header:active`, `band.quick`
  (the one-click words, rose or flat row) and `band.group_label` (in-column
  labels and the `(nothing visible)` placeholder). Its rows — and the armed
  quick word — reuse `dialog.list_selected`; it draws no frame, and borrows
  `panel.border:active`'s colour for its whole fill while resize mode is
  targeting it.
- **Decorated panes** — configurable per-pane borders (`none`/`single`/`double`/
  `thick`/`rounded`) via the shared `[panel]` chrome above: unfocused panels use
  `panel.border`, the focused one uses `panel.border:active`. The map's top
  border carries a centered **layer-tab strip** (active layer highlighted, via
  `panel.tab:active`); the story's top border shows the **adventure title**
  (taken from an override, the game's opening banner, or the filename). The
  status line and input prompt can be boxed too — all via `style.toml`.
- **Unified dialogs** — every modal (saves, file browser, config screen,
  hotkey dialog, room/diagnostics panels) shares one themeable chrome:
  a bordered, titled, opaque frame with a clickable **✕**, mouse-clickable
  buttons, and an optional **drop-shadow**. The confirm button (OK / Save) is
  **underlined** and starts focused, so **Enter** triggers it; **Tab** / **Shift-Tab**
  (and **←** / **→** on the confirm dialogs) cycle focus through the other buttons
  (the focused one is highlighted) and Enter then fires whichever is focused. `Esc` and **✕** always close. Text-entry modals
  keep **Enter** = submit the field; the navigation panels (file browser, saves)
  keep their own keys and just show the default button underlined. Colors are
  configurable under the `[dialog]` surface section — `background`, `border` (the
  dialog's own frame), `title`, `button` / `button:active`, and `shadow` — and a
  modal's on-screen **placement** — centered (default) or anchored to any edge or
  corner with a margin — via `[dialog]`'s `placement` / `margin` keys.

### Editing your theme
The file *is* the editor. All visual settings live in a standalone `style.toml`,
referenced from `config.toml` by `style = "<name or path>"` (the single styling
source — `config.toml` carries no style of its own). On first run, if you have no
`style.toml`, lanthorn seeds one in your user directory **fully commented out**:
every selector is there, grouped by section (roles, panels, Glk styles, map,
debug, transcript rules, status bar), each with a short explanatory comment, and
every commented line already spelling out the built-in default — so the seeded
file is a working reference you edit in place, not a blank page. It never
overwrites an existing file. Uncomment the lines you want to change, save, and
run **`reload-style`** to see the change live (a syntax error keeps the current
look and warns you instead of crashing); flip `watch_style = true` in
`config.toml` (or run **`toggle-watch`**) and every save reloads on its own.
`style.example.toml` at the repo root is generated from the same registry, so it
always matches the seeded template.

**Per-game looks**: drop a `style.toml` into the game's own save directory
(`<data-base>/<story-key>.save/style.toml` — the same folder as its saves and
`map.txt`) to layer overrides on top of the global theme for just that game;
it's re-applied every time that story opens. There's no "Save Game" button —
you write the file directly.

**Per-game settings**: alongside that style file, a game's save directory can hold
its own `config.toml` — a separate, deliberately tiny sidecar carrying at most
`honor_game_colours`, `borderless_windows`, `show_map`, `v6_pixel_lock`,
`guidance`, `command_band`, `return_probe`, `pictures`, `v6_render` and
`interpreter_number`. It is written for you when you
toggle one of those for a story (`/set-game-colours`, `/set-game-borders`,
`/set-v6-pixel-lock`, `/set-guidance`, `/set-v6-render`, `/set-return-probe`,
hiding the map, opening
the command band — or clicking any of the toggle controls on the story pane's
border, which run exactly those commands), and it is a *sparse override layer*,
not a copy of your global config:
bare uncommented lines, only the keys that differ, and the file is deleted once
nothing is overridden. An absent key means "inherit the global value" — which is
why lanthorn never seeds the annotated template into a game directory, and why you
shouldn't either: every line you uncommented would become a per-game override
pinning that value for that story.

**Resolution order**, most specific first: an *explicit* user per-game slot →
a garglk per-stream override → the game's own live style hints → the
`glk.*` slot (global theme, defaults, and any shipped `garglk.ini`) → that
slot's role → your terminal colors. The `honor_game_colours` setting gates the
two game-driven layers (per-stream override and live style hints); turn it off
to have your theme own every color regardless of what the game requests — see
[interpreter](interpreter.md) for the game-colour toggle itself. An explicit
per-game slot always wins over the game, even with game colours honored.

**garglk.ini import**: if a `garglk.ini` (or `<story>.ini`) sits beside the
story, lanthorn reads the section matching that game and imports what a terminal
can honor — its `tcolor`/`gcolor`/`linkcolor`/`bordercolor`/`windowcolor`
palette, `stylehint` (→ `honor_game_colours`), the text-window margins
(`tmarginx`/`tmarginy`, converted from pixels to character cells with a nominal
8×16 cell), and the inter-window border width (`wborderx`/`wbordery` → the
borderless-windows toggle: `0` → borderless). Colours layer per the resolution
order above. The text margin and border toggle are applied at runtime — nothing
is written back to any sidecar — and, consistent with `honor_game_colours`, an
explicit per-game `config.toml` value always wins over the garglk.ini (the text
margin has no per-game key today, so garglk overrides only your global default).

**And we answer for it.** A game can ask the interpreter what colour it actually
paints a given style — and at least one game asks in order to find out whether
its own config file was applied. Kerkerkruip's ini sets `style_User2` to Fashion
Fuchsia (`tcolor 10 F400A1 ffffff`) for no reason except that nobody else on
earth would, then measures that style at startup; a host that answers "fuchsia"
must be running the author's config, so the game skips its screen-reader prompt,
switches its menus to hyperlinks and opens its graphical title screen. lanthorn
now reports the per-style colour it really renders, so the answer is honest —
which means **shipping the ini beside the story is the opt-in for that
presentation**. Keep `Kerkerkruip.ini` next to `Kerkerkruip.gblorb` and you get
the author's Gargoyle look; move it away and the game asks you its questions the
usual way. Nothing else to configure: the file's presence is the switch.

**Schema note (pre-release, breaking):** the `style.toml` schema described
above is new. An old-schema file (with top-level `[colors]` / `[symbols]`
sections) is left untouched — it is not auto-migrated or overwritten — but its
sections no longer apply; regenerate by deleting it and letting lanthorn
re-seed the new template, or hand-write the new shape from
`style.example.toml`.

## Configuration
- TOML config at `~/.lanthorn/config.toml` plus command-line flags
  (`--user-dir`, `--config`); CLI overrides the file, which overrides defaults.
- **The settings screen is Global Settings, and it means both words.** Every row
  on it holds a value that lives in the global `config.toml` — nothing on it is a
  button that merely *does* something, and nothing on it is a setting for the one
  story in front of you (per-game choices live on the pane borders and in the
  `config.toml` beside the game). Moving through the screen changes a working copy
  and touches nothing else: **Save** writes the file *and* applies the change to the
  session you are in, so sound, colours, margins, the status bar, the room numbers,
  the period look and the rest simply take effect. **Cancel** discards. A handful of
  settings genuinely cannot be applied to a game already running — `user_dir`
  resolved this story's save and map folders when it launched, `undo_levels` set the
  machine's undo cap, `interpreter_number` wrote header byte `$1E` — and each of
  those rows says *on next launch* in its own description rather than looking like
  it worked.
- **The config file documents itself.** On first run lanthorn seeds
  `config.toml` the same way it seeds `style.toml`: every setting it reads is
  listed, grouped and commented, with the value shown being the **default** — so
  the whole surface is browsable from the file instead of only from the source,
  and uncommenting a line as-is changes nothing. Where a default can't be written
  down (an unset path, or a value lanthorn picks per story) the line is marked as
  an example, because uncommenting *that* does change behaviour. An existing
  config is never overwritten, and later edits from the settings screen preserve
  your comments.
- **…and it keeps documenting itself.** Seeding happens once, so a config written
  a release ago would otherwise never learn about a setting invented since — and
  a setting you cannot see in your own file is a setting you cannot discover.
  Lanthorn appends what is missing, commented, at the end of the section it
  belongs to: nothing you wrote is touched, reordered or reformatted, running it
  again adds nothing, and a commented line changes nothing until you edit it.
  `adult_words` arrives uncommented, because that list is only a default rather
  than an invisible filter if you can read it. A file you emptied on purpose is
  left empty, a file that doesn't parse is left alone, and a line reading
  `# lanthorn: no-top-up` stops it for good.
- **A broken config file says so.** TOML is parsed as one document, so a single
  stray character — an unclosed quote, a stray bracket — costs you every setting
  in the file, not just the line it's on. The same is true of a value lanthorn
  can't use (`volume = 300`, `auto_load = "yes"`): the file is valid TOML, but
  the *config* isn't, and it is dropped just as wholesale. lanthorn names the
  file and shows the error at startup instead of quietly running on defaults,
  and it refuses to save settings over a file it couldn't read, so the text you
  need in order to find the mistake is never overwritten. Fix the file (or move
  it aside and let lanthorn seed a fresh one) and saving resumes.
- **Lanthorn's Guiding Light** — `guidance` (default `true`) is the one switch
  for everything lanthorn offers you *while you play*: the words this story's
  parser knows, a completed noun, a caution before a move that cannot be taken
  back. One switch rather than one per feature — a player who does not want the
  interpreter talking over the story should not have to enumerate five of them.
  `--guidance on|off` says it for a launch; `/set-guidance` (bare toggles, or say
  `on`/`off`) — and the Guiding Light's own `●`/`○` control in the story pane's
  bottom border — says it **for that story**, remembered in its own
  `config.toml` sidecar, because whether you want help is a standing preference
  about the game in front of you: off for the one you know by heart, on for the
  one you just opened. `/set-guidance auto` hands the story back to your global
  default. The **settings screen** sets that global default — the one new games
  inherit — which is where the one-line introduction above your first hint sends
  you.
  The **vocabulary offer** is the first of them: when a word in your command is
  not in the story's dictionary, the light names words that are — one keystroke
  away (`lanturn` → `lantern`), the same word with a different ending (`opening`
  → `open`), a form English inflects irregularly and no ending rule can reach
  (`lit` → `light`, `took` → `take`, `broke` → `break`), what your word MEANS when nothing about
  its shape can help (`illuminate` → `light`), or the story's own synonyms for a
  verb once it has one (`smell · sniff`). Never more than three, never a word the parser would refuse,
  and never a rewrite of what you typed: it is an offer, and the command you sent
  went to the game exactly as you wrote it. It works from the dictionary rather
  than from the game's reply, so it behaves the same on a story that words its
  refusal however it likes — and it says nothing at all unless it is confident,
  which is most turns.
  The table behind the third of those sources is generated, and
  `crates/verb-synonyms-gen/README.md` has the procedure for rebuilding it — and
  for checking the offers against a corpus — when new stories arrive.
  One of those sources answers to the adult list below and the rest
  never do: correcting your own word is your business, but proposing a *different*
  word from what yours means is lanthorn's own voice, and that half is filtered
  the way an unprompted panel is.
  And the offer is **tried before you see it**. `guidance_probe` (default `true`)
  forks your game into a silent throwaway copy, types each suggestion into it
  from exactly where you are standing, keeps only the ones that did something and
  throws every copy away — so the line reads `try instead — light` rather than
  `this story knows — light`, because it is a recommendation and not a lookup.
  Nothing the copy does reaches your screen, your saves or the game you are
  playing: sound and graphics are off in it, it may READ your game's own stored
  data and never write a byte of it, and a story that reaches for `@save` inside
  one is told the write failed. Reading is what makes it quick — a big Glulx game
  keeps a cache of its own startup work, and a copy that could not see it would
  sit through the whole initialisation your launch skipped.
  It also runs **out of the way**: the game answers you immediately and the
  suggestion appears a moment later, so nothing waits on it. On a heavy story
  that moment can be a second or two, and if you have already typed your next
  command by then the suggestion is simply dropped rather than printed under the
  wrong one. Turn it off and the offer still appears, in the modest wording it
  can still support.
  Two honest limits, because a vetted suggestion is evidence and not a promise:
  a game that draws on randomness can answer the copy and your game differently,
  and a refusal the probe's own control commands never provoke — "that's not
  something you can open" — reads as a success and survives.
  The **word reveal** is the same light pointed the other way. The offer can only
  help once the parser has already said no; click the `◈` on the story pane's
  bottom border (or run `/reveal-words`) and every noun, name or object *already
  on screen* that this story knows lights up for a few seconds, over the story's
  own prose, without moving a line of it. It goes out on your next keystroke, on
  your next turn, or on its own — one press, one look, and you are back in the
  game. It answers the oldest frustration in the genre: a room description names
  a dozen nouns and two of them are implemented, and until now the only way to
  find out which was to type at all twelve. Mini-Zork's opening screen names
  five — `field`, `house`, `door`, `mailbox`, `window` — and the story has never
  heard the word `field` at all, so that one stays dark while the rest light.
  The question it asks is **does one of your OBJECTS answer to this word**,
  asked of the story's own things first, and it says so in the corner every
  time: *words this story knows — not necessarily things that are here*. A
  description that mentions a sword sitting in the next room lights it, and that
  is the point rather than a leak — every word it touches is one the story has
  already printed on your own screen, so it can reveal nothing you have not been
  told. It used to walk the object tree wherever it could and light only what
  was within reach, which sounds stricter and read as broken: the engines that
  know the most lit the least, and Arthur's "imbedded in one of the knobs is a
  sliver of crystal" — a real object with a real use — lit nothing at all.
  It only lights a story's own nouns and adjectives — a real Zork I house
  fetches `white` right along with it — and never a verb, an article or a
  preposition: the command band already answers "what can I do", and this answers
  "what does this game know about". Glulx answers with its own objects too —
  the Inform object list is read straight out of Glulx memory, so *Dr Ludwig
  and the Devil* lights its devil and its summoning circle rather than `the`
  and `an`. Only Scott — and any Glulx image whose object list cannot be
  verified — falls back to the dictionary's own idea of a noun, a weaker
  guarantee: an Inform dictionary marks a word "usable in noun position" rather
  than "names a thing", so an article there can still slip through.
  One honest limit, and it is the parser's own: a Version 3 dictionary keeps six
  characters of a word, so `candle` and `candlesticks` are the same entry, and a
  room holding a candle lights both. That is not a mistake on lanthorn's part —
  `take candlesticks` really does take the candle — it is the game's own
  behaviour, shown. (The reveal reads the ordinary text screen, so it has nothing
  to light in v6 **raster** mode, where the story's text is a picture.)
  And when it cannot read a story's words at all it says *that*, rather than
  something it does not know: a game whose object names and dictionary flags are
  both out of reach — Dialog's output, or a story whose dictionary declares no
  entries — answers **lanthorn cannot read this story's words**. It used to
  answer "nothing on screen is a word this story takes", which is a claim about
  your room, and in that case a wrong one: the story takes plenty of the words in
  front of you and lanthorn simply cannot say which.
- **And the map can go looking for the way back.** `return_probe` (default
  `false`) forks the same silent copy after a move that leaves a gap in the map,
  walks one direction in it, and records the passage only if the copy comes out
  in the room you just left — closing the one-way gaps an automap is otherwise
  full of, without ever assuming a passage runs both ways. A probe that lands
  anywhere else records nothing at all, not even that the room exists. It is off
  by default because it runs your game a few extra turns in private; the
  footprint on the story pane's bottom border — beside the map toggle, where it
  stays reachable with the map hidden, because the search keeps running either
  way — turns it on, `/set-return-probe` does it from the keyboard, and both
  remember the answer for that story. See
  [mapping](mapping.md) for what it does to the map.
- **A choice for one run stays a choice for one run.** `--sound off`, `--user-dir`,
  `--game-colours off` and `--interpreter` are instructions for the launch you typed
  them on,
  and so are the things lanthorn works out for itself: an interpreter number this
  game's own sidecar pins, a `garglk.ini` sitting beside the story, a `/game-colours`
  choice, or two-colour artwork that has no colours to give and so switches
  `honor_game_colours` off for that one rendition. None of them can reach
  `config.toml`. That matters because saving settings is not always something you
  set out to do — the story picker's "remember this directory?" prompt writes the
  file too, and before this a single `--sound off` session was enough to leave every
  later launch silent with nothing on screen to say why. The rule is one line: while
  a value is still the one that launch handed it, the file keeps whatever *it* said.
  Change the setting on the settings screen and it becomes yours from then on —
  including when you change it to exactly what the flag asked for.
- **Settings are written atomically.** Every file lanthorn owns — `config.toml`,
  saves and archives, the aux/VFS sidecars — is built beside its target and moved
  into place in one step, so a crash, a power cut, or a kill during a write leaves
  the previous file intact rather than a truncated one.
- **Default story directory** — `default_story_dir` is opened when lanthorn is
  launched with no path argument. The first time you point lanthorn at a
  directory on the command line without one set, it offers to remember that
  directory as the default (writing it to the config file); after that, a bare
  `lanthorn` opens the story picker there. With no argument and no default set,
  lanthorn prints how to fix it and exits.
- **Virtual screen size** — `virtual_screen_cols` / `virtual_screen_rows` pin the
  screen dimensions reported to the game. Leave them **unset** (the default) and
  lanthorn reports the story pane's real measured size and re-reports it whenever
  you resize the terminal, so a v4+ game's cursor-addressed forms and status
  displays fill the pane and line up with the prose. Set one to reproduce a game's
  original fixed layout (say `virtual_screen_cols = 80`) — a pinned width narrower
  than the pane is drawn centred, and a pinned width wider than it scrolls to
  follow the cursor. Version 6 stories ignore both: they lay out on their own
  fixed pixel screen, which lanthorn scales into whatever pane it has.
- `undo_levels` (default 16) — how many in-memory undo states the Z-machine
  keeps for the game's own UNDO command (0 disables undo).
- **Random seed** — `random_seed` pins the number every engine's random-number
  generator starts from. Leave it **unset** (the default) and lanthorn draws a
  fresh seed from the system at each launch, so the dice are different every
  time you sit down — which is the whole point of a game like *Kerkerkruip*,
  whose dungeon is dealt at the start. Set it and the story becomes a recording:
  the same shuffles, the same rolls, the same monsters in the same rooms, every
  run. lanthorn prints the seed it used on the console as it starts —
  `lanthorn: random seed 3735928559 (set random_seed = 3735928559 to replay this
  run)` — so when a run turns out to be the good one, that number is how you ask
  for it again, and how you hand it to somebody else. A restart (`@restart`, or
  restarting from the menu) re-draws the seed the same way the launch did: pinned
  means the same game back, unpinned means a new one. A game that asks the
  interpreter for entropy *itself* — Glulx's `setrandom 0` — still gets it, seed
  or no seed; the spec says it must, and almost nothing does.
- **Command band** — the `[command_band]` section configures the point-and-click
  phrase builder (see [Interface](interface.md#playing-aids); not to be confused
  with the unrelated top-level `command_bar` boolean, which moves the *typed*
  prompt into a persistent bar). `height` (default 5) is the band's rows — it
  draws no frame, so every one of them is content — clamped to 3–11 and to
  whatever the screen can spare;
  resize mode writes this key. `auto_open` (default false) opens the band with
  the story.

  The VERB column normally needs no configuring at all: it is read from the
  running story's own grammar table. The two keys are for when you want
  something else. `verbs` REPLACES the whole column — the story's grammar
  included — and `extra_verbs` ADDS to whatever is in force, which is usually
  that grammar, so it patches a real verb list rather than a constant. Same
  entry shape either way, `{ word = "unlock", arity = "pair", prep = "with" }`,
  where `arity` is one of `solo` (complete on its own), `object` (one object,
  required), `object_opt` (one object, optional) or `pair` (two objects joined
  by `prep`, which also names that column). An `extra_verbs` entry whose word
  the story already has re-shapes that one verb rather than duplicating it. An
  unrecognised `arity` is reported in the transcript and that entry is skipped,
  never silently reinterpreted.

  Both keys mean exactly what they meant before the column had a story behind
  it, so a config written against the old built-in table still does what it
  always did — `verbs` was a complete statement of what you wanted offered, and
  folding two hundred of the story's own verbs in beside your twelve would
  destroy the only thing the key is for. A column filled from `verbs` labels
  itself **VERB — yours**, and the built-in fallback (for a story whose grammar
  cannot be read) labels itself **VERB — generic**; the story's own grammar is
  the one that goes unlabelled. `quick` replaces the one-click quick-action row,
  which is not read from the grammar — the compass is not in the verb table on
  the Infocom family at all.
- **The adult list** — `hide_adult_words` (default `true`) and `adult_words`, both
  **top level**, not part of `[command_band]`. Infocom's dictionaries are saltier
  than their prose — Zork I's verb table holds `fuck`, `shit`, `rape` and
  `molest` — and now that the VERB column is the story's real grammar, a panel
  lists the lot to anyone who opens it. `hide_adult_words` keeps the words in
  `adult_words` out of any panel that enumerates a story's vocabulary unprompted.

  The default list is written out in your `config.toml`, **uncommented** — in a
  config lanthorn seeds for you, and appended to one you already had, since a
  list you cannot read is not a default at all — and is deliberately the strong
  end only:

  ```toml
  adult_words = ["fuck", "fucked", "fucking", "shit", "cunt", "cum", "wank", "bastard", "bitch", "asshole", "whore", "slut", "rape", "molest"]
  ```

  `damn` and `barf` are Infocom being Infocom and stay visible; so do `hell`,
  `crap`, `screw`, `suck`, `piss`, `pee` and `sod`. `rape` and `molest` are not
  swearing at all — they are on the list because a panel listing them unbidden is
  worse than any expletive. Matching is whole-word and case-insensitive, never by
  prefix: old dictionaries truncate (a v6 story's four-character keys hold `bast`
  for *bastard*), and a prefix rule wide enough to catch those would also eat the
  real verbs `rap` and `who`. Add the truncations you want gone to your own list.

  **Two switches, and either one turns it off.** `hide_adult_words = false`
  restores the full column *and keeps the words*, so turning it back on needs no
  retyping; `adult_words = []` does the same from the other end. The settings
  screen flips the boolean. Shortening or extending the line changes what counts.

  **It filters what lanthorn says unbidden, never what you reached for.** Every
  word taken out is still a word the story knows: typing it parses exactly as it
  always did, and the Guiding Light still offers it when you reach for a word
  close to it — mistype `molst` and you are told the story knows `molest`,
  because that is your word, corrected, not ours. The one place the list does
  reach the Light is where it proposes a *different* word from what yours means:
  Zork I answers `sod` with `fuck · shit · damn` on its own initiative, and with
  the list on you get `damn`. Nothing here touches what the parser accepts, what
  the synonym data holds, or what a game prints.

  **There is no key for Infocom's test rig, and that is deliberate.** Verbs
  beginning `#` or `$` — `#record`, `#command`, `#random`, `$verify` — are kept
  out of the VERB column by a rule about the sigil, not by a list of words. The
  adult list is a *judgement*, so it ships written out where you can read and
  edit it; the sigil rule is *structure*, there is nothing to disagree with, and
  folding the two together would make both harder to reason about. It is
  display-only in exactly the same way: typing `$verify` still works.
- **v6 story rendering** — `v6_render` selects how graphical v6 titles (*Zork Zero*,
  *Shogun*, …) draw their story pane on an image-capable terminal: `hybrid`
  (the default) keeps the story text as real terminal text inside an image
  chrome ring; `raster` bakes the whole pane — frame, status, and story text —
  into one scaled pixel image instead; `extended` is raster at a whole
  magnification with the frame grown downward, so the pane's surplus height
  becomes extra rows of prose rather than empty margin. (A fourth mode,
  `frameless`, was removed —
  a config still naming it silently reads as `hybrid`.) It also cycles
  in the settings screen, and `/set-v6-render` switches modes live mid-game —
  remembered *for that story*, not for every story, so you can keep one game on
  raster and another on hybrid. (Applies only to graphical v6 stories;
  other games are unaffected.) See [Graphical v6](v6-graphics.md) for the full
  picture.
- **Fusing an EGA dither** — `fuse_art_dither` (default `true`) blends the colour
  dither in a 640-wide EGA plate, the way the card blended it for you. EGA's
  sixteen colours were soldered in, so its artists made the missing ones by
  alternating two they had, column by column — *Zork Zero*'s bronze arch is brown
  against bright red — and since those columns were half as wide as an MCGA
  pixel, the screen fused each pair into a colour the palette never held.
  lanthorn keeps all 640 columns, so it does the fusing itself. Set it `false` to
  see the archive's own pixels instead, every column distinct. It changes nothing
  else: two-colour CGA line art is never fused either way (blurring line art only
  makes grey), and 320-wide MCGA and Amiga art has no dither at this frequency to
  fuse. See [Graphical v6](v6-graphics.md#the-colours-come-with-the-card).
- **v6 arrow keys** — `v6_arrow_keys` (default `false`) controls whether arrow
  keypresses reach a v6 story as movement input. Off by default, so arrows go on
  driving lanthorn's own scrollback recall and map panning the way they do in
  every other story; set it `true` (in config.toml or the settings screen) to hand
  them to a game that binds arrows to movement. Only v6 stories are affected —
  v1-5 and Glulx games always get arrows, and so do v6 menus and "press any key"
  screens either way. See [Graphical v6](v6-graphics.md#arrow-keys-movement-or-map-panning-your-call).
- **Locking v6 art to whole pixels** — `v6_pixel_lock` (default `false`) snaps the
  v6 letterbox magnification to a rung of a ladder on which one *art* pixel is
  always a whole number of device pixels, instead of scaling by whatever fraction
  fills your pane. Artwork comes out nearest-neighbour crisp, a resampled edge stops
  landing half a pixel off the font glyph beside it, and every tiled side border
  repeats on an exact boundary. The rungs are derived from the artwork you mounted,
  not from a fixed list: a 320-wide rendition (Blorb, Amiga, MCGA) goes 0.5×, 1×,
  1.5×, 2× …, while the standard Macintosh's monochrome plate and the 640-wide
  EGA/CGA ones go 1×, 2×, 3× … The cost is screen area — the picture stops at the
  rung below your pane rather than filling it, so the margin around it (painted with
  the story's own page) gets wider. A pane too small for even the smallest rung
  quietly falls back to free scaling, and on the **half-blocks** backend the switch
  does nothing at all — half-blocks paints coloured cells rather than pixels, so
  there is no device pixel for an art pixel to land a whole number of. `/dump-terminal`
  says so in those words. Whether the trade is worth it depends on the
  press you mounted, so it is settled **per game**: `/set-v6-pixel-lock` toggles it
  live (`on` / `off` to say so outright, `auto` to go back to inheriting this global
  key) and remembers the answer in that story's own sidecar, below — your global
  `config.toml` is never written by it. See
  [Graphical v6](v6-graphics.md#v6_pixel_lock--a-whole-number-of-device-pixels-per-art-pixel).
- **Which boot medium answers for a system typeface** — `system_font_disk`
  (default empty) picks between the boot media you keep in `~/.lanthorn/` when
  more than one carries the face a machine is asking for. Drop a Mac OS System
  startup disk or an Amiga **Kickstart ROM** (`*.rom`) in there and a Version 6
  game off that machine's own media is drawn with the
  typeface the machine really used — Geneva on a Macintosh, which lives in the
  System file and on no Infocom disk, and topaz 8 on an Amiga, which lives in
  Kickstart and on no floppy at all (a Workbench drawer carries topaz **11** and
  six display faces nobody's interpreter drew with). Nothing is shipped and
  nothing is copied; the media stay yours, the same arrangement `stories/` runs
  on, and with none there the built-in face answers exactly as before. This key
  is only a **tiebreak**: every medium of the right kind is read and the faces
  pool together, so a case-insensitive piece of a filename (`"6.0.8"` finds the
  System 6.0.8 startup disk in a folder holding System 6 and 7, `"Kick"`
  promotes your ROM) moves that one to the front without excluding any other —
  name a file that
  lacks the face and the rest still answer. Worth setting when two disks carry
  the same face from different releases of the operating system, since a System 7
  Geneva is not the 1988 one; not worth setting otherwise. The picker's info
  panel lists every face found, grouped by the medium
  it came off. See
  [Graphical v6](v6-graphics.md#your-own-boot-disk-your-machines-own-typeface).
- **Story text margins** — `text_margin_x` / `text_margin_y` (default 0) reserve
  blank columns on each side / rows top and bottom *inside* the story text pane,
  for a little breathing room around the transcript. The margin applies to the
  text buffer only — the upper-window status line stays flush — and adjusts in
  the settings screen with `←` / `→`. A game's imported `garglk.ini` margin (below)
  overrides this default while that story is open.
- **In-app config screen** — pop the leader panel (default `Ctrl+P`) and press
  the `open-settings` key for a global-settings modal covering the common options, with an
  explicit Save (writes the config file, comments and layout preserved) and
  Cancel; changes apply live.
- **Portable home** — everything lanthorn keeps (config, style, saves, sidecars)
  lives under `~/.lanthorn` by default; point `--user-dir` somewhere else to
  relocate the whole home, or `--data-dir` to split just the saves and sidecars
  off on their own.
