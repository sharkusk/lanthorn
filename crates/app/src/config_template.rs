//! Commented `config.toml` template (SQ-0573).
//!
//! Mirrors what `style.toml` already does (see [`crate::theme::template`]): emit
//! EVERY setting, each with a short comment, and leave the line commented out when
//! it holds the default — so the file is a browsable catalogue of what lanthorn can
//! be told to do, and uncommenting a line is a no-op until you change its value.
//!
//! Before this, [`crate::config::write_config`] only wrote keys it had a value for,
//! so a setting nobody had touched simply did not appear. `interpreter_number` was
//! the case that surfaced it: a real, useful knob that was invisible unless you read
//! the source.
//!
//! Three kinds of line, distinguished because only some of them are safe to
//! blanket uncomment:
//!
//! * [`Line::Default`] — the value shown IS the default. Uncommenting reproduces
//!   current behaviour exactly (the `template_default_lines_are_really_the_defaults`
//!   test proves it key by key).
//! * [`Line::Example`] — the default cannot be written down (`None`, or a computed
//!   path), so the value is an illustration. Uncommenting it CHANGES behaviour.
//! * [`Line::Live`] — the value is the default AND the line ships uncommented,
//!   because the setting is only a default if the player can see and edit what it
//!   holds. `adult_words` is the one (SQ-1122); the same test proves its value.
//!
//! Section headers (`[search]`, `[animation]`, `[keymap]`) are emitted UNCOMMENTED,
//! matching style.toml, and the schema `version` stamp is a live key: everything that
//! is actually a *setting* stays commented.
//!
//! Scope: this is the GLOBAL config only (`<user_dir>/config.toml`). A game's own
//! save directory can hold a second, unrelated `config.toml` — the sparse per-game
//! override sidecar in [`crate::styles`], carrying whatever a per-game control or a
//! `set-*` command can pin for one story — the list is
//! [`crate::styles::PerGameConfig::KEYS`], never a copy of it — as bare uncommented
//! lines, and deleted when empty. It is deliberately NOT templated: an absent key
//! there means "inherit the global value", so seeding defaults into it would turn
//! every one of them into a per-game override. [`auto_seed`] is only ever called
//! with the user dir.
//!
//! [`auto_seed`] writes the template on first run and never overwrites an existing
//! file, exactly like the style seed. `write_config` still owns runtime edits and
//! stays format-preserving, so a seeded file keeps its comments as settings change.
//!
//! Which leaves the file a player already has: seeded once and never re-seeded, and
//! only ever UPDATED key by key, it never gains a setting invented after they
//! installed. [`top_up`] closes that — it appends the documented settings a file has
//! never held, touching nothing that is already there (SQ-1129).

#[cfg(test)]
use crate::config::Config;
use crate::config::CONFIG_SCHEMA_VERSION;

/// Whether a template line's value reproduces the default or merely illustrates the
/// shape. See the module docs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Line {
    /// The value shown is the default; uncommenting changes nothing.
    Default,
    /// The default is unrepresentable (`None`/computed); the value is an example and
    /// uncommenting it changes behaviour.
    Example,
    /// The value is the default, and the line is written LIVE — uncommented — so
    /// the file states it as content rather than as documentation.
    ///
    /// One setting uses this: `adult_words` (SQ-1122). What it holds is words
    /// lanthorn declines to enumerate unprompted, and a filter nobody can inspect
    /// is censorship where one written out in the player's own config file is a
    /// default. Writing it live means it is edited, not first uncommented.
    Live,
}

/// One documented setting: TOML key, the literal to show, whether that literal is
/// the real default, and the comment lines above it (no leading `#`).
pub(crate) struct Row {
    pub(crate) key: &'static str,
    pub(crate) value: &'static str,
    pub(crate) line: Line,
    pub(crate) doc: &'static [&'static str],
}

const fn d(key: &'static str, value: &'static str, doc: &'static [&'static str]) -> Row {
    Row { key, value, line: Line::Default, doc }
}
const fn ex(key: &'static str, value: &'static str, doc: &'static [&'static str]) -> Row {
    Row { key, value, line: Line::Example, doc }
}
const fn live(key: &'static str, value: &'static str, doc: &'static [&'static str]) -> Row {
    Row { key, value, line: Line::Live, doc }
}

/// A group of settings under a banner comment. `table` names the TOML table the rows
/// belong to (`Some("[search]")`), or `None` for top-level keys.
pub(crate) struct Group {
    pub(crate) banner: &'static str,
    pub(crate) table: Option<&'static str>,
    pub(crate) rows: &'static [Row],
}

const STARTUP: &[Row] = &[
    ex(
        "user_dir",
        "\"~/.lanthorn\"",
        &["Root directory for lanthorn data (maps/, saves/, style.toml).", "Default: ~/.lanthorn."],
    ),
    ex(
        "default_story_dir",
        "\"~/games/if\"",
        &[
            "Directory (or single story file) opened when lanthorn is launched with",
            "no path argument. Unset by default, so a path is required.",
        ],
    ),
    ex(
        "style",
        "\"style.toml\"",
        &[
            "Style-file pointer: a built-in name or a file path. Unset uses",
            "<user_dir>/style.toml when present, else the built-in theme.",
        ],
    ),
    d("watch_style", "false", &["Watch the resolved style file and live-reload it on change."]),
];

const SAVES: &[Row] = &[
    d(
        "auto_load",
        "true",
        &[
            "Restore game state from the archive on startup so play resumes where it",
            "left off. Set false to start fresh while keeping the accumulated map.",
        ],
    ),
    d(
        "auto_save",
        "false",
        &["Save the archive after every turn, on top of the exit-save and Ctrl+S."],
    ),
    d("prompt_save_on_quit", "true", &["When auto_save is off, offer to save on quit."]),
    d("prompt_load_on_launch", "true", &["When auto_load is off, offer to resume a save found on launch."]),
    d(
        "record_turn_history",
        "false",
        &[
            "Record a per-turn rewind/replay history into the archive. Opt-in: it grows",
            "the archive and keeps per-turn blobs in memory.",
        ],
    ),
    d(
        "history_turns",
        "500",
        &[
            "How many of the most recent turns record_turn_history retains before",
            "evicting the oldest. Bounds memory on a long session; no 0 = unbounded.",
        ],
    ),
    d("undo_levels", "16", &["Undo depth: retained in-memory snapshots. 0 disables undo."]),
    d(
        "aux_storage",
        "\"ask\"",
        &["Where v5 auxiliary save data goes: \"ask\" (default), \"archive\", \"global\"."],
    ),
];

const INTERFACE: &[Row] = &[
    d(
        "mouse",
        "true",
        &[
            "Capture the mouse: click-to-select in the browser and map, wheel scrolling,",
            "and Glk mouse input for games that ask for it.",
        ],
    ),
    d("mouse_wheel_invert", "false", &["Invert wheel direction, for terminals reporting \"natural\" scrolling."]),
    d(
        "command_bar",
        "false",
        &[
            "Type into a persistent command bar instead of the inline story prompt.",
            "(Unrelated to the [command_panel] section further down, which is the",
            "point-and-click phrase builder.)",
        ],
    ),
    d("command_prefix", "\"/\"", &["The character that routes a line to a slash command."]),
    d(
        "guidance",
        "true",
        &[
            "Lanthorn's Guiding Light: help offered while you play — the words the",
            "parser knows, a completed noun, a caution before a move that cannot be",
            "undone. Marked in the margin with its own glyph rather than in the text;",
            "\"gutter.assist\" in style.toml sets the mark. False for silence.",
        ],
    ),
    d(
        "guidance_probe",
        "true",
        &[
            "Before offering a word, try it in a silent throwaway copy of the game",
            "and keep only what actually did something — so the light recommends",
            "rather than merely lists. The copy runs out of the way, on its own",
            "thread: the game answers you at once and the suggestion follows a beat",
            "later, or is dropped if you have already typed again. It may READ your",
            "game's own stored data and never writes a byte of it, and nothing it",
            "does reaches the screen, your saves or the game you are playing.",
            "False still offers, more modestly.",
        ],
    ),
    d(
        "return_probe",
        "true",
        &[
            "After a move, look for the way BACK in a silent throwaway copy of the",
            "game, and put it on the map when it is found. Automaps otherwise learn",
            "passages one direction at a time, and the honest alternative — assuming",
            "the way back is the way you came — is wrong often enough in these games",
            "to be worse than the gap. Nothing is recorded unless the copy actually",
            "comes out in the room you left: a probe that lands somewhere else",
            "records nothing at all, and neither does one that finds no way back.",
            "",
            "On by default: it runs your game a few extra turns in private after",
            "every move that opens a gap, and never touches your screen or saves.",
            "The footprint on the STORY pane's bottom border switches it — beside",
            "the map toggle, since the search keeps running with the map hidden —",
            "and \"/set-return-probe\" persists it per-game.",
        ],
    ),
    d(
        "hide_adult_words",
        "true",
        &[
            "Keep the words below out of any panel that ENUMERATES a story's",
            "vocabulary unprompted — the command panel's VERB column and its like.",
            "Infocom's dictionaries are saltier than their prose, and a panel puts",
            "the whole lot in front of anyone who opens it.",
            "",
            "DISPLAY ONLY. The story still knows every word: typing one parses",
            "exactly as it always did, and Lanthorn's Guiding Light still offers it",
            "when you reach for it. False shows the full column and keeps the list.",
        ],
    ),
    live(
        "adult_words",
        "[\"fuck\", \"fucked\", \"fucking\", \"shit\", \"cunt\", \"cum\", \"wank\", \"bastard\", \"bitch\", \"asshole\", \"whore\", \"slut\", \"rape\", \"molest\"]",
        &[
            "…and these are the words. Written out, uncommented, and yours: shorten",
            "it, extend it, or set it to [] to hide nothing. It is deliberately the",
            "strong end only — `damn`, `barf`, `hell`, `crap` and `piss` are Infocom",
            "being Infocom and stay visible. `rape` and `molest` are not swearing at",
            "all; they are here because a panel listing them unbidden is worse.",
            "",
            "Matched whole and case-insensitively, never by prefix — old dictionaries",
            "truncate, and a prefix rule wide enough to catch `bast` would also eat",
            "the real verbs `rap` and `who`.",
        ],
    ),
    d("show_status_bar", "true", &["Show the status/score bar across the top of the story pane."]),
    d("show_room_numbers", "false", &["Show room numbers (#id) inside Boxes-zoom room boxes."]),
    d("split_ratio", "50", &["The story pane's share of the story/map split, as a percentage."]),
    d("inv_dock_pct", "33", &["Inventory panel height cap, as a percentage of screen height."]),
    d(
        "room_dock_pct",
        "33",
        &[
            "Room panel height, as a percentage of screen height. The panel docks",
            "at the bottom of the map pane and describes the room you are in (or",
            "the one you clicked).",
        ],
    ),
    d(
        "text_margin_x",
        "0",
        &["Blank columns reserved inside each side of the transcript window."],
    ),
    d("text_margin_y", "0", &["Blank rows reserved above and below the transcript text."]),
    d(
        "background_tidy",
        "\"every_room\"",
        &[
            "Automatic map re-tidy when new rooms appear: \"off\", \"every_room\"",
            "(default), \"on_overlap\", \"debounced\".",
        ],
    ),
    d(
        "hint_skip_screen_warning",
        "true",
        &[
            "Auto-skip the InvisiClues \"your screen is only N characters wide…\" banner",
            "and land on the topic menu. Set false to see and dismiss it yourself.",
        ],
    ),
];

const INTERPRETER: &[Row] = &[
    d(
        "honor_game_colours",
        "true",
        &[
            "Honour game-set colours. Set false to use only the configured theme.",
            "Override for a single run with `lanthorn --game-colours on|off`.",
        ],
    ),
    d(
        "system_colours",
        "false",
        &[
            "Advertise a named machine's own default page and ink ($2C/$2D) even",
            "when the story did not come off its original media. Automatic off a",
            "release disk — that is what the disk means — so this is only for a",
            "machine you named yourself with interpreter_number, on a story that",
            "did not come off one. It cannot conjure a machine where none was named,",
            "so an ordinary story file always gets your terminal's own colours.",
        ],
    ),
    d(
        "period_look",
        "true",
        &[
            "Paint a v1-v4 story the way its own machine's interpreter did: that",
            "machine's page and ink, its status band, and the shape and colour of",
            "its cursor. Measured off emulator captures of the release disks, so it",
            "applies only when the story came off one (or you named --interpreter).",
            "Narrower than honor_game_colours, which takes this with it when off.",
            "A colour you set yourself in style.toml always wins.",
        ],
    ),
    d(
        "honor_timed_input",
        "true",
        &["Honour the Z-machine's timed input. Set false to treat every read as untimed."],
    ),
    ex(
        "interpreter_number",
        "6",
        &[
            "Interpreter number advertised in header byte 0x1E. Games branch on it:",
            "Beyond Zork picks character graphics over colour on IBM PC, and several v6",
            "story files were built for one specific machine. ZMSD §11.1.3's values:",
            "",
            "   1  DECSystem-20      7  Commodore 128",
            "   2  Apple IIe         8  Commodore 64",
            "   3  Macintosh         9  Apple IIc",
            "   4  Amiga            10  Apple IIgs",
            "   5  Atari ST         11  Tandy Color",
            "   6  IBM PC",
            "",
            "Unset auto-selects: 6 (IBM PC) for v6, else 1 (DECSystem-20). Override for",
            "a single run with `lanthorn --interpreter N`.",
        ],
    ),
    ex(
        "random_seed",
        "20250811",
        &[
            "Pin the seed every engine's random-number generator starts from, so a",
            "story replays exactly: the same shuffles, the same dice, the same dungeon.",
            "Unset (default) draws a fresh seed from the system at every launch, which",
            "is what makes a randomised game like Kerkerkruip a different game twice.",
            "lanthorn prints the seed it used on the console as it starts — copy that",
            "number in here to play that run again. A game that asks the interpreter",
            "for entropy itself (Glulx setrandom 0) still gets it, seed or no seed.",
        ],
    ),
    d(
        "glk_pixel_scale",
        "\"native\"",
        &[
            "How the pixel sizes a Glulx game asks for are mapped to your terminal.",
            "A Glk game sizes its graphics windows in pixels chosen against a normal",
            "screen (advent.blb wants a 36px toolbar), so on a HiDPI display or with a",
            "large font that request buys fewer text rows and the game's artwork",
            "shrinks against the text beside it.",
            "  \"native\" — report your terminal's real cell size (default). Safe for",
            "             every game, since it moves nobody's pixel constants.",
            "  \"auto\"   — report a 14px-tall cell whatever your font is, so artwork",
            "             scales WITH your text. Helps games whose art is too small",
            "             (advent.blb's toolbar); can SHRINK games whose art is sized",
            "             for a big screen (Counterfeit Monkey's map sidebar).",
            "  2, 3…    — divide your cell size by this instead",
            "Only affects Glulx: v6 Z-machine and Scott Adams lay out on their own",
            "fixed canvas, which lanthorn scales into the pane already.",
        ],
    ),
    d(
        "v6_render",
        "\"hybrid\"",
        &[
            "How v6 graphical games (Zork Zero, Arthur, Journey, Shogun) are drawn:",
            "  \"hybrid\"   — crisp terminal story inside a scaled pixel frame (default)",
            "  \"raster\"   — the whole pane as one pixel image, letterboxed",
            "  \"extended\" — the same pixel image, but pinned to a whole magnification",
            "               and grown DOWNWARD: the pane's surplus height becomes extra",
            "               story rows in the game's own bitmap typeface instead of",
            "               empty letterbox. The game is told nothing — its own screen",
            "               keeps the layout it always had, at the top of a taller frame.",
        ],
    ),
    d(
        "fuse_art_dither",
        "true",
        &[
            "Fuse the colour dither in a 640-wide EGA plate, the way the card did.",
            "EGA's sixteen colours were fixed in the silicon, so its artists dithered",
            "for the ones they lacked — Zork Zero's bronze arch is brown and bright red",
            "alternating column by column — and on a 640x200 screen those half-width",
            "columns blended in the eye into a colour the palette never held. lanthorn",
            "keeps all 640 columns, so it does the blending itself.",
            "Set false to see the archive's own pixels, dither and all. CGA line art is",
            "never fused either way, and 320-wide MCGA and Amiga art has no dither at",
            "this frequency to fuse.",
        ],
    ),
    d(
        "v6_arrow_keys",
        "false",
        &[
            "Forward arrow keys to a v6 story as ZSCII 129-132, for a game that binds",
            "them to movement. Off by default, so arrows keep driving scrollback and",
            "map panning the way they do in every other story. v1-5 and Glulx always",
            "get them, and so do v6 menus and \"press any key\" screens either way.",
        ],
    ),
    d(
        "system_font_disk",
        "\"\"",
        &[
            "Which of your own boot media under ~/.lanthorn/ answers first when",
            "several carry the machine's system typeface. A case-insensitive piece of",
            "the file's name — \"6.0.8\" picks the System 6.0.8 startup disk out",
            "of a folder holding System 6 and 7 — and empty means no preference.",
            "It only breaks a tie. Every medium of the right kind is read and the",
            "faces pool together, so a file named here that does not carry the face",
            "being asked for falls through to the others rather than losing it; with",
            "no preference the pool is ordered by filename.",
            "Drop a Mac OS System disk or an Amiga Kickstart ROM (*.rom) in",
            "~/.lanthorn/ and a Version 6 game off that machine's own",
            "media is drawn with the face the machine really used — Geneva on a",
            "Macintosh, which lives in the System file and on no Infocom disk, and",
            "topaz 8 on an Amiga, which lives in Kickstart and on no floppy at all.",
            "Nothing is shipped and nothing is copied; the media stay yours. With",
            "none there, the built-in face answers exactly as before.",
        ],
    ),
    d(
        "v6_pixel_lock",
        "false",
        &[
            "Lock the v6 picture to whole device pixels per ART pixel, instead of",
            "scaling it to fill the pane at whatever fraction fits. Arbitrary",
            "fractional scaling is what softens the artwork and leaves seams where a",
            "resampled edge meets a font glyph; a locked magnification is",
            "nearest-neighbour exact, and it makes every tiled side border repeat on",
            "an exact boundary too.",
            "The steps come from the artwork itself, not from a fixed list: a 320-wide",
            "rendition (Blorb, Amiga, MCGA) goes 0.5x, 1x, 1.5x, 2x…, while the",
            "standard Macintosh's mono plate and the 640-wide EGA/CGA ones go 1x, 2x,",
            "3x… The screen is centred in the pane and the margin around it carries the",
            "story's own page, as it already does.",
            "The cost is screen area — the picture stops at the rung below the pane",
            "rather than filling it. A pane too small for even the smallest step falls",
            "back to free scaling.",
        ],
    ),
    ex(
        "virtual_screen_cols",
        "80",
        &[
            "Override the screen size reported to the Z-machine in header bytes $21/$20.",
            "Unset (default) follows the story pane, which is what ZMSD §8.4 asks for.",
            "Set both only to pin a fixed virtual screen.",
        ],
    ),
    ex("virtual_screen_rows", "24", &["The height half of the same override; set it alongside the width."]),
];

const SOUND: &[Row] = &[
    d("enable_sound", "true", &["Play audio for sound_effect: bleeps and Blorb samples."]),
    d("volume", "100", &["Master volume 0-100, combined with the game's own per-sound scale."]),
];

const SEARCH: &[Row] = &[
    d("start_backward", "true", &["A new /search starts backward, from the most recent match."]),
    d("key_back", "\"n\"", &["Key that steps to an older match."]),
    d("key_forward", "\"N\"", &["Key that steps to a newer match."]),
];

const ANIMATION: &[Row] = &[
    d("enabled", "true", &["Master switch. False makes every animation instant."]),
    d(
        "easing",
        "\"ease-out\"",
        &["Easing curve: \"linear\", \"ease-in\", \"ease-out\", \"ease-in-out\"."],
    ),
    d("scroll_ms", "120", &["Smooth-scroll duration in milliseconds. 0 is instant."]),
    d(
        "scrollbar_hide_ms",
        "1500",
        &[
            "How long the STORY PANE's scrollbar stays up after you scroll it,",
            "in milliseconds. 0 keeps it up permanently. Only the story pane",
            "auto-hides - a modal's bar is reserved out of its content width, so",
            "hiding it there would reflow the list.",
        ],
    ),
    d(
        "scrollbar_fade_ms",
        "300",
        &["Fade-out time for that bar once the delay expires. 0 pops it."],
    ),
];

const COMMAND_BAND: &[Row] = &[
    d(
        "height",
        "5",
        &[
            "Rows the band occupies. It has no frame (SQ-0667) - every row here",
            "is content. Clamped to 3-11, and to whatever the screen can spare.",
            "Resize mode (the band is one of its targets while open) writes this",
            "key.",
        ],
    ),
    d("auto_open", "false", &["Open the command panel as soon as the story starts."]),
    ex(
        "verbs",
        "[ { word = \"unlock\", arity = \"pair\", prep = \"with\" }, { word = \"polish\", arity = \"object\" } ]",
        &[
            "REPLACE the whole VERB column, including the running story's own",
            "grammar - which is where the column comes from when this is unset,",
            "and normally where you want it to come from. Each entry declares the",
            "verb's shape, which decides the columns offered after it is picked:",
            "",
            "   arity = \"solo\"        look, wait, n/s/e/w - complete on its own",
            "   arity = \"object\"      take, open, read    - one object, required",
            "   arity = \"object_opt\"  search, push        - one object, optional",
            "   arity = \"pair\"        unlock ... with ... - two, joined by `prep`",
            "",
            "`prep` is the preposition a pair verb joins its objects with, and is",
            "shown as that column's header. A column filled from this key labels",
            "itself \"VERB - yours\"; unset it to get the story's own verbs back.",
        ],
    ),
    ex(
        "extra_verbs",
        "[ { word = \"xyzzy\", arity = \"solo\" } ]",
        &[
            "ADDITIVE form of the same shape: layered on whichever table is in force -",
            "usually the story's own grammar, so this patches a real verb list.",
            "An entry whose word is already there re-shapes it instead of duplicating,",
            "so this is also how you fix one verb's shape.",
        ],
    ),
    ex(
        "quick",
        "[\"n\", \"s\", \"e\", \"w\", \"ne\", \"nw\", \"se\", \"sw\", \"up\", \"down\", \"in\", \"out\", \"look\", \"inventory\", \"wait\", \"again\"]",
        &[
            "The one-click quick-action words. The compass words draw as a rose;",
            "the rest flow beside it (a narrow band falls back to a single row).",
            "The value",
            "shown is the built-in row; unset the key to keep it. Picking one SENDS",
            "it AT ONCE - no Enter, unlike every other pick in the band (which",
            "composes onto the story input line and waits for Enter, same as",
            "typing). These words are also left out of the VERB column, since",
            "showing them twice would be redundant.",
        ],
    ),
];

const KEYMAP: &[Row] = &[d(
    "use_defaults",
    "true",
    &[
        "Keep the built-in bindings and layer your overrides on top. False starts",
        "from an empty keymap, so only what you bind below works.",
    ],
)];

pub(crate) const GROUPS: &[Group] = &[
    Group { banner: "Startup and files", table: None, rows: STARTUP },
    Group { banner: "Saving and undo", table: None, rows: SAVES },
    Group { banner: "Interface", table: None, rows: INTERFACE },
    Group { banner: "Interpreter behaviour", table: None, rows: INTERPRETER },
    Group { banner: "Sound", table: None, rows: SOUND },
    Group { banner: "Transcript search", table: Some("search"), rows: SEARCH },
    Group { banner: "Animation", table: Some("animation"), rows: ANIMATION },
    Group { banner: "Command panel", table: Some("command_panel"), rows: COMMAND_BAND },
    Group { banner: "Key bindings", table: Some("keymap"), rows: KEYMAP },
];

/// Free-form trailing blocks for the open-ended tables, which have no fixed set of
/// keys to enumerate: `[keymap.*]` maps any key spec to any command string, and
/// `[hotkeys]` holds an arbitrary list of groups. Shown as commented examples.
const TRAILER: &str = r#"
# Bind keys to commands. Each entry is "key-spec" = "command args" — the KEY on the
# left, the command it runs on the right. Bind a command to two keys by writing two
# entries. Names are the hyphenated command names from the command registry (see
# /help); a command with arguments keeps them in the value ("zoom-map in").
#
# [keymap.global]
# "ctrl+q" = "quit"
# "f2" = "toggle-map"
# "ctrl+m" = "toggle-map"
# "ctrl+d" = "dump-windows"
# "ctrl+g" = "dump-cells"
#
# [keymap.map]
# "+" = "zoom-map in"
#
# [keymap.anim]
# "ctrl+t" = "animate-tidy"
#
# The story browser (the screen you get when lanthorn is pointed at a directory)
# has its own context. Only its own commands work there — it runs before a story
# is loaded, so nothing that acts on a running game has anything to act on.
#
# [keymap.browser]
# "p" = "play-story"
# "ctrl+f" = "search-ifdb"

# ── Hotkey dialog ─────────────────────────────────────────────────────────────
# prefix       — the key that opens the hotkey dialog
# direct       — commands always available without opening the dialog
# [[hotkeys.group]] — one block per group shown in the dialog
#
# [hotkeys]
# prefix = "ctrl+p"
# direct = ["toggle-map", "quit"]
#
# [[hotkeys.group]]
# title = "Map"
# commands = ["zoom-map in", "zoom-map out", "tidy-map"]
"#;

/// Render the whole commented `config.toml`.
///
/// Every line is commented, so the template parses as an EMPTY document and yields
/// [`Config::default()`] — writing it changes nothing until the user edits it.
pub fn commented_template() -> String {
    let mut out = String::new();
    out.push_str("# lanthorn config.toml\n");
    out.push_str("#\n");
    out.push_str("# Every setting lanthorn reads is listed here. Lines are commented out, and the\n");
    out.push_str("# value shown is the DEFAULT unless the comment says otherwise — so uncommenting\n");
    out.push_str("# a line as-is changes nothing. Edit the value to change behaviour.\n");
    out.push_str("#\n");
    out.push_str("# Settings added by a later release are appended, commented, so the list stays\n");
    out.push_str("# complete. A line reading `# lanthorn: no-top-up` anywhere stops that for good.\n");
    out.push_str("#\n");
    out.push_str("# Colours, glyphs and borders are NOT here: they live in style.toml.\n");
    out.push_str("#\n");
    out.push_str("# PER-GAME overrides live in a second, separate config.toml inside that game's\n");
    out.push_str("# own save directory. It holds whatever a per-game control or a `set-*` command\n");
    out.push_str("# can change for one story rather than for all of them, which today is:\n");
    out.push_str("#\n");
    // Derived from `PerGameConfig::KEYS`, never retyped: this sentence used to
    // name three keys in prose and had been wrong for two releases, because a
    // list living far from the code that decides what is per-game goes stale
    // silently. `styles::tests::write_emits_exactly_the_declared_keys` is what
    // keeps that constant honest against the writer.
    let w = crate::styles::PerGameConfig::KEYS.iter().map(|k| k.len()).max().unwrap_or(0);
    for chunk in crate::styles::PerGameConfig::KEYS.chunks(3) {
        let row: Vec<String> = chunk.iter().map(|k| format!("{k:<w$}")).collect();
        out.push_str(&format!("#   {}\n", row.join("   ").trim_end()));
    }
    out.push_str("#\n");
    out.push_str("# That file is a sparse override layer, not a copy of this one: it carries only\n");
    out.push_str("# the keys that differ, bare and uncommented, and is deleted once nothing is\n");
    out.push_str("# overridden. An absent key there means \"inherit whatever this file says\", so\n");
    out.push_str("# do NOT paste this template into a game directory — every line you uncommented\n");
    out.push_str("# would become a per-game override pinning that value for that story.\n");
    out.push_str("#\n# `version` below is the schema stamp — lanthorn manages it; leave it alone.\n");
    out.push_str("# It is also the anchor that keeps settings lanthorn writes for you (from the\n");
    out.push_str("# settings screen, say) together at the top rather than scattered.\n");
    out.push_str(&format!("version = {CONFIG_SCHEMA_VERSION}\n"));

    for g in GROUPS {
        out.push_str(&banner(g.banner));
        if let Some(t) = g.table {
            // Real, UNCOMMENTED header — exactly as style.toml does it. It also gives
            // the document structure, so a key `write_config` adds lands in the root
            // table above the sections instead of scattering.
            out.push_str(&format!("[{t}]\n"));
        }
        push_rows(&mut out, g.rows.iter());
    }
    out.push_str(TRAILER);
    out
}

/// A group's banner line, sized to the same 72-column rule everywhere it appears.
fn banner(text: &str) -> String {
    format!("\n# ── {} {}\n", text, "─".repeat(72usize.saturating_sub(text.len() + 6)))
}

/// Render `rows` — doc comment, the example caveat, then the assignment — blank-comment
/// separated, exactly as the template lays a group out.
///
/// [`top_up`] renders through this too rather than reimplementing it, so a settings
/// block appended to an existing config is indistinguishable from the same block in a
/// freshly seeded one (`a_topped_up_row_reads_exactly_as_the_template_writes_it`).
fn push_rows<'a>(out: &mut String, rows: impl Iterator<Item = &'a Row>) {
    for (i, row) in rows.enumerate() {
        if i > 0 {
            out.push_str("#\n");
        }
        for line in row.doc {
            if line.is_empty() {
                out.push_str("#\n");
            } else {
                out.push_str(&format!("# {line}\n"));
            }
        }
        if row.line == Line::Example {
            out.push_str("# (example — the default is unset)\n");
        }
        // A `Live` row is real config, not documentation of it (SQ-1122).
        let hash = if row.line == Line::Live { "" } else { "# " };
        out.push_str(&format!("{hash}{} = {}\n", row.key, row.value));
    }
}

/// Write [`commented_template`] to `config_file` when it does not exist. NEVER
/// overwrites: an existing config is the user's. Best-effort — a write failure
/// (read-only home) is swallowed so startup cannot crash on it.
///
/// Takes the config FILE, not a directory: the file's location is whatever
/// [`crate::config::config_path`] resolved (`--config`, `--user-dir`, or the default
/// home), and seeding `user_dir/config.toml` instead meant a `--user-dir` run seeded a
/// file it would never read back (SQ-0574).
pub fn auto_seed(config_file: &std::path::Path) {
    if config_file.exists() {
        return;
    }
    // Atomic (SQ-0644), like every other file lanthorn owns: a torn seed would leave
    // a config.toml that exists (so it is never re-seeded) and may not parse (so every
    // later settings save is refused by SQ-0580's guard) — a dead file the user has to
    // find and delete by hand.
    let _ = crate::storage::atomic_write(config_file, commented_template().as_bytes());
}

/// A line whose text — after any leading `#` — starts with this turns [`top_up`] off
/// for that file, permanently. Matched at the START of a line's content so this very
/// sentence, and the note the top-up writes, do not trip it.
pub const NO_TOP_UP: &str = "lanthorn: no-top-up";

/// Appended to a group's own banner where a top-up writes it, so an added block is
/// never mistaken for something the player put there.
const NEW_BANNER: &str = "new since this file was written";

/// Written once, under the banner of the first block a top-up adds.
const NEW_NOTE: &str = "\
# Lanthorn adds settings that arrived after this file did, so it stays a complete
# list of what lanthorn can be told to do. `adult_words` most of all: that list is
# a default rather than a filter nobody can see only because it is written out here
# where you can read and edit it.
#
# Nothing above was touched, and a commented line changes nothing until you edit it.
# Delete whatever you do not want. To stop lanthorn ever adding to this file again,
# give it a line reading `# lanthorn: no-top-up`.
#
";

/// Add every documented setting an existing `config.toml` has never held (SQ-1129).
///
/// [`auto_seed`] writes the catalogue once and never again, and
/// [`crate::config::write_config`] only ever updates keys the file already carries —
/// so a config written by an older release never gains a setting added since, and its
/// owner cannot discover from their own file that the setting exists. That is merely
/// stale for most settings and actively wrong for one: `adult_words` is a default
/// rather than an invisible filter BECAUSE the list ships written out where the
/// player can read and delete it (SQ-1122), and an upgraded file has no list at all.
///
/// What it does, and does not do:
///
/// * **Only ever appends.** Nothing already in the file is edited, reordered or
///   reformatted — values, spacing and the player's own comments come through byte
///   for byte.
/// * A key the file mentions **anywhere, commented or not**, is left alone. So the
///   second run adds nothing, and a line the player uncommented and edited is never
///   duplicated.
/// * Each block lands at the END of the section it belongs to, so uncommenting a
///   line puts the setting in the right table.
/// * [`Line::Live`] rows arrive uncommented, exactly as the template ships them.
/// * A file that does not parse is not touched (startup already tells the user about
///   it), nor is an EMPTY one — a config stripped to nothing is a deliberate blank
///   slate, and there is nothing there to keep complete. For a file that has content
///   but wants no more of ours, a line reading `# lanthorn: no-top-up` is the opt-out.
///
/// Why the edit is textual rather than a `toml_edit` mutation, when `toml_edit` is
/// exactly the format-preserving editor for this: what we add is mostly COMMENTS, and
/// a comment has no existence of its own in a TOML document — it lives as decor on an
/// item, and a block of commented-out settings has no item to hang from. `toml_edit`
/// still does the structural half, which is the half that can be got wrong: it says
/// where each section's body ends (so an uncommented line lands in the right table),
/// and it re-parses the result, which is refused if it is not valid TOML.
///
/// Best-effort and atomic, like [`auto_seed`]. Returns the keys it added.
pub fn top_up(config_file: &std::path::Path) -> Vec<&'static str> {
    let Ok(text) = std::fs::read_to_string(config_file) else {
        return Vec::new();
    };
    let Some((updated, added)) = topped_up(&text) else {
        return Vec::new();
    };
    match crate::storage::atomic_write(config_file, updated.as_bytes()) {
        Ok(()) => added,
        Err(_) => Vec::new(),
    }
}

/// The pure half of [`top_up`]: the file's new text and the keys added, or `None`
/// when there is nothing to add — or nothing safe to add.
fn topped_up(text: &str) -> Option<(String, Vec<&'static str>)> {
    if text.trim().is_empty() || opted_out(text) {
        return None;
    }
    // `ImDocument`, not `DocumentMut`: the mutable form despans as it is built, and
    // the spans are the whole reason we parse — they are how we know where a section's
    // body ends. (`DocumentMut::span()` compiles fine and answers `None` every time.)
    let doc = toml_edit::ImDocument::parse(text).ok()?;
    let mentioned = keys_mentioned(text);
    let headers = header_starts(doc.as_table());

    // A file whose last line has no newline still gets its block on a line of its own.
    // The only byte this adds is that missing terminator, which is not content.
    let base: String = if text.ends_with('\n') { text.to_string() } else { format!("{text}\n") };

    // ONE block per insertion point, not per group: every group of top-level keys
    // ends the same root table, and a file missing five of them wants one heading,
    // not five. Each group inside a block still carries the banner the template gives
    // it, so the reader can see which part of the catalogue arrived.
    let mut blocks: Vec<(usize, String)> = Vec::new();
    let mut added: Vec<&'static str> = Vec::new();
    for g in GROUPS {
        let rows: Vec<&Row> = g.rows.iter().filter(|r| !mentioned.contains(r.key)).collect();
        if rows.is_empty() {
            continue;
        }
        let point = insert_point(&headers, g.table, base.len());
        let at = point.unwrap_or(base.len());
        let slot = blocks.iter().position(|&(a, _)| a == at).unwrap_or_else(|| {
            blocks.push((at, String::new()));
            blocks.len() - 1
        });
        let first = added.is_empty();
        let body = &mut blocks[slot].1;
        body.push_str(&banner(&format!("{} — {NEW_BANNER}", g.banner)));
        if first {
            body.push_str(NEW_NOTE);
        }
        if point.is_none() {
            // The section itself is not in the file. Write its header live, as the
            // template does, so a key under it means what it says the moment it is
            // uncommented. An empty table is the same document as no table at all.
            body.push_str(&format!("[{}]\n", g.table.expect("only a table can be absent")));
        }
        push_rows(body, rows.iter().copied());
        added.extend(rows.iter().map(|r| r.key));
    }
    if blocks.is_empty() {
        return None;
    }

    // Splice from the back, so an offset taken against the original text is still
    // that place in the string.
    let mut out = base;
    blocks.sort_by_key(|&(at, _)| at);
    for (at, block) in blocks.into_iter().rev() {
        out.insert_str(at, &block);
    }
    // Whatever the file was doing that we did not anticipate — a section defined by
    // dotted keys, say — leaving it exactly as it was is the safe answer.
    toml_edit::ImDocument::parse(out.as_str()).ok()?;
    Some((out, added))
}

/// The player's standing "stop adding to this file".
fn opted_out(text: &str) -> bool {
    text.lines().any(|l| comment_body(l).starts_with(NO_TOP_UP))
}

/// A line with its indent and any leading `#` stripped: what the line SAYS, whether
/// it is live config or commented-out config.
fn comment_body(line: &str) -> &str {
    line.trim_start().trim_start_matches('#').trim_start()
}

/// Keys assigned anywhere in `text`, commented or not — `guidance = true` and
/// `# guidance = true` both mean the file already has that setting, one as a value
/// and one as documentation of it, and neither wants a second copy.
///
/// Flat rather than per-table, because a comment belongs to no table: every row key
/// in [`GROUPS`] is unique across the whole template (`row_keys_are_unique` pins it),
/// so there is nothing a table would disambiguate.
fn keys_mentioned(text: &str) -> std::collections::HashSet<&str> {
    let mut set = std::collections::HashSet::new();
    for line in text.lines() {
        let Some((key, _)) = comment_body(line).split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        set.insert(key);
        // `search.key_back = "n"` is the same setting as `key_back` under `[search]`.
        if let Some((_, leaf)) = key.rsplit_once('.') {
            set.insert(leaf);
        }
    }
    set
}

/// Every table header in the file, in document order: its dotted path, and the byte
/// offset where the trivia leading it begins.
///
/// The trivia, not the `[` — a header's blank line and banner comment are part of
/// the NEXT section as the reader sees it, so a block inserted at this offset ends
/// the previous section rather than wedging itself under the next one's banner.
fn header_starts(root: &toml_edit::Table) -> Vec<(String, usize)> {
    fn at(t: &toml_edit::Table) -> Option<usize> {
        let span = t.span()?;
        Some(t.decor().prefix().and_then(|p| p.span()).map_or(span.start, |s| s.start))
    }
    fn walk(t: &toml_edit::Table, path: &str, out: &mut Vec<(String, usize)>) {
        for (key, item) in t.iter() {
            let p = if path.is_empty() { key.to_string() } else { format!("{path}.{key}") };
            match item {
                toml_edit::Item::Table(child) => {
                    out.extend(at(child).map(|a| (p.clone(), a)));
                    walk(child, &p, out);
                }
                // `[[hotkeys.group]]`: each element carries its own header.
                toml_edit::Item::ArrayOfTables(arr) => {
                    for child in arr.iter() {
                        out.extend(at(child).map(|a| (p.clone(), a)));
                        walk(child, &p, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(root, "", &mut out);
    out.sort_by_key(|&(_, a)| a);
    out
}

/// Where a line appended is still inside `table` (`None` being the root table): just
/// before the next header. `None` means the file has no such section at all.
fn insert_point(headers: &[(String, usize)], table: Option<&str>, eof: usize) -> Option<usize> {
    match table {
        None => Some(headers.first().map_or(eof, |&(_, a)| a)),
        Some(t) => {
            let i = headers.iter().position(|(p, _)| p == t)?;
            Some(headers.get(i + 1).map_or(eof, |&(_, a)| a))
        }
    }
}

/// A `Config`'s Debug string with the schema stamp normalized. An absent `version`
/// key deliberately reads as 0 ("written before versioning", see the field's docs)
/// while `Config::default()` carries the current stamp, so the stamp is the one
/// difference a commented template is EXPECTED to have.
#[cfg(test)]
fn shape(cfg: &Config) -> String {
    format!("{cfg:?}").replacen(&format!("version: {}", cfg.version), "version: <stamp>", 1)
}

/// Every documented row, for the tests below and for anyone auditing coverage.
#[cfg(test)]
fn all_rows() -> Vec<(&'static str, &'static str, Line)> {
    GROUPS.iter().flat_map(|g| g.rows.iter().map(|r| (r.key, r.value, r.line))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The template must parse — and, with every line commented, must describe
    /// exactly the default config. This is what makes it safe to seed on first run.
    #[test]
    fn template_is_valid_toml_and_a_no_op_as_written() {
        let t = commented_template();

        let parsed: toml::Table = toml::from_str(&t).expect("the template parses as TOML");
        // Only the managed schema stamp and the (empty) section headers are live —
        // every actual setting is commented out.
        let mut live: Vec<&str> = parsed.keys().map(String::as_str).collect();
        live.sort_unstable();
        assert_eq!(
            live,
            // `adult_words` is the one SETTING written live (SQ-1122): the list is
            // only a default if the player can read it, and it is the default, so
            // the template is still a no-op as written.
            ["adult_words", "animation", "command_panel", "keymap", "search", "version"],
            "live keys: {parsed:?}"
        );
        for t in ["animation", "command_panel", "keymap", "search"] {
            assert!(
                parsed[t].as_table().is_some_and(|x| x.is_empty()),
                "section [{t}] is a bare header with every setting commented: {:?}",
                parsed[t]
            );
        }
        let cfg: Config = toml::from_str(&t).expect("and deserializes as a Config");
        assert_eq!(
            shape(&cfg),
            shape(&Config::default()),
            "the commented template must yield exactly the default config"
        );
    }

    /// The `[keymap.*]` and `[hotkeys]` examples must be **live config**, not prose
    /// (SQ-0759). The shipped block used to document the entry backwards and in
    /// snake_case — `quit = "ctrl+q"`, `direct = ["toggle_map"]` — so a user who
    /// copied it got `cannot parse key 'quit'` and no binding. Uncomment the whole
    /// trailer and run it through the real resolvers: any warning at all means the
    /// example does not work as written.
    #[test]
    fn the_commented_keymap_and_hotkey_examples_actually_bind() {
        // Keep the prose comments; take only the lines that are config — a table
        // header on its own, or a `key = value` assignment.
        let uncommented: String = TRAILER
            .lines()
            .filter_map(|l| l.strip_prefix("# "))
            .filter(|l| {
                let header = l.starts_with('[') && l.ends_with(']');
                let assignment = l.split_once(" = ").is_some_and(|(k, _)| {
                    !k.contains(' ') && !k.is_empty()
                });
                header || assignment
            })
            .collect::<Vec<_>>()
            .join("\n");
        let cfg: Config = toml::from_str(&uncommented)
            .unwrap_or_else(|e| panic!("the examples must parse as TOML: {e}\n{uncommented}"));
        assert!(
            uncommented.contains("[keymap.global]") && uncommented.contains("[[hotkeys.group]]"),
            "both example blocks must be picked up, or this guard proves nothing:\n{uncommented}"
        );

        let (km, warns) = crate::keymap::KeyMap::resolve(&cfg.keymap);
        assert!(warns.is_empty(), "the [keymap.*] examples must resolve cleanly: {warns:?}");
        // …and actually bind: a resolver that silently dropped every entry would
        // also produce no warnings.
        let ctrl_q: crate::keymap::KeySpec = "ctrl+q".parse().unwrap();
        assert_eq!(km.lookup(&ctrl_q, crate::keymap::Context::Global), Some("quit"));

        let (_layout, warns) = crate::keymap::HotkeyLayout::resolve(&cfg.hotkeys);
        assert!(warns.is_empty(), "the [hotkeys] examples must resolve cleanly: {warns:?}");
    }

    /// Uncommenting a `Line::Default` row must reproduce the default it claims to
    /// document — otherwise the file lies about what lanthorn does. Checked one key
    /// at a time so a failure names the offender.
    #[test]
    fn template_default_lines_are_really_the_defaults() {
        let base = shape(&Config::default());
        for (key, value, line) in all_rows() {
            // `Live` rows are defaults as well — they are simply written
            // uncommented, so their value has to be the default just as hard.
            if line == Line::Example {
                continue;
            }
            // Table rows need their header to land in the right section.
            let table = GROUPS
                .iter()
                .find(|g| g.rows.iter().any(|r| r.key == key))
                .and_then(|g| g.table);
            let doc = match table {
                Some(t) => format!("[{t}]\n{key} = {value}\n"),
                None => format!("{key} = {value}\n"),
            };
            let cfg: Config = toml::from_str(&doc)
                .unwrap_or_else(|e| panic!("template line for `{key}` is not valid TOML/type: {e}\n{doc}"));
            assert_eq!(
                shape(&cfg),
                base,
                "`{key} = {value}` is documented as the default but changes the config"
            );
        }
    }

    /// Anti-drift: every key `Config` loads from the file must appear in the
    /// template, or a new setting silently becomes undiscoverable again — the exact
    /// problem this module exists to fix. The persisted set is read straight out of
    /// `resolve`'s field-by-field merge, so adding a field there without documenting
    /// it here fails this test.
    #[test]
    fn every_persisted_setting_is_documented() {
        let src = include_str!("config.rs");
        let mut persisted: Vec<&str> = src
            .lines()
            .filter_map(|l| {
                let (key, _) = l.trim().strip_prefix("cfg.")?.split_once(" = from_file.")?;
                Some(key)
            })
            .collect();
        persisted.sort_unstable();
        persisted.dedup();
        assert!(persisted.len() > 30, "sanity: found the merge list ({} keys)", persisted.len());

        let documented: Vec<&str> = all_rows().iter().map(|(k, _, _)| *k).collect();
        let trailer = TRAILER;
        // `version` is managed by lanthorn (stamped by write_config, never hand-set),
        // and the two open-ended tables are documented as commented example blocks in
        // the trailer rather than as enumerable keys.
        // `font_check_pending` joins it for the same reason and no other: it is a
        // note lanthorn leaves itself that the font question is still owed, not a
        // preference. Templating it would advertise a key whose only honest value
        // is whatever lanthorn last wrote, and a reader who set it by hand would be
        // asking to be prompted once and then never again — which is what
        // `--font-check on` and `/run-font-check` already do, on purpose (SQ-1112).
        // `command_band` is exempt for a narrower reason than the other two: the
        // Rust FIELD keeps that name (an internal identifier, SQ-1237 left it
        // alone), but the TOML section it (de)serialises to is renamed to
        // `command_panel` via `#[serde(rename = "command_panel")]` — so the
        // generic `GROUPS.table == field name` check below can't find it under
        // its own name. `Group { table: Some("command_panel"), .. }` is exactly
        // where it is documented.
        let exempt = ["version", "font_check_pending", "command_band"];
        let missing: Vec<&str> = persisted
            .iter()
            .copied()
            .filter(|k| {
                !documented.contains(k)
                    && !exempt.contains(k)
                    && !trailer.contains(&format!("[{k}"))
                    && !GROUPS.iter().any(|g| g.table == Some(*k))
            })
            .collect();
        assert!(
            missing.is_empty(),
            "these settings are loaded from config.toml but not documented in the template: {missing:?}"
        );
    }

    /// The end-to-end shape the reporter hit: seed a fresh config, then let something
    /// save settings (the story browser's "remember this directory?" prompt is enough)
    /// and confirm the file is still the annotated template rather than the old flat
    /// key list. `write_config` used to stamp all ~36 keys unconditionally, and because
    /// an all-comment file parses as trailing trivia they landed ABOVE the comments —
    /// so a brand-new config read as the old format at first glance.
    #[test]
    fn a_settings_save_keeps_the_seeded_template() {
        let dir = std::env::temp_dir().join(format!("bm-cfgsave-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        auto_seed(&dir.join("config.toml"));
        let path = dir.join("config.toml");
        let seeded = std::fs::read_to_string(&path).unwrap();

        // Exactly what the story-list prompt does: set default_story_dir and save.
        // `user_dir` deliberately left at its default: the real flow only sets it when
        // `--user-dir` was passed, and an unchanged one must not be persisted either.
        let mut cfg = Config::default();
        cfg.default_story_dir = Some(std::path::PathBuf::from("/games/if"));
        crate::config::write_config(&dir, &cfg).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();

        // The documentation survives intact, comment for comment.
        let comments = |t: &str| t.lines().filter(|l| l.trim_start().starts_with('#')).count();
        assert_eq!(comments(&after), comments(&seeded), "every comment line survives the save");
        assert!(after.contains("# ── Interpreter behaviour"), "the group banners survive");
        assert!(after.contains("#    6  IBM PC"), "the interpreter table survives");

        // And only the setting that actually changed went live.
        let parsed: toml::Table = toml::from_str(&after).unwrap();
        let mut live: Vec<&str> = parsed.keys().map(String::as_str).collect();
        live.sort_unstable();
        assert_eq!(
            live,
            ["adult_words", "animation", "command_panel", "default_story_dir", "keymap", "search", "version"],
            "only the changed setting joins the stamp, the seeded adult list and the section headers: {after}"
        );
        for t in ["animation", "command_panel", "keymap", "search"] {
            assert!(parsed[t].as_table().is_some_and(|x| x.is_empty()), "[{t}] stays a bare header");
        }

        // Re-reading it gives back the config that was saved.
        let reread: Config = toml::from_str(&after).unwrap();
        assert_eq!(reread.default_story_dir, Some(std::path::PathBuf::from("/games/if")));
        assert!(reread.auto_load, "an absent key still means its default");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The seed never clobbers a real config.
    #[test]
    fn auto_seed_creates_once_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("bm-cfgseed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        auto_seed(&dir.join("config.toml"));
        let path = dir.join("config.toml");
        let seeded = std::fs::read_to_string(&path).expect("seeded");
        assert!(seeded.contains("interpreter_number"), "the template documents interpreter_number");

        std::fs::write(&path, "volume = 40\n").unwrap();
        auto_seed(&dir.join("config.toml"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "volume = 40\n", "an existing config is untouched");
        // SQ-0644: the seed lands atomically, so it leaves no temp for the next scan
        // (or the user) to find, and never a half-written config.toml that exists —
        // and is therefore never re-seeded — but doesn't parse.
        assert!(crate::storage::leftover_temps(&dir).is_empty(), "no temp left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Topping up an existing config (SQ-1129) ──────────────────────────────

    /// A config as a player might have it two releases ago: a few settings they set,
    /// their own comments, spacing they typed themselves, and the sections in an
    /// order the template never wrote.
    const OLD_CONFIG: &str = "\
# lanthorn config.toml
#
# this is the little laptop — quiet, small screen
version = 3
auto_load = true
default_story_dir = \"/games/if\"

# 25 is about right after midnight
volume   =   25

[animation]
enabled = true
easing = \"linear\"

# I step matches forwards, like less
[search]
key_back = \"j\"
key_forward = \"k\"
";

    /// `after` must be `before` with ONE contiguous run of new bytes spliced into it.
    /// Returns that run. This is the byte-preservation proof: everything outside the
    /// insertion is compared byte for byte, not line by line, so a re-indent or a
    /// rewritten value could not slip through.
    ///
    /// The run it returns can be ROTATED by however many bytes the block happens to
    /// share with what followed it — both start `\n# \u{2500}\u{2500} `, so the common prefix runs
    /// a few characters into the block and the same few characters reappear at its
    /// end. That costs the equality proof nothing; it only means a caller should look
    /// for its content with `contains` rather than at a fixed offset.
    fn sole_insertion<'a>(before: &str, after: &'a str) -> &'a str {
        assert!(after.len() > before.len(), "nothing was added");
        let mut pre = before.bytes().zip(after.bytes()).take_while(|(a, b)| a == b).count();
        while !before.is_char_boundary(pre) || !after.is_char_boundary(pre) {
            pre -= 1;
        }
        let mut suf = before[pre..]
            .bytes()
            .rev()
            .zip(after[pre..].bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        while !before.is_char_boundary(before.len() - suf) || !after.is_char_boundary(after.len() - suf) {
            suf -= 1;
        }
        assert_eq!(
            format!("{}{}", &after[..pre], &after[after.len() - suf..]),
            before,
            "the top-up must be one insertion and nothing else"
        );
        &after[pre..after.len() - suf]
    }

    /// Uncomment one line of `text`, the way a player would.
    fn uncomment(text: &str, key: &str) -> String {
        text.lines()
            .map(|l| {
                if comment_body(l).starts_with(&format!("{key} = ")) {
                    format!("{}\n", comment_body(l))
                } else {
                    format!("{l}\n")
                }
            })
            .collect()
    }

    /// The flat "does the file already mention this key" test is only sound because
    /// no two rows share a key — otherwise a `[search]` key would be answered by a
    /// same-named root one. Cheap to check, and the alternative (scoping a scan of
    /// COMMENTS to a table, which comments do not belong to) is not available.
    #[test]
    fn row_keys_are_unique_across_the_whole_template() {
        let mut seen: Vec<&str> = all_rows().iter().map(|(k, _, _)| *k).collect();
        let n = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), n, "two documented rows share a key");
    }

    /// The end this quest exists for: a file written before this release gains the
    /// settings it has never held — `adult_words` above all, which is a default
    /// rather than an unreadable filter only because the player can see the list.
    ///
    /// The "old" file is the current template with those four assignments deleted,
    /// so what is missing is exactly what a 0.3.0 seed would have lacked, and the
    /// test cannot drift as more settings are added.
    #[test]
    fn a_config_from_before_this_release_gains_exactly_this_release() {
        const NEW: [&str; 4] = ["guidance", "guidance_probe", "hide_adult_words", "adult_words"];
        let before: String = commented_template()
            .lines()
            .filter(|l| !NEW.iter().any(|k| comment_body(l).starts_with(&format!("{k} = "))))
            .map(|l| format!("{l}\n"))
            .collect();
        for k in NEW {
            assert!(!keys_mentioned(&before).contains(k), "`{k}` must be missing to start with");
        }

        let (after, mut added) = topped_up(&before).expect("four settings are missing");
        added.sort_unstable();
        assert_eq!(added, ["adult_words", "guidance", "guidance_probe", "hide_adult_words"]);

        let block = sole_insertion(&before, &after);
        assert!(block.contains(NEW_BANNER), "the block says where it came from:\n{block}");
        // The list arrives LIVE — the whole point (SQ-1122). Every other row stays
        // commented, so the file still describes the same configuration.
        assert!(
            block.lines().any(|l| l.starts_with("adult_words = [\"")),
            "adult_words must arrive uncommented:\n{block}"
        );
        for k in ["guidance", "guidance_probe", "hide_adult_words"] {
            assert!(block.contains(&format!("\n# {k} = ")), "`{k}` arrives commented:\n{block}");
        }
        // …and in the root table, not swept into the first section below it.
        let words = after.find("\nadult_words = [").expect("live key");
        assert!(words < after.find("\n[search]").expect("first section"), "the block ends the root table");

        // A commented line changes nothing, so the only setting that moved is the
        // one that arrived live — and it arrived at its default.
        let old: Config = toml::from_str(&before).unwrap();
        let new: Config = toml::from_str(&after).unwrap();
        assert_eq!(new.adult_words, Config::default().adult_words, "the list is the default list");
        assert_eq!(
            shape(&Config { adult_words: old.adult_words.clone(), ..new.clone() }),
            shape(&old),
            "nothing but adult_words may differ after a top-up"
        );
    }

    /// Running it again must be a no-op — including on a file where the player has
    /// since uncommented and edited one of the lines we added.
    #[test]
    fn topping_up_twice_adds_nothing_the_second_time() {
        let (once, _) = topped_up(OLD_CONFIG).expect("an old config is missing plenty");
        assert!(topped_up(&once).is_none(), "the second run has nothing to add");

        let edited = uncomment(&once, "guidance").replace("guidance = true", "guidance = false");
        assert!(edited.contains("\nguidance = false"), "the player turned it off");
        assert!(topped_up(&edited).is_none(), "an edited key is not re-offered");
    }

    /// The player's file comes through untouched: their values, their comments, their
    /// spacing, their section order.
    #[test]
    fn every_byte_the_player_wrote_survives() {
        let (after, _) = topped_up(OLD_CONFIG).unwrap();
        for line in OLD_CONFIG.lines() {
            assert!(after.contains(line), "line lost or reformatted: {line:?}");
        }
        assert!(after.contains("volume   =   25"), "their spacing is theirs");
        assert!(after.contains("# 25 is about right after midnight"), "their comment stays with it");
        // And their section order is not "corrected" to the template's.
        assert!(
            after.find("[animation]").unwrap() < after.find("[search]").unwrap(),
            "sections are not reordered"
        );
        // Settings they had set are not offered again.
        assert_eq!(after.matches("\nvolume").count(), 1, "volume is not duplicated:\n{after}");
        assert_eq!(after.matches("\nenabled = ").count(), 1, "nor is [animation] enabled");
    }

    /// A block for a table's settings lands inside THAT table, so uncommenting a line
    /// means what it says. Placing it at the end of the file instead would silently
    /// put it in whichever section happened to be last.
    #[test]
    fn a_sections_settings_land_inside_that_section() {
        let (after, _) = topped_up(OLD_CONFIG).unwrap();
        let anim = after.find("[animation]").unwrap();
        let search = after.find("[search]").unwrap();
        let scroll = after.find("# scroll_ms = 120").expect("[animation] was missing scroll_ms");
        assert!(anim < scroll && scroll < search, "it belongs between the two headers:\n{after}");

        // The proof that "inside" is real and not merely visual: uncommenting it puts
        // the value in the animation table, not at the root and not in [search].
        let live: Config = toml::from_str(&uncomment(&after, "scroll_ms")).expect("still parses");
        assert_eq!(live.animation.scroll_ms, 120);
        let raw: toml::Table = toml::from_str(&uncomment(&after, "scroll_ms")).unwrap();
        assert!(!raw.contains_key("scroll_ms"), "not at the root");
    }

    /// A file with no sections at all still gets them, header and all — otherwise its
    /// table settings would arrive as root keys that quietly do nothing.
    #[test]
    fn a_missing_section_arrives_with_its_header() {
        let flat = "version = 3\nvolume = 25\n";
        let (after, added) = topped_up(flat).unwrap();
        assert!(added.contains(&"key_back"), "a [search] setting is among the missing");
        assert!(after.contains("\n[search]\n"), "the section header arrives live:\n{after}");
        // Root keys still come before the first header, or they would land in it.
        let words = after.find("\nadult_words = [").unwrap();
        assert!(words < after.find("\n[search]\n").unwrap(), "root keys stay at the root");
        // Empty sections are the same document as no sections: nothing changed but
        // the one live list.
        let after_cfg: Config = toml::from_str(&after).unwrap();
        assert_eq!(shape(&after_cfg), shape(&toml::from_str::<Config>(flat).unwrap()));
    }

    /// A freshly seeded file is already complete — which is the real test that the
    /// top-up derives what is missing from the same rows the template writes, rather
    /// than from a list someone has to remember to update.
    #[test]
    fn a_freshly_seeded_config_needs_no_top_up() {
        let seeded = commented_template();
        assert!(topped_up(&seeded).is_none(), "the template documents everything the top-up knows");
        // …and the sentence in it that names the opt-out must not BE the opt-out.
        assert!(!opted_out(&seeded), "the template describes `no-top-up` without triggering it");
    }

    /// The three files the top-up must not touch, and why each is a deliberate state
    /// rather than an oversight.
    #[test]
    fn an_emptied_an_opted_out_and_a_broken_config_are_left_alone() {
        // Emptied on purpose: an empty config is a valid one (everything at its
        // default), and there is nothing there to keep complete. Re-filling it would
        // undo the only way a player has of saying "none of this, thank you".
        assert!(topped_up("").is_none());
        assert!(topped_up("\n\n   \n").is_none());
        // Opted out: the file has content, and a standing instruction.
        let opted = format!("{OLD_CONFIG}\n# {NO_TOP_UP}\n");
        assert!(topped_up(&opted).is_none());
        assert!(topped_up(&format!("# {NO_TOP_UP} — I curate this myself\n{OLD_CONFIG}")).is_none());
        // Unreadable: the same refusal `write_config_at` makes. Startup already tells
        // the player their config could not be loaded; appending to it would only put
        // our text below their mistake.
        assert!(topped_up("volume = = 25\n").is_none());
    }

    /// What a top-up writes must be what a seed would have written, because both go
    /// through `push_rows` — otherwise the file drifts into two dialects of itself.
    #[test]
    fn a_topped_up_row_reads_exactly_as_the_template_writes_it() {
        let (after, _) = topped_up("version = 3\n").unwrap();
        let seeded = commented_template();
        for (key, _, _) in all_rows() {
            let mut want = String::new();
            let row = GROUPS.iter().flat_map(|g| g.rows).find(|r| r.key == key).unwrap();
            push_rows(&mut want, std::iter::once(row));
            assert!(seeded.contains(&want), "the template renders `{key}` as:\n{want}");
            assert!(after.contains(&want), "and so must the top-up:\n{want}");
        }
    }

    /// The whole round trip on disk, atomically, exactly as startup calls it.
    #[test]
    fn top_up_on_disk_is_atomic_and_leaves_the_file_readable() {
        let dir = std::env::temp_dir().join(format!("bm-cfgtopup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, OLD_CONFIG).unwrap();

        let added = top_up(&path);
        assert!(added.contains(&"adult_words"), "the list is added: {added:?}");
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.starts_with("# lanthorn config.toml"), "their file, still theirs");
        let _: Config = toml::from_str(&after).expect("and it still loads");
        assert!(crate::storage::leftover_temps(&dir).is_empty(), "no temp left behind");

        assert!(top_up(&path).is_empty(), "a second pass adds nothing");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), after, "and rewrites nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
