<!-- generated from crates/app/src/config_template.rs (config_template::GROUPS) by docs_reference; do not edit by hand -->
# Config reference

Every setting `~/.lanthorn/config.toml` accepts, grouped the way the seeded template groups them. "example" means the default cannot be written down (unset/computed) and the value shown only illustrates the shape; "live default" means the setting ships uncommented because it is content rather than documentation.

## Startup and files

| Key | Default | Note | Description |
|---|---|---|---|
| `user_dir` | `"~/.lanthorn"` | example | Root directory for lanthorn data (maps/, saves/, style.toml). Default: ~/.lanthorn. |
| `default_story_dir` | `"~/games/if"` | example | Directory (or single story file) opened when lanthorn is launched with no path argument. Unset by default, so a path is required. |
| `style` | `"style.toml"` | example | Style-file pointer: a built-in name or a file path. Unset uses <user_dir>/style.toml when present, else the built-in theme. |
| `watch_style` | `false` |  | Watch the resolved style file and live-reload it on change. |

## Saving and undo

| Key | Default | Note | Description |
|---|---|---|---|
| `auto_load` | `true` |  | Restore game state from the archive on startup so play resumes where it left off. Set false to start fresh while keeping the accumulated map. |
| `auto_save` | `false` |  | Save the archive after every turn, on top of the exit-save and Ctrl+S. |
| `prompt_save_on_quit` | `true` |  | When auto_save is off, offer to save on quit. |
| `prompt_load_on_launch` | `true` |  | When auto_load is off, offer to resume a save found on launch. |
| `record_turn_history` | `false` |  | Record a per-turn rewind/replay history into the archive. Opt-in: it grows the archive and keeps per-turn blobs in memory. |
| `history_turns` | `500` |  | How many of the most recent turns record_turn_history retains before evicting the oldest. Bounds memory on a long session; no 0 = unbounded. |
| `undo_levels` | `16` |  | Undo depth: retained in-memory snapshots. 0 disables undo. |
| `aux_storage` | `"ask"` |  | Where v5 auxiliary save data goes: "ask" (default), "archive", "global". |

## Interface

| Key | Default | Note | Description |
|---|---|---|---|
| `mouse` | `true` |  | Capture the mouse: click-to-select in the browser and map, wheel scrolling, and Glk mouse input for games that ask for it. |
| `mouse_wheel_invert` | `false` |  | Invert wheel direction, for terminals reporting "natural" scrolling. |
| `command_bar` | `false` |  | Type into a persistent command bar instead of the inline story prompt. (Unrelated to the [command_panel] section further down, which is the point-and-click phrase builder.) |
| `command_prefix` | `"/"` |  | The character that routes a line to a slash command. |
| `guidance` | `true` |  | Lanthorn's Guiding Light: help offered while you play — the words the parser knows, a completed noun, a caution before a move that cannot be undone. Marked in the margin with its own glyph rather than in the text; "gutter.assist" in style.toml sets the mark. False for silence. |
| `guidance_probe` | `true` |  | Before offering a word, try it in a silent throwaway copy of the game and keep only what actually did something — so the light recommends rather than merely lists. The copy runs out of the way, on its own thread: the game answers you at once and the suggestion follows a beat later, or is dropped if you have already typed again. It may READ your game's own stored data and never writes a byte of it, and nothing it does reaches the screen, your saves or the game you are playing. False still offers, more modestly. |
| `return_probe` | `true` |  | After a move, look for the way BACK in a silent throwaway copy of the game, and put it on the map when it is found. Automaps otherwise learn passages one direction at a time, and the honest alternative — assuming the way back is the way you came — is wrong often enough in these games to be worse than the gap. Nothing is recorded unless the copy actually comes out in the room you left: a probe that lands somewhere else records nothing at all, and neither does one that finds no way back.  On by default: it runs your game a few extra turns in private after every move that opens a gap, and never touches your screen or saves. The footprint on the STORY pane's bottom border switches it — beside the map toggle, since the search keeps running with the map hidden — and "/set-return-probe" persists it per-game. |
| `hide_adult_words` | `true` |  | Keep the words below out of any panel that ENUMERATES a story's vocabulary unprompted — the command panel's VERB column and its like. Infocom's dictionaries are saltier than their prose, and a panel puts the whole lot in front of anyone who opens it.  DISPLAY ONLY. The story still knows every word: typing one parses exactly as it always did, and Lanthorn's Guiding Light still offers it when you reach for it. False shows the full column and keeps the list. |
| `adult_words` | `["fuck", "fucked", "fucking", "shit", "cunt", "cum", "wank", "bastard", "bitch", "asshole", "whore", "slut", "rape", "molest"]` | live default | …and these are the words. Written out, uncommented, and yours: shorten it, extend it, or set it to [] to hide nothing. It is deliberately the strong end only — `damn`, `barf`, `hell`, `crap` and `piss` are Infocom being Infocom and stay visible. `rape` and `molest` are not swearing at all; they are here because a panel listing them unbidden is worse.  Matched whole and case-insensitively, never by prefix — old dictionaries truncate, and a prefix rule wide enough to catch `bast` would also eat the real verbs `rap` and `who`. |
| `show_status_bar` | `true` |  | Show the status/score bar across the top of the story pane. |
| `show_room_numbers` | `false` |  | Show room numbers (#id, or a small ordinal for a name-only room) inside Boxes-zoom room boxes. |
| `split_ratio` | `50` |  | The story pane's share of the story/map split, as a percentage. |
| `inv_dock_pct` | `33` |  | Inventory panel height cap, as a percentage of screen height. |
| `room_dock_pct` | `33` |  | Room panel height, as a percentage of screen height. The panel docks at the bottom of the map pane and describes the room you are in (or the one you clicked). |
| `text_margin_x` | `0` |  | Blank columns reserved inside each side of the transcript window. |
| `text_margin_y` | `0` |  | Blank rows reserved above and below the transcript text. |
| `background_tidy` | `"every_room"` |  | Automatic map re-tidy when new rooms appear: "off", "every_room" (default), "on_overlap", "debounced". |
| `hint_skip_screen_warning` | `true` |  | Auto-skip the InvisiClues "your screen is only N characters wide…" banner and land on the topic menu. Set false to see and dismiss it yourself. |

## Interpreter behaviour

| Key | Default | Note | Description |
|---|---|---|---|
| `honor_game_colours` | `true` |  | Honour game-set colours. Set false to use only the configured theme. Override for a single run with `lanthorn --game-colours on\|off`. |
| `system_colours` | `false` |  | Advertise a named machine's own default page and ink ($2C/$2D) even when the story did not come off its original media. Automatic off a release disk — that is what the disk means — so this is only for a machine you named yourself with interpreter_number, on a story that did not come off one. It cannot conjure a machine where none was named, so an ordinary story file always gets your terminal's own colours. |
| `period_look` | `true` |  | Paint a v1-v4 story the way its own machine's interpreter did: that machine's page and ink, its status band, and the shape and colour of its cursor. Measured off emulator captures of the release disks, so it applies only when the story came off one (or you named --interpreter). Narrower than honor_game_colours, which takes this with it when off. A colour you set yourself in style.toml always wins. |
| `honor_timed_input` | `true` |  | Honour the Z-machine's timed input. Set false to treat every read as untimed. |
| `interpreter_number` | `6` | example | Interpreter number advertised in header byte 0x1E. Games branch on it: Beyond Zork picks character graphics over colour on IBM PC, and several v6 story files were built for one specific machine. ZMSD §11.1.3's values:     1  DECSystem-20      7  Commodore 128    2  Apple IIe         8  Commodore 64    3  Macintosh         9  Apple IIc    4  Amiga            10  Apple IIgs    5  Atari ST         11  Tandy Color    6  IBM PC  Unset auto-selects: 6 (IBM PC) for v6, else 1 (DECSystem-20). Override for a single run with `lanthorn --interpreter N`. |
| `random_seed` | `20250811` | example | Pin the seed every engine's random-number generator starts from, so a story replays exactly: the same shuffles, the same dice, the same dungeon. Unset (default) draws a fresh seed from the system at every launch, which is what makes a randomised game like Kerkerkruip a different game twice. lanthorn prints the seed it used on the console as it starts — copy that number in here to play that run again. A game that asks the interpreter for entropy itself (Glulx setrandom 0) still gets it, seed or no seed. |
| `glk_pixel_scale` | `"native"` |  | How the pixel sizes a Glulx game asks for are mapped to your terminal. A Glk game sizes its graphics windows in pixels chosen against a normal screen (advent.blb wants a 36px toolbar), so on a HiDPI display or with a large font that request buys fewer text rows and the game's artwork shrinks against the text beside it.   "native" — report your terminal's real cell size (default). Safe for              every game, since it moves nobody's pixel constants.   "auto"   — report a 14px-tall cell whatever your font is, so artwork              scales WITH your text. Helps games whose art is too small              (advent.blb's toolbar); can SHRINK games whose art is sized              for a big screen (Counterfeit Monkey's map sidebar).   2, 3…    — divide your cell size by this instead Only affects Glulx: v6 Z-machine and Scott Adams lay out on their own fixed canvas, which lanthorn scales into the pane already. |
| `v6_render` | `"hybrid"` |  | How v6 graphical games (Zork Zero, Arthur, Journey, Shogun) are drawn:   "hybrid"   — crisp terminal story inside a scaled pixel frame (default)   "raster"   — the whole pane as one pixel image, letterboxed   "extended" — the same pixel image, but pinned to a whole magnification                and grown DOWNWARD: the pane's surplus height becomes extra                story rows in the game's own bitmap typeface instead of                empty letterbox. The game is told nothing — its own screen                keeps the layout it always had, at the top of a taller frame. |
| `fuse_art_dither` | `true` |  | Fuse the colour dither in a 640-wide EGA plate, the way the card did. EGA's sixteen colours were fixed in the silicon, so its artists dithered for the ones they lacked — Zork Zero's bronze arch is brown and bright red alternating column by column — and on a 640x200 screen those half-width columns blended in the eye into a colour the palette never held. lanthorn keeps all 640 columns, so it does the blending itself. Set false to see the archive's own pixels, dither and all. CGA line art is never fused either way, and 320-wide MCGA and Amiga art has no dither at this frequency to fuse. |
| `v6_arrow_keys` | `false` |  | Forward arrow keys to a v6 story as ZSCII 129-132, for a game that binds them to movement. Off by default, so arrows keep driving scrollback and map panning the way they do in every other story. v1-5 and Glulx always get them, and so do v6 menus and "press any key" screens either way. |
| `system_font_disk` | `""` |  | Which of your own boot media under ~/.lanthorn/ answers first when several carry the machine's system typeface. A case-insensitive piece of the file's name — "6.0.8" picks the System 6.0.8 startup disk out of a folder holding System 6 and 7 — and empty means no preference. It only breaks a tie. Every medium of the right kind is read and the faces pool together, so a file named here that does not carry the face being asked for falls through to the others rather than losing it; with no preference the pool is ordered by filename. Drop a Mac OS System disk or an Amiga Kickstart ROM (*.rom) in ~/.lanthorn/ and a Version 6 game off that machine's own media is drawn with the face the machine really used — Geneva on a Macintosh, which lives in the System file and on no Infocom disk, and topaz 8 on an Amiga, which lives in Kickstart and on no floppy at all. Nothing is shipped and nothing is copied; the media stay yours. With none there, the built-in face answers exactly as before. |
| `v6_pixel_lock` | `false` |  | Lock the v6 picture to whole device pixels per ART pixel, instead of scaling it to fill the pane at whatever fraction fits. Arbitrary fractional scaling is what softens the artwork and leaves seams where a resampled edge meets a font glyph; a locked magnification is nearest-neighbour exact, and it makes every tiled side border repeat on an exact boundary too. The steps come from the artwork itself, not from a fixed list: a 320-wide rendition (Blorb, Amiga, MCGA) goes 0.5x, 1x, 1.5x, 2x…, while the standard Macintosh's mono plate and the 640-wide EGA/CGA ones go 1x, 2x, 3x… The screen is centred in the pane and the margin around it carries the story's own page, as it already does. The cost is screen area — the picture stops at the rung below the pane rather than filling it. A pane too small for even the smallest step falls back to free scaling. |
| `virtual_screen_cols` | `80` | example | Override the screen size reported to the Z-machine in header bytes $21/$20. Unset (default) follows the story pane, which is what ZMSD §8.4 asks for. Set both only to pin a fixed virtual screen. |
| `virtual_screen_rows` | `24` | example | The height half of the same override; set it alongside the width. |

## Sound

| Key | Default | Note | Description |
|---|---|---|---|
| `enable_sound` | `true` |  | Play audio for sound_effect: bleeps and Blorb samples. |
| `volume` | `100` |  | Master volume 0-100, combined with the game's own per-sound scale. |

## Transcript search

| Key | Default | Note | Description |
|---|---|---|---|
| `search.start_backward` | `true` |  | A new /search starts backward, from the most recent match. |
| `search.key_back` | `"n"` |  | Key that steps to an older match. |
| `search.key_forward` | `"N"` |  | Key that steps to a newer match. |

## Animation

| Key | Default | Note | Description |
|---|---|---|---|
| `animation.enabled` | `true` |  | Master switch. False makes every animation instant. |
| `animation.easing` | `"ease-out"` |  | Easing curve: "linear", "ease-in", "ease-out", "ease-in-out". |
| `animation.scroll_ms` | `120` |  | Smooth-scroll duration in milliseconds. 0 is instant. |
| `animation.scrollbar_hide_ms` | `1500` |  | How long the STORY PANE's scrollbar stays up after you scroll it, in milliseconds. 0 keeps it up permanently. Only the story pane auto-hides - a modal's bar is reserved out of its content width, so hiding it there would reflow the list. |
| `animation.scrollbar_fade_ms` | `300` |  | Fade-out time for that bar once the delay expires. 0 pops it. |

## Command panel

| Key | Default | Note | Description |
|---|---|---|---|
| `command_panel.height` | `5` |  | Rows the band occupies. It has no frame (SQ-0667) - every row here is content. Clamped to 3-11, and to whatever the screen can spare. Resize mode (the band is one of its targets while open) writes this key. |
| `command_panel.auto_open` | `false` |  | Open the command panel as soon as the story starts. |
| `command_panel.verbs` | `[ { word = "unlock", arity = "pair", prep = "with" }, { word = "polish", arity = "object" } ]` | example | REPLACE the whole VERB column, including the running story's own grammar - which is where the column comes from when this is unset, and normally where you want it to come from. Each entry declares the verb's shape, which decides the columns offered after it is picked:     arity = "solo"        look, wait, n/s/e/w - complete on its own    arity = "object"      take, open, read    - one object, required    arity = "object_opt"  search, push        - one object, optional    arity = "pair"        unlock ... with ... - two, joined by `prep`  `prep` is the preposition a pair verb joins its objects with, and is shown as that column's header. A column filled from this key labels itself "VERB - yours"; unset it to get the story's own verbs back. |
| `command_panel.extra_verbs` | `[ { word = "xyzzy", arity = "solo" } ]` | example | ADDITIVE form of the same shape: layered on whichever table is in force - usually the story's own grammar, so this patches a real verb list. An entry whose word is already there re-shapes it instead of duplicating, so this is also how you fix one verb's shape. |
| `command_panel.quick` | `["n", "s", "e", "w", "ne", "nw", "se", "sw", "up", "down", "in", "out", "look", "inventory", "wait", "again"]` | example | The one-click quick-action words. The compass words draw as a rose; the rest flow beside it (a narrow band falls back to a single row). The value shown is the built-in row; unset the key to keep it. Picking one SENDS it AT ONCE - no Enter, unlike every other pick in the band (which composes onto the story input line and waits for Enter, same as typing). These words are also left out of the VERB column, since showing them twice would be redundant. |

## Key bindings

| Key | Default | Note | Description |
|---|---|---|---|
| `keymap.use_defaults` | `true` |  | Keep the built-in bindings and layer your overrides on top. False starts from an empty keymap, so only what you bind below works. |

