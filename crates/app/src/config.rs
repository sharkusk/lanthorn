use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::Parser;
use serde::{Deserialize, Deserializer};

use crate::anim::Easing;

// ── Keymap config ─────────────────────────────────────────────────────────────

/// The `[keymap]` section of config.toml.
///
/// `use_defaults = true` (the default) layers user bindings on top of the
/// built-in defaults. Set `use_defaults = false` for a clean-slate keymap.
///
/// Per-context override tables map key-spec strings to command strings:
///
///   `[keymap]`
///   use_defaults = true
///   [keymap.global]
///   "ctrl+s" = "save-state"
///   [keymap.map]
///   "left" = "pan-map -1 0"
///   [keymap.anim]
///   "l" = "anim-step forward"
///   [keymap.browser]
///   "p" = "play-story"
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct KeymapConfig {
    pub use_defaults: bool,
    pub global: std::collections::BTreeMap<String, String>,
    pub map: std::collections::BTreeMap<String, String>,
    pub anim: std::collections::BTreeMap<String, String>,
    /// The pre-game story browser's keys (SQ-0796). Only `Library` commands may
    /// be bound here; a game command has no `AppState` to act on before a story
    /// is chosen.
    pub browser: std::collections::BTreeMap<String, String>,
}

impl Default for KeymapConfig {
    fn default() -> Self {
        Self {
            use_defaults: true,
            global: Default::default(),
            map: Default::default(),
            anim: Default::default(),
            browser: Default::default(),
        }
    }
}

// ── Symbol config ─────────────────────────────────────────────────────────────

pub(crate) fn default_box_style() -> String { "rounded".into() }
pub(crate) fn default_arrow_set() -> String { "filled".into() }
pub(crate) fn default_portal_icons() -> String { "ascii".into() }
pub(crate) fn default_path_style() -> String { "light".into() }
/// The badge preset a style file with no `badge_icons` key gets: the letters.
pub(crate) fn default_badge_icons() -> String { "plain".into() }
// …and the three per-badge defaults, taken FROM that preset rather than spelled a
// second time beside it (SQ-1159). An absent `badge_*` key means "whatever the
// preset says", so `[elements] badge_icons = "nerdfont"` moves all three at once.
pub(crate) fn default_badge_save() -> String { crate::symbols::StoryBadges::PLAIN.save.to_string() }
pub(crate) fn default_badge_hint() -> String { crate::symbols::StoryBadges::PLAIN.hint.to_string() }
pub(crate) fn default_badge_hint_available() -> String {
    crate::symbols::StoryBadges::PLAIN.hint_available.to_string()
}
pub(crate) fn default_diagonal_corners() -> bool { true }
pub(crate) fn default_portal_path_style() -> String { "dotted".into() }
pub(crate) fn default_control_icons() -> String { "plain".into() }

/// The resolved map glyph configuration, built from style.toml's `[map]`
/// section by `style::finalize_symbols`.  All fields default to the preset
/// names that match today's hardcoded glyphs, so an absent section is a no-op.
#[derive(Debug, Deserialize, Clone)]
pub struct SymbolConfig {
    /// Room outline style preset name.
    #[serde(default = "default_box_style")]
    pub box_style: String,
    /// Arrow glyph set preset name.
    #[serde(default = "default_arrow_set")]
    pub arrow_set: String,
    /// Portal icon preset name.
    #[serde(default = "default_portal_icons")]
    pub portal_icons: String,
    /// Path line-art preset name.
    #[serde(default = "default_path_style")]
    pub path_style: String,
    /// Line-art preset for the up/down/in/out portal connectors, chosen
    /// separately from the cardinal `path_style` (default "dotted" — the
    /// ┊/┄ connectors the map has always drawn).
    #[serde(default = "default_portal_path_style")]
    pub portal_path_style: String,
    /// Preset name for the pane-border toggle controls' glyphs (SQ-1123):
    /// "plain" (Geometric Shapes, the default) or "nerdfont". The font check
    /// writes this key alongside `arrow_set`/`portal_icons`.
    #[serde(default = "default_control_icons")]
    pub control_icons: String,
    /// Row "a save exists" artifact badge glyph (default "S").
    #[serde(default = "default_badge_save")]
    pub badge_save: String,
    /// Row "a hint file exists" artifact badge glyph (default "H").
    #[serde(default = "default_badge_hint")]
    pub badge_hint: String,
    /// Row "a hint is available to download" artifact badge glyph (default "h").
    #[serde(default = "default_badge_hint_available")]
    pub badge_hint_available: String,
    /// Draw a diagonal stub out of a room corner for ne/nw/se/sw exits (SQ-0314).
    /// Default true. Set false for a terminal/font without Unicode 13 Legacy
    /// Computing coverage: the map falls back to the corner arrow plus a purely
    /// orthogonal path (the pre-SQ-0314 look).
    #[serde(default = "default_diagonal_corners")]
    pub diagonal_corners: bool,
    /// Per-slot overrides (slot key → single-char value).
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
}

impl Default for SymbolConfig {
    fn default() -> Self {
        Self {
            box_style: default_box_style(),
            arrow_set: default_arrow_set(),
            portal_icons: default_portal_icons(),
            path_style: default_path_style(),
            portal_path_style: default_portal_path_style(),
            control_icons: default_control_icons(),
            badge_save: default_badge_save(),
            badge_hint: default_badge_hint(),
            badge_hint_available: default_badge_hint_available(),
            diagonal_corners: default_diagonal_corners(),
            overrides: BTreeMap::new(),
        }
    }
}

// ── Search config ─────────────────────────────────────────────────────────────

fn default_start_backward() -> bool { true }
fn default_key_back() -> char { 'n' }
fn default_key_forward() -> char { 'N' }

/// Deserialize a single-char string field, defaulting to 'n' on empty.
/// Used for key_back and key_forward (first char of the string).
fn deserialize_char_key_back<'de, D>(d: D) -> Result<char, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    Ok(s.chars().next().unwrap_or('n'))
}

fn deserialize_char_key_forward<'de, D>(d: D) -> Result<char, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    Ok(s.chars().next().unwrap_or('N'))
}

/// The `[search]` section of config.toml.
#[derive(Debug, Deserialize, Clone)]
pub struct SearchConfig {
    /// When true (default), a new /search starts backward from the bottom (most recent match).
    #[serde(default = "default_start_backward")]
    pub start_backward: bool,
    /// Key to navigate backward (toward older lines). Default 'n'.
    #[serde(default = "default_key_back", deserialize_with = "deserialize_char_key_back")]
    pub key_back: char,
    /// Key to navigate forward (toward newer lines). Default 'N'.
    #[serde(default = "default_key_forward", deserialize_with = "deserialize_char_key_forward")]
    pub key_forward: char,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            start_backward: default_start_backward(),
            key_back: default_key_back(),
            key_forward: default_key_forward(),
        }
    }
}

// ── CLI ───────────────────────────────────────────────────────────────────────

/// lanthorn: a Z-machine interpreter with live automapping.
/// `--interpreter-version`: a decimal byte, or a single character taken as its
/// ASCII code.
///
/// Both spellings are accepted because the corpus renders the byte both ways —
/// *Shogun* r295 prints it as a decimal ("version 6.**8**") and *Nord and Bert*
/// r19 as a letter ("Version **C**") — so whichever a person is trying to
/// reproduce, they can type what they SAW rather than convert it (SQ-0885).
///
/// A single digit is a NUMBER, not a character: `--interpreter-version 8` means
/// 8, never 56. Nobody reproducing a banner wants the ASCII code of a digit, and
/// the letter form exists for letters.
fn parse_interpreter_version(s: &str) -> Result<u8, String> {
    if let Ok(n) = s.parse::<u8>() {
        return Ok(n);
    }
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) if c.is_ascii() => Ok(c as u8),
        _ => Err(format!(
            "expected a number 0-255 or one ASCII character, got {s:?}"
        )),
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "lanthorn",
    version = buildinfo::LONG,
    about = "Interactive-fiction interpreter (Z-machine, Glulx, Scott Adams) with live automapping",
    // Show `lanthorn <version>` at the top of --help (clap omits it by default).
    help_template = "{before-help}{name} {version}\n{about-with-newline}\n{usage-heading} {usage}\n\n{all-args}{after-help}",
    // SQ-1093: one wrap authority, shared with the three CLIs' hand-written
    // `HELP` constants. `term_width` PINS the width rather than capping a
    // detected one, which is the point: the `wrap_help` feature reflows to
    // whatever the terminal happens to be, so the same paragraph came out at 80
    // columns in one window and 200 in another — and next to `zvm-cli`, whose
    // prose is a string constant, the two front-ends disagreed outright. See
    // `cli_host::HELP_WIDTH` for why 80.
    term_width = cli_host::HELP_WIDTH
)]
pub struct Cli {
    /// Path to a story file (.z3/.z5/.z8 etc.) or a directory to browse. When
    /// omitted, falls back to the `default_story_dir` config setting.
    ///
    /// An `http://` or `https://` URL works here too: lanthorn downloads it,
    /// opens it exactly as it would the same file on disk — story files, Blorbs,
    /// release disk images and ZIPs alike — and then offers to keep it in your
    /// library.
    pub story: Option<PathBuf>,

    /// Override the lanthorn home directory (default: ~/.lanthorn)
    #[arg(long, value_name = "PATH")]
    pub user_dir: Option<PathBuf>,

    /// Override the storage base for saves/sidecars (default: <user_dir>/saves).
    /// Files land in `<data_dir>/<story-filename>/`.
    #[arg(long, value_name = "PATH")]
    pub data_dir: Option<PathBuf>,

    /// Path to a non-default config file
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Glulx accelerated-function interception (debug; default: on).
    #[arg(long, value_enum, value_name = "ON|OFF")]
    pub accel: Option<OnOff>,

    /// Sound for this run (bleeps + sampled audio); with it off the border still
    /// flashes as the accessibility cue. Overrides the config's `enable_sound`
    /// in both directions, so `--sound on` plays a story whose config persisted
    /// `enable_sound = false`.
    //
    // SQ-1082: which is the whole point of the rename. Spelled `--no-sound`, this
    // was one-way — it could force sound off for a run, and nothing on the command
    // line could force it on.
    #[arg(long, value_enum, value_name = "ON|OFF")]
    pub sound: Option<OnOff>,

    /// Force the terminal image protocol for cover art (default: auto-detect).
    #[arg(long, value_enum, default_value_t = ImageProtocol::Auto)]
    pub image_protocol: ImageProtocol,

    /// Image rendering — in-game graphics and story-picker cover art (default: on).
    #[arg(long, value_enum, value_name = "ON|OFF")]
    pub images: Option<OnOff>,

    /// Honour the colours the GAME asks for: `set_colour` / true-colour output on
    /// the Z-machine, Glk stylehints on Glulx. `off` tells the story the
    /// interpreter has none at all (ZMSD §8.3.2) and lets the theme paint
    /// everything.
    ///
    /// Overrides the config's `honor_game_colours` in both directions, and
    /// outranks a `garglk.ini` beside the story or this game's own sidecar — an
    /// instruction for the launch you typed it on, exactly as `--interpreter` is.
    /// Never written back to config.toml; `/set-game-colours` still overrides it
    /// mid-game.
    ///
    /// A different question from `--colour`: this one is whether the story's own
    /// requests are obeyed, that one is what DEFAULT resolves to.
    #[arg(long = "game-colours", alias = "game-colors", value_enum, value_name = "ON|OFF")]
    pub game_colours: Option<OnOff>,

    /// Lanthorn's Guiding Light — the help lanthorn offers while you PLAY: the
    /// words the parser knows, a completed noun, a caution before a move that
    /// cannot be undone (default: on).
    ///
    /// One switch for the whole set. `/set-guidance` says the same thing mid-game
    /// and the settings screen persists it; this is the launch you typed it on,
    /// and like every other flag here it is never written back to config.toml.
    //
    // Spelled `--guidance`, not `--set-guidance`: every flag in all four
    // front-ends is a bare noun with a value (`--sound`, `--images`, `--accel`,
    // `--game-colours`), and the `set-` belongs to the slash command, whose
    // registry requires a verb.
    #[arg(long, value_enum, value_name = "ON|OFF")]
    pub guidance: Option<OnOff>,

    /// Ask whether this terminal's font draws lanthorn's Nerd Font icon glyphs —
    /// the map's arrows, the portal and stairs icons, and the mark of Lanthorn's
    /// Guiding Light — and set every icon preset from the answer (SQ-1104).
    ///
    /// Three states. ABSENT asks only on a first run, when there is no
    /// `config.toml` yet. `on` asks now regardless, which is what you want after
    /// changing terminal fonts; `/run-font-check` says the same thing mid-game,
    /// against the terminal you are actually looking at. `off` never asks.
    ///
    /// The answer is written to `style.toml` as preset NAMES in `[map]`, not to
    /// `config.toml` — glyphs live in the style file — so this flag has nothing
    /// to pin for one run and no persisted key that means "the answer".
    ///
    /// `config.toml`'s `font_check_pending` is not that key and is not settable
    /// here: it records that a launch which WOULD have asked could not, so the
    /// next interactive one still can (SQ-1112). Absent means nothing is owed.
    //
    // Spelled `--font-check`, not `--set-font-check`: a bare noun with a value,
    // like `--sound`, `--images`, `--accel` and `--guidance`. The `set-` belongs
    // to the slash command, whose registry requires a verb.
    #[arg(long = "font-check", value_enum, value_name = "ON|OFF")]
    pub font_check: Option<OnOff>,

    /// Where the story pane's DEFAULT page and ink come from — the pair reported
    /// to the story in header `$2C`/`$2D` (SQ-1082).
    ///
    /// Three sources. lanthorn already consults them machine-first, falling
    /// through to the theme and then to the terminal; naming one pins it instead
    /// of letting that chain run. (They are listed below in the other order,
    /// narrowest first, which is where each one falls through TO.)
    ///
    /// `machine` is also the opt-in that says you mean the number you typed: a
    /// release disk names its own machine and gets that pair automatically, while
    /// `--interpreter 4 --colour machine` gets the Amiga's page on a plain `.z3`.
    /// It cannot conjure a machine where none was named.
    ///
    /// Absent, the full chain runs, which is `machine` without that opt-in.
    /// Inert under `--game-colours off`: an interpreter that has just declared
    /// itself colourless has no default page and ink to report.
    #[arg(long = "colour", alias = "color", value_enum, value_name = "SOURCE")]
    pub colour: Option<ColourSource>,

    /// Interpreter number to advertise in the story header (0x1E).
    ///
    /// Games branch on it: Beyond Zork picks character graphics over colour on
    /// IBM PC, and several v6 story files were built for one specific machine.
    /// The numbers are ZMSD §11.1.3's table, which `--machines` prints in full
    /// — along with the page and ink each one reports, the palette its colours
    /// resolve through, and the screen its own interpreter drew.
    ///
    /// Overrides `interpreter_number` in config.toml. With neither set, lanthorn
    /// auto-selects per Frotz's rule: 6 (IBM PC) for v6, else 1 (DECSystem-20).
    //
    // The flag is spelled `--interpreter`, matching `zvm-cli`'s `-I`/`--interpreter`
    // (SQ-0855); the FIELD keeps the config key's name because that is what it sets.
    #[arg(long = "interpreter", value_name = "N")]
    pub interpreter_number: Option<u8>,

    /// Interpreter VERSION to advertise in the story header (0x1F).
    ///
    /// A number (`8`) or a single character (`A`, taken as its ASCII code).
    /// Both spellings are accepted because games render this byte both ways:
    /// Shogun prints it as a decimal, Nord and Bert as a letter.
    ///
    /// This is an EXPERIMENT knob, not a setting — there is no config key and
    /// nothing is written back. lanthorn's default is `A` (65), which has no
    /// provenance, and the original Amiga wrote 8: on release 295 of Shogun
    /// the credits read "Amiga Interpreter version 6.65" here against the
    /// real machine's "6.8". Whether any story BRANCHES on the byte rather
    /// than merely printing it is unknown, and this is how to find out
    /// (SQ-0885).
    #[arg(long = "interpreter-version", value_name = "V",
          value_parser = parse_interpreter_version)]
    pub interpreter_version: Option<u8>,

    /// Native Infocom picture archive to draw this story's art from.
    ///
    /// The path is taken as given if absolute, else resolved beside the STORY
    /// — which is where these archives sit. Naming one is an instruction, not
    /// a hint: it beats a Blorb next to the story and the `Pic.data` an Amiga
    /// `.adf` carries, and it OUTRANKS the `pictures` key in the game's own
    /// config.toml. A file that is absent or will not decode says so and
    /// falls back to the Blorb; it never fails quietly.
    ///
    /// The archive also picks the machine, unless --interpreter says otherwise:
    /// a DOS .MG1/.EG1/.CG1 asks for the IBM PC, an Amiga Pic.data for the
    /// Amiga. So `--pictures zork0.mg1` beside `stories/zork0.z6` draws the MCGA
    /// rendition and reports an IBM PC.
    ///
    /// Requires a story on the command line: the flag names art FOR a story,
    /// so it has no referent when lanthorn opens a library. Pick a rendition
    /// from the browser with Shift-Enter instead.
    ///
    /// NOTE: Arthur's and Journey's EGA art shipped on two disks (.EG1 +
    /// .EG2); naming the first loads both. EGA's dithered colours do not yet
    /// fuse at 1:1, so fine detail reads as speckle; MCGA (.MG1), CGA (.CG1)
    /// and the Amiga Pic.data have nothing to fuse and are exact today.
    #[arg(long, value_name = "PATH", requires = "story")]
    pub pictures: Option<PathBuf>,

    /// Which story to open, on a volume or a library that holds several.
    ///
    /// A 1-based position in the list the browser would have shown, or enough
    /// of a name to pick out one story — matched case-insensitively against
    /// both the name the medium stores it under and the title the browser
    /// prints. A name that fits two stories is refused with the list rather
    /// than guessed at, and one that fits none never falls back to booting
    /// something else.
    ///
    /// So `--story arthur` and `--story 7` both reach one game on
    /// `InfocomMasterpieces.img`.
    ///
    /// This is the browser's choice, made on the command line: without it a
    /// compilation disc can only be opened by launching it and picking, so
    /// nothing headless — a capture, a harness, a bug report — can reach any
    /// game on one but the first (SQ-1078). `zvm-cli --story` spells it and
    /// matches it the same way.
    ///
    /// Requires a story on the command line, like --pictures: the flag names
    /// a story ON something, so it has no referent when lanthorn opens the
    /// default library. Naming one story goes straight into it and exits when
    /// the game does — no browser on the way in, none on the way out.
    #[arg(long = "story", value_name = "N|NAME", requires = "story")]
    pub story_pick: Option<String>,

    /// Fetch IFDB metadata and cover art for the library, then exit.
    ///
    /// The browser's `r` (missing) or `f` (all) pass, run without a terminal:
    /// the stories under the directory, sub-folders included, get their
    /// sidecar and cover written where the browser writes them, with one
    /// printed line per story as it completes. A library on a server gets
    /// its sidecars built this way, with no one at the picker.
    ///
    /// `missing` skips a story whose sidecar is current; `all` refetches them
    /// all. Exits 0 when nothing failed to fetch.
    #[arg(long, value_enum, value_name = "MISSING|ALL")]
    pub fetch: Option<FetchMode>,

    /// Import curated metadata for stories from a TSV file, then exit.
    ///
    /// For stories the IFDB pass could not identify by IFID, or that IFDB has
    /// no cover for. A header row names the columns (any order): `path`, and
    /// then `ifdb_tuid` (the story is fetched from IFDB by that id), or
    /// `title`, `author`, `year`, `genre`, `language`, `description` (written
    /// as a curated record), and `cover_url` (downloaded as the cover). One
    /// printed line per row; exits 0 unless a row failed.
    #[arg(long = "import-metadata", value_name = "TSV")]
    pub import_metadata: Option<PathBuf>,

    /// How the Version 6 graphical pane is drawn, for this launch only. The two
    /// modes are listed below, out of the same doc comments the settings screen
    /// reads, so there is no second description here to fall out of step.
    ///
    /// The same choice `/set-v6-render` makes mid-game and the settings
    /// screen persists, said before the game boots — so the first frame is
    /// already the one you meant, which is what a headless capture and a bug
    /// report both need (SQ-1079). Overrides `v6_render` in config.toml and
    /// is never written back.
    #[arg(long = "v6-render", value_enum, value_name = "MODE")]
    pub v6_render: Option<V6RenderMode>,

    /// Snap the v6 magnification to the ladder the ARTWORK implies, so one art
    /// pixel is a whole number of device pixels, for this launch only.
    ///
    /// `/set-v6-pixel-lock`'s `on` and `off` — `auto` and the bare toggle are
    /// about a session already running and have nothing to mean here. Outranks
    /// both `v6_pixel_lock` in config.toml and this game's own sidecar, for the
    /// reason `--interpreter` does: a flag is an instruction for the launch you
    /// typed it on, and a file beside the story is not.
    ///
    /// Inert on the half-blocks backend, which has no device pixels to land on;
    /// `/dump-terminal` says so when it is.
    #[arg(long = "v6-pixel-lock", value_enum, value_name = "ON|OFF")]
    pub v6_pixel_lock: Option<OnOff>,

    /// Print the ZMSD §11.1.3 machine table and exit.
    ///
    /// What every interpreter number carries — the page and ink it reports in
    /// `$2C`/`$2D`, the palette those colour numbers resolve through, the §8.3
    /// screen rules, and the screen its own interpreter drew — followed by the
    /// rows a story's VERSION moves, because a machine is not a screen: Infocom
    /// shipped two IBM interpreters and they disagree about white.
    ///
    /// The table `--interpreter` selects a row of, so you can pick a number
    /// knowing what it does. Answered before anything else, exactly as `--help`
    /// is: it describes the program rather than a story, so it needs none.
    #[arg(long)]
    pub machines: bool,

    /// Debug trace sections to enable from boot: comma list of screen,map,hostio,v6
    /// (or `all`/`none`). Output goes to <user_dir>/trace.log. (trace feature)
    #[arg(long, value_name = "LIST")]
    pub trace: Option<String>,

    /// Trace execution from boot into the debug disassembly cache and auto-open
    /// the inspector. Loads/saves a per-story executed-PC sidecar so coverage
    /// (blue lines) persists across runs. (SQ-0449)
    #[arg(long)]
    pub debug: bool,
}

/// A boolean setting said the way its slash command says it, so a flag and the
/// `/set-…` that changes it mid-game read alike (SQ-1079). `Option<OnOff>` and
/// not a bare `bool`, because "not mentioned" has to stay distinct from "off" —
/// a flag's absence must never turn a persisted `true` back off.
#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum OnOff {
    On,
    Off,
}

impl From<OnOff> for bool {
    fn from(v: OnOff) -> bool {
        matches!(v, OnOff::On)
    }
}

/// Which stories a headless `--fetch` visits.
#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum FetchMode {
    /// Stories with no current sidecar: what the browser's `r` does.
    Missing,
    /// All of them, ignoring the cache: the browser's `f`, for the lot.
    All,
}

impl FetchMode {
    /// Whether the fetch ignores a current sidecar.
    pub fn forced(self) -> bool {
        matches!(self, FetchMode::All)
    }
}

/// Which source the story pane's DEFAULT page and ink are taken from (SQ-1082).
///
/// Not a new idea — these are the three arms of the `or_else` chain
/// `colors::host_default_colours` has always resolved `$2C`/`$2D` through, given
/// names so one of them can be pinned outright. The variants are ordered from
/// the narrowest source to the widest, and each falls through to the one above
/// it when its own source has nothing to say.
///
/// A DIFFERENT axis from `honor_game_colours`, which is whether the story's own
/// `set_colour` requests are obeyed. The two were conflated because one branch
/// answered both at once; see `colors::host_default_colours` for where they part.
///
/// **It selects a REGIME, not merely the first rung of a chain** (SQ-1154), and
/// the two halves of that are symmetrical: `machine` on a bare story file is the
/// media path applied to a raw file, and `theme`/`terminal` on original media is
/// the raw path applied to a medium. Both are one predicate —
/// [`Config::machine_colours_licensed`], which those two arms withhold outright.
/// So a floppy launched under `--colour terminal` resolves its colour numbers
/// through §8.3.1's table, is told the host's own pair, and shows no period look
/// or two-colour card, exactly as the same story does as a bare file. Its
/// ARTWORK is unmoved: pictures resolve through the archive's own palette.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, clap::ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum ColourSource {
    /// the OSC 10/11 probe — your terminal's own text and background
    Terminal,
    /// your style.toml's transcript colours, when it names both; else terminal
    Theme,
    //
    // The DEFAULT variant, because it is the chain that already ran — asking for
    // it explicitly adds only the `Asked` opt-in
    // (`ProfileSource::licenses_machine_colours`), which is what
    // `--system-colours` used to be on its own.
    /// the machine's own ZMSD §8.3.3 pair, when one is named; else theme
    #[default]
    Machine,
}

/// Terminal image protocol for cover art. `Auto` detects the best available
/// (falling back to half-blocks); the rest force a specific mode for testing.
#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum ImageProtocol {
    Auto,
    Halfblocks,
    Kitty,
    Sixel,
    Iterm2,
}

fn default_image_protocol() -> ImageProtocol {
    ImageProtocol::Auto
}

fn default_images() -> bool { true }

// ── Hotkeys config ────────────────────────────────────────────────────────────

/// One group of commands shown together in the hotkey dialog.
#[derive(Debug, Default, Deserialize, Clone)]
pub struct HotkeyGroupConfig {
    pub title: String,
    pub commands: Vec<String>,
}

/// The `[hotkeys]` section of config.toml.
/// `prefix` overrides the dialog-open key (default: Ctrl+P).
/// `direct` overrides which commands are always available (bypass dialog).
/// `group` overrides the command groups shown in the dialog.
#[derive(Debug, Default, Deserialize, Clone)]
pub struct HotkeysConfig {
    /// Override the dialog-prefix key spec string (e.g. "ctrl+p").
    pub prefix: Option<String>,
    /// Override the set of always-available commands (by snake_case name).
    pub direct: Option<Vec<String>>,
    /// Override the command groups in the dialog.
    #[serde(default)]
    pub group: Vec<HotkeyGroupConfig>,
}

// ── [command_panel] ────────────────────────────────────────────────────────────

/// One verb entry in `[command_panel] verbs` / `extra_verbs`:
/// `{ word = "unlock", arity = "pair", prep = "with" }`.
///
/// `arity` is one of `solo`, `object`, `object_opt` (`object?` is accepted too)
/// and `pair`; an unrecognised value is reported as a warning and the entry is
/// skipped rather than silently reinterpreted.
///
/// The runtime model behind the band is no longer an arity enum but a list of
/// sentence shapes read from the story's own grammar (SQ-1111). `arity` survives
/// as the CONFIG SPELLING — every one of these four words still names a shape
/// list exactly, and `object_opt` is the one that gives the game away: it was
/// the enum straining to hold two lines at once, and now simply IS two
/// ([`arity_lines`]).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct VerbConfig {
    pub word: String,
    #[serde(default = "default_verb_arity")]
    pub arity: String,
    #[serde(default)]
    pub prep: Option<String>,
}

fn default_verb_arity() -> String {
    "object".to_string()
}

/// The `[command_panel]` section: the bottom command band's size, whether it
/// opens with the story, and its grammar.
///
/// Not to be confused with the top-level `command_bar` boolean, which is an
/// unrelated setting (type into a persistent command bar instead of the inline
/// story prompt).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CommandBandConfig {
    /// Band rows. The band has no frame since SQ-0667 (2026-08-05) — every
    /// row here is content. Clamped to `MIN_BAND_ROWS..=MAX_BAND_ROWS` (3..=11)
    /// at layout time.
    #[serde(default = "default_band_height")]
    pub height: u16,
    /// Open the band as soon as the story starts.
    #[serde(default)]
    pub auto_open: bool,
    /// REPLACES the verb column wholesale when set (non-empty) — the story's
    /// own grammar included. See [`CommandBandConfig::resolve_verbs_with`] for
    /// why that is still the right reading of this key now that the column has
    /// a real source behind it.
    #[serde(default)]
    pub verbs: Vec<VerbConfig>,
    /// ADDITIVE form: appended to whichever table is in force — the story's
    /// grammar, the built-in fallback, or a `verbs` replacement. A word already
    /// present is overridden, so this is also how one verb's shape gets fixed.
    #[serde(default)]
    pub extra_verbs: Vec<VerbConfig>,
    /// The one-click quick-action row. Empty means the built-in row.
    #[serde(default)]
    pub quick: Vec<String>,
}

impl Default for CommandBandConfig {
    fn default() -> Self {
        CommandBandConfig {
            height: default_band_height(),
            auto_open: false,
            verbs: Vec::new(),
            extra_verbs: Vec::new(),
            quick: Vec::new(),
        }
    }
}

/// The sentence shapes a config `arity` spelling names, or `None` for a spelling
/// that is not one of the four.
///
/// This is the whole of what the retired `Arity` enum did, as data: the four
/// words each name a list of [`VerbLine`]s, and `object_opt` — the value the
/// enum needed a separate variant for — is simply the two-line list a real
/// grammar would have written out.
fn arity_lines(
    arity: &str,
    prep: Option<&str>,
) -> Option<Vec<crate::render::command_band::VerbLine>> {
    use crate::render::command_band::VerbLine;
    Some(match arity.trim().to_ascii_lowercase().as_str() {
        "solo" => vec![VerbLine::bare()],
        "object" => vec![VerbLine::object()],
        "object_opt" | "object?" | "objectopt" => vec![VerbLine::bare(), VerbLine::object()],
        "pair" => vec![VerbLine::pair(prep.unwrap_or("with"))],
        _ => return None,
    })
}

/// Lower one `[command_panel]` verb entry, pushing a warning for an unrecognised
/// `arity` rather than silently reinterpreting it.
fn lower_verb(
    v: &VerbConfig,
    warnings: &mut Vec<String>,
) -> Option<crate::render::command_band::VerbEntry> {
    match arity_lines(&v.arity, v.prep.as_deref()) {
        Some(lines) => Some(crate::render::command_band::VerbEntry::new(&v.word, lines)),
        None => {
            warnings.push(format!(
                "command_band: verb '{}' has unknown arity '{}' \
                 (expected solo, object, object_opt or pair); skipped",
                v.word, v.arity
            ));
            None
        }
    }
}

impl CommandBandConfig {
    /// Resolve the configured grammar with no story grammar to hand — the band
    /// opens from `apply_action`, which has the config but no engine, so this is
    /// what it is born on. `render::command_band::refresh_verbs` calls
    /// [`resolve_verbs_with`](Self::resolve_verbs_with) a tick later with the
    /// story's own words.
    pub fn resolve_verbs(&self) -> (crate::render::command_band::VerbTable, Vec<String>) {
        self.resolve_verbs_with(None)
    }

    /// Resolve the verb column, given whatever the running story's grammar
    /// answered (`None` = it could not be read, or has not been asked yet).
    ///
    /// # What the two keys mean now the column has a real source
    ///
    /// The question SQ-1111 put deliberately, answered deliberately:
    ///
    /// * **`verbs` still REPLACES the whole column**, story grammar included. It
    ///   is an explicit, complete statement of what a player wants offered —
    ///   somebody who wrote out twelve verbs in their own order chose those
    ///   twelve, and quietly folding two hundred of the story's own in beside
    ///   them would destroy the only thing the key is for. Every existing config
    ///   therefore keeps its exact behaviour: the key replaced the whole column
    ///   before and replaces the whole column now.
    /// * **`extra_verbs` EXTENDS whatever is in force**, which is now usually the
    ///   story's own list — so it patches the grammar rather than a constant, and
    ///   an entry naming a word the story already has re-shapes that one verb.
    ///
    /// The alternative readings (make `verbs` a filter, or an ordering hint)
    /// were rejected on the same ground: both silently change what an existing
    /// config produces, and neither is what either key's NAME says.
    ///
    /// Returns the table plus any warnings (bad `arity` spellings), which
    /// startup surfaces the same way it surfaces keymap warnings.
    pub fn resolve_verbs_with(
        &self,
        story: Option<Vec<crate::render::command_band::VerbEntry>>,
    ) -> (crate::render::command_band::VerbTable, Vec<String>) {
        use crate::render::command_band::{default_verbs, VerbSource, VerbTable};
        let mut warnings = Vec::new();
        let base = if !self.verbs.is_empty() {
            VerbTable::new(
                self.verbs.iter().filter_map(|v| lower_verb(v, &mut warnings)).collect(),
                VerbSource::Configured,
            )
        } else if let Some(entries) = story.filter(|e| !e.is_empty()) {
            VerbTable::new(entries, VerbSource::Story)
        } else {
            default_verbs()
        };
        let table = self.layer_extra_verbs_into(base, &mut warnings);
        (table, warnings)
    }

    /// Layer `extra_verbs` onto a table that is already resolved — the half of
    /// [`resolve_verbs_with`](Self::resolve_verbs_with) that has to happen again
    /// when the story's own grammar arrives a tick after the band opened.
    ///
    /// A bad `arity` here is SILENT, because the open-time resolve above already
    /// reported that same entry and warning twice for one typo is noise.
    pub fn layer_extra_verbs(
        &self,
        table: crate::render::command_band::VerbTable,
    ) -> crate::render::command_band::VerbTable {
        self.layer_extra_verbs_into(table, &mut Vec::new())
    }

    fn layer_extra_verbs_into(
        &self,
        mut table: crate::render::command_band::VerbTable,
        warnings: &mut Vec<String>,
    ) -> crate::render::command_band::VerbTable {
        for extra in &self.extra_verbs {
            if let Some(e) = lower_verb(extra, warnings) {
                match table.entries.iter_mut().find(|t| t.word == e.word) {
                    Some(slot) => *slot = e,
                    None => table.entries.push(e),
                }
            }
        }
        table
    }

    /// The quick row in force: the configured one, else the built-in.
    pub fn resolve_quick(&self) -> Vec<String> {
        if self.quick.is_empty() {
            crate::render::command_band::default_quick()
        } else {
            self.quick.clone()
        }
    }
}

// ── Config ────────────────────────────────────────────────────────────────────

fn default_command_prefix() -> char { '/' }
fn default_undo_levels() -> usize { 16 }
/// Rewind/replay history cap (SQ-1185): generous enough that the feature still
/// reaches "further back than the game's own UNDO" (`docs/internals/saves.md`),
/// while bounding the per-turn VM snapshots the archive keeps in memory across
/// an arbitrarily long session.
fn default_history_turns() -> usize { 500 }

/// Fallback screen size used only before the story pane has been measured (the
/// engine boots before the first frame) and by the Glulx factory, whose real
/// size arrives one poll later from `poll_glulx_resize`. ZMSD §8.4 asks for "at
/// least 60 characters wide by 14 lines deep"; 80×24 is the classic terminal.
pub const FALLBACK_SCREEN_COLS: u16 = 80;
pub const FALLBACK_SCREEN_ROWS: u16 = 24;
pub(crate) fn default_split_ratio() -> u16 { 50 }
pub(crate) fn default_inv_dock_pct() -> u16 { 33 }
/// The room dock's height as a percentage of the frame (SQ-0692, retuned SQ-0694).
///
/// Back to `inv_dock_pct`'s 33 now that the exit card spends COLUMNS instead of
/// rows: at a typical split-pane map width the twelve directions lay out three
/// across in four rows, so the whole Info body — header, objects, card — wants
/// about eleven rows rather than the sixteen the single column needed. 33% of a
/// 40-row terminal is thirteen, which admits all of it with room to spare.
pub(crate) fn default_room_dock_pct() -> u16 { 33 }
pub(crate) fn default_band_height() -> u16 {
    crate::render::command_band::DEFAULT_BAND_ROWS
}
fn default_honor_game_colours() -> bool { true }
fn default_period_look() -> bool { true }
fn default_system_colours() -> bool { false }
fn default_acceleration() -> bool { true }
fn default_honor_timed_input() -> bool { true }
fn default_enable_sound() -> bool { true }
fn default_volume() -> u8 { 100 }

/// Deserialize a single-char string field into a `char`.  Takes the first
/// Unicode scalar value of the string; falls back to `/` on an empty string.
fn deserialize_char_from_str<'de, D>(d: D) -> Result<char, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    Ok(s.chars().next().unwrap_or('/'))
}

/// Fallback for [`Config::config_file`] when a `Config` is built without [`resolve`]
/// (tests, `Config::default()`): the default home's config.toml.
fn default_config_file() -> PathBuf {
    default_user_dir().join("config.toml")
}

fn default_user_dir() -> PathBuf {
    let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join(".lanthorn")
}

fn default_true() -> bool { true }

// ── The adult list (SQ-1122) ─────────────────────────────────────────

/// Words lanthorn does not ENUMERATE unprompted, unless the player says otherwise.
///
/// The principle, and the reason this is a top-level key rather than one of the
/// command band's: *unprompted enumeration gets a default; what the player
/// reached for does not.* A panel that lists a story's whole vocabulary puts
/// these words in front of somebody who only opened a panel; a word the player
/// typed, or one lanthorn offers BECAUSE they typed something close to it, is a
/// different act and is never filtered — SQ-1115's "faithful, we don't censor"
/// governs that half and still does. [`crate::vocab`] does not read this list,
/// and must not start.
///
/// **Strong profanity only, plus two the user named.** Chosen by reading the
/// corpus rather than from imagination: every word here is in some story's real
/// dictionary or grammar (Zork I r88 alone holds `fuck`, `shit`, `rape` and
/// `molest` in its verb table). `damn` and `barf` are Infocom being Infocom and
/// stay visible; so do `hell`, `crap`, `screw`, `suck`, `piss`, `pee` and `sod`,
/// which are coarse rather than obscene. `rape` and `molest` are not cuss words
/// at all — they are here because listing them unbidden in a panel is worse
/// than any expletive.
///
/// Matching is EXACT and case-insensitive, never by prefix. Old dictionaries
/// truncate (a v6 story's four-character keys hold `bast` for *bastard*), and a
/// prefix rule that caught those would also hide `rap` and `who`, which are real
/// verbs in forty and twenty-five corpus stories respectively. Under-filtering
/// is the instruction; a player who wants a truncation hidden adds it to their
/// own `adult_words`.
pub const DEFAULT_ADULT_WORDS: &[&str] = &[
    "fuck", "fucked", "fucking", "shit", "cunt", "cum", "wank", "bastard", "bitch", "asshole",
    "whore", "slut", "rape", "molest",
];

fn default_adult_words() -> Vec<String> {
    DEFAULT_ADULT_WORDS.iter().map(|s| s.to_string()).collect()
}

// ── Background-tidy mode ──────────────────────────────────────────────────────

/// Controls when the map is automatically re-tidied after new rooms are discovered.
///
/// TOML: `background_tidy = "every_room"` (default), `"off"`, `"on_overlap"`, `"debounced"`.
///
/// NOTE: the default (`EveryRoom`) changes today's behavior — a full relayout runs on
/// each turn that discovers a new room. Set `background_tidy = "off"` to keep the
/// manual-only tidy behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTidy {
    /// Never auto-tidy; only manual Retidy / AnimateTidy.
    Off,
    /// Re-tidy whenever a turn discovers a new room (default).
    #[default]
    EveryRoom,
    /// Re-tidy only when incremental placement caused an overlap or distorted edge.
    OnOverlap,
    /// Re-tidy once every K new rooms (`BG_TIDY_DEBOUNCE`).
    Debounced,
}

/// Number of new rooms that must accumulate before a `Debounced` background tidy fires.
pub const BG_TIDY_DEBOUNCE: u32 = 5;

/// Where to persist v5 auxiliary save data (the `save/restore table` opcodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuxStorage {
    /// Ask the user on first use, then store the choice in config.
    #[default]
    Ask,
    /// Inside each `.lanthorn` save archive.
    Archive,
    /// In one per-game file in the save directory (shared across playthroughs).
    Global,
}

/// How many device pixels the host reports per "pixel" a Glulx game asks for
/// (SQ-0593).
///
/// TOML: `glk_pixel_scale = "auto"` (default) or an integer like `1` or `2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlkPixelScale {
    /// Report the terminal's own cell size, unchanged. **The default**, because a
    /// game's pixel constants can be too small OR too large for the screen it ends up
    /// on, and only leaving them alone is safe for both (see [`GlkPixelScale::apply`]).
    #[default]
    Native,
    /// Normalise to a [`REFERENCE_CELL_PX`]-tall cell, so a game's artwork scales with
    /// the text rather than holding a fixed pixel size.
    Auto,
    /// Report the cell size divided by exactly this, whatever the terminal says.
    Fixed(u32),
}

/// Accepts `"auto"` or a bare integer. A derived `untagged` impl cannot do this — an
/// untagged unit variant matches null, not the string `"auto"` — so the two shapes are
/// spelled out.
impl<'de> serde::Deserialize<'de> for GlkPixelScale {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Num(u32),
            Str(String),
        }
        match Raw::deserialize(d)? {
            Raw::Num(n) => Ok(GlkPixelScale::Fixed(n)),
            Raw::Str(s) if s.eq_ignore_ascii_case("native") => Ok(GlkPixelScale::Native),
            Raw::Str(s) if s.eq_ignore_ascii_case("auto") => Ok(GlkPixelScale::Auto),
            // A quoted number is a natural thing to write and costs nothing to accept.
            Raw::Str(s) => s.trim().parse::<u32>().map(GlkPixelScale::Fixed).map_err(|_| {
                serde::de::Error::custom(format!(
                    "glk_pixel_scale must be \"native\", \"auto\" or a positive integer, got {s:?}"
                ))
            }),
        }
    }
}

/// A conventional terminal cell height in device pixels, and the yardstick `Auto`
/// measures against. It is what an unscaled 1x display reports for a normal font, so
/// `Auto` resolves to 1 there and changes nothing.
pub const REFERENCE_CELL_PX: u32 = 14;

impl GlkPixelScale {
    /// The cell size to hand a Glulx game, given what the terminal reports.
    ///
    /// A Glk game sizes its graphics windows in PIXELS, and those requests are
    /// constants its author picked against a conventional screen — advent.blb asks for
    /// a 36px toolbar. The row count that buys is `ceil(36 / cell_height)`, so the
    /// terminal's cell size decides how much of the screen the game's artwork occupies:
    /// a fixed strip at 40px whatever the font, which on a scaled display or with a
    /// large font is a shrinking fraction of a growing screen.
    ///
    /// `Auto` normalises that away by reporting a cell of exactly
    /// [`REFERENCE_CELL_PX`] tall, with the width scaled to preserve the terminal's own
    /// aspect ratio. Every terminal then presents the same pixel space, so a game's
    /// layout is identical everywhere and its artwork scales WITH the text rather than
    /// against it — 3 rows at any font size, `3 × cell_height` on screen.
    ///
    /// Deliberately not keyed off the display's DPI: a large font on an unscaled
    /// display produces the identical complaint while reporting no unusual DPI, and
    /// reading the real scale would need a separate platform path for each Linux
    /// compositor, macOS and Windows. It is also deliberately not an integer divisor,
    /// which was the first attempt — `round(cell / 14)` puts a cliff at 21px, where a
    /// one-pixel font change doubled the toolbar (40px → 84px). 21px is exactly a 1.5x
    /// scaled 14px cell, i.e. a common Windows and GNOME default, so that cliff sat in
    /// the middle of real configurations.
    ///
    /// **`Native` is the default, and normalisation is opt-in**, because the two
    /// directions cannot both be satisfied. A game's pixel constants are chosen against
    /// the screen its author had, and they land on both sides of ours: advent.blb wants
    /// a 36px toolbar (too SMALL on a modern screen — normalising helps), while an
    /// Inform 7 map sidebar asks for a fixed 722px (too LARGE — normalising to a 693px
    /// screen makes it not fit, and Counterfeit Monkey's map stops being half the
    /// screen). Whichever reference is chosen, one of those two breaks; leaving the
    /// terminal's own cell alone is the only setting that is never actively wrong, so
    /// it is what an unconfigured lanthorn does.
    ///
    /// `Fixed(n)` divides by `n`, for pinning the trade-off manually. Never returns 0
    /// on either axis.
    pub fn apply(self, cell: (u32, u32)) -> (u32, u32) {
        match self {
            GlkPixelScale::Native => cell,
            GlkPixelScale::Fixed(n) => {
                let n = n.max(1);
                ((cell.0 / n).max(1), (cell.1 / n).max(1))
            }
            GlkPixelScale::Auto => {
                let h = cell.1.max(1);
                let w = ((cell.0 as f32 * REFERENCE_CELL_PX as f32 / h as f32).round() as u32).max(1);
                (w, REFERENCE_CELL_PX)
            }
        }
    }
}

/// How the v6 graphical story pane is rendered.
///
/// TOML: `v6_render = "hybrid"` (default), `"raster"` or `"extended"`. Any other
/// string — including `"frameless"`, the mode SQ-0895 removed — silently reads as
/// `Hybrid`; see [`deserialize_v6_render`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Default, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "lowercase")]
pub enum V6RenderMode {
    /// Chrome (frame + status) as a scaled pixel ring around a terminal story
    /// viewport with crisp text (default).
    #[default]
    Hybrid,
    /// The whole pane rasterized into one pixel image (feature-limited).
    Raster,
    /// [`Self::Raster`], but the frame GROWS downward instead of letterboxing:
    /// the magnification is pinned to a whole number of device pixels per native
    /// pixel and the pane's surplus height buys whole text rows of prose in the
    /// game's own bitmap face (SQ-1032). The game is told nothing — its own screen
    /// is the top of a taller canvas, laid out exactly as it always was.
    Extended,
}

/// The `v6_render` token for a mode — what the file holds, and so what a one-run
/// pin has to hold too. One spelling for both ends, because [`write_config_at`]
/// compares the pinned value against the value it is about to write, and a
/// second copy of this `match` that ever disagreed would silently un-pin the key
/// (SQ-1079).
pub fn v6_render_key(mode: V6RenderMode) -> &'static str {
    match mode {
        V6RenderMode::Hybrid => "hybrid",
        V6RenderMode::Raster => "raster",
        V6RenderMode::Extended => "extended",
    }
}

/// The mode a `v6_render` token names, or `None` for anything else — the inverse
/// of [`v6_render_key`], used to read the per-game sidecar's own copy of the key
/// (SQ-1123). An unrecognised token inherits the global mode rather than failing
/// a boot, which is what every other malformed sidecar value already does.
pub fn v6_render_from_key(token: &str) -> Option<V6RenderMode> {
    match token {
        "hybrid" => Some(V6RenderMode::Hybrid),
        "raster" => Some(V6RenderMode::Raster),
        "extended" => Some(V6RenderMode::Extended),
        _ => None,
    }
}

/// Read `v6_render`, falling back to the default on any unrecognised string.
///
/// Deliberately silent, matching [`deserialize_easing`]: a config naming a mode
/// this build does not have must not stop the game from launching. That covers
/// the removed `"frameless"` (SQ-0895) and a plain typo alike — the trade is
/// that `"rastr"` quietly renders hybrid rather than complaining, which is the
/// behaviour every other token-valued key here already has.
fn deserialize_v6_render<'de, D>(d: D) -> Result<V6RenderMode, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    Ok(match s.as_str() {
        "raster" => V6RenderMode::Raster,
        "extended" => V6RenderMode::Extended,
        _ => V6RenderMode::Hybrid,
    })
}

// ── Animation config ──────────────────────────────────────────────────────────

fn default_scroll_ms() -> u64 { 120 }
fn default_scrollbar_hide_ms() -> u64 { 1500 }
fn default_scrollbar_fade_ms() -> u64 { 300 }
fn default_easing() -> Easing { Easing::EaseOut }

/// Deserialize an easing token string (e.g. "ease-out") into an [`Easing`].
/// Unknown tokens fall back to `EaseOut` (via `parse_easing`).
fn deserialize_easing<'de, D>(d: D) -> Result<Easing, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    Ok(crate::anim::parse_easing(&s))
}

/// The `[animation]` section of config.toml. Controls the shared TUI animation
/// engine. With `enabled = false` (or `scroll_ms = 0`) every animation is
/// instant, exactly reproducing the pre-animation behavior.
#[derive(Debug, Deserialize, Clone)]
pub struct AnimationConfig {
    /// Master switch (default true). When false, every animation is instant.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Easing curve token (default "ease-out").
    #[serde(default = "default_easing", deserialize_with = "deserialize_easing")]
    pub easing: Easing,
    /// Smooth-scroll duration in milliseconds (default 120). Zero = instant.
    #[serde(default = "default_scroll_ms")]
    pub scroll_ms: u64,
    /// SQ-0782: how long the STORY PANE's scrollbar stays up after a scroll,
    /// in milliseconds (default 1500). Zero keeps it up permanently — the
    /// pre-auto-hide behaviour. Only the story pane auto-hides: a modal's bar
    /// is reserved out of its content width, so hiding one there would reflow
    /// the list, while the story pane's sits in the margin band beside the text.
    #[serde(default = "default_scrollbar_hide_ms")]
    pub scrollbar_hide_ms: u64,
    /// How long that bar takes to fade out once the delay expires, in
    /// milliseconds (default 300). Zero — or `enabled = false` — pops it.
    #[serde(default = "default_scrollbar_fade_ms")]
    pub scrollbar_fade_ms: u64,
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            easing: Easing::EaseOut,
            scroll_ms: 120,
            scrollbar_hide_ms: default_scrollbar_hide_ms(),
            scrollbar_fade_ms: default_scrollbar_fade_ms(),
        }
    }
}

// ── One-run overrides ─────────────────────────────────────────────────────────

/// The `config.toml` key names a one-run source can pin. Spelled once, used at
/// both ends — the pin site and [`write_config_at`] — because a typo in either
/// half silently disables the guard and the bleed comes back with no test to
/// catch it (the whole point of [`OneRunOverrides`] is that nothing about it is
/// per-key by hand).
pub mod keys {
    pub const USER_DIR: &str = "user_dir";
    pub const HONOR_GAME_COLOURS: &str = "honor_game_colours";
    pub const ENABLE_SOUND: &str = "enable_sound";
    pub const INTERPRETER_NUMBER: &str = "interpreter_number";
    pub const V6_PIXEL_LOCK: &str = "v6_pixel_lock";
    pub const GUIDANCE: &str = "guidance";
    pub const RETURN_PROBE: &str = "return_probe";
    pub const V6_RENDER: &str = "v6_render";
    pub const SYSTEM_FONT_DISK: &str = "system_font_disk";
    pub const SYSTEM_COLOURS: &str = "system_colours";
}

/// A value a one-run source pinned, in the shape the TOML key holds it.
#[derive(Debug, Clone, PartialEq)]
pub enum OneRunValue {
    Bool(bool),
    Int(i64),
    Str(String),
}

impl From<bool> for OneRunValue {
    fn from(v: bool) -> Self { OneRunValue::Bool(v) }
}
impl From<u8> for OneRunValue {
    fn from(v: u8) -> Self { OneRunValue::Int(i64::from(v)) }
}
impl From<String> for OneRunValue {
    fn from(v: String) -> Self { OneRunValue::Str(v) }
}
impl From<&str> for OneRunValue {
    fn from(v: &str) -> Self { OneRunValue::Str(v.to_string()) }
}

/// Which `config.toml` keys are in force for THIS LAUNCH ONLY, and what the
/// one-run source put in them (SQ-0807).
///
/// A CLI flag, a per-game sidecar key, a discovered `garglk.ini`, a choice the
/// launch-options dialog made without persisting it, something inferred from the
/// story's artwork — all of them mutate the live [`Config`], and [`write_config_at`]
/// writes any value that differs from the default. So without this, the first
/// settings save of the session — the story browser's "remember this directory?"
/// prompt is enough — makes a throwaway choice permanent AND global. One
/// `--sound off`, and sound is off for every story forever.
///
/// One rule covers every key: **while the live value still equals what the one-run
/// source pinned, the file's own value (or its absence) is left exactly as it is.**
/// The moment the value differs, the pin no longer describes it and the key persists
/// like anything else — which is the promotion case, and why this compares values
/// rather than tracking a "was overridden" bit. A deliberate edit that lands back on
/// the pinned value releases the pin outright ([`OneRunOverrides::release`], wired to
/// the settings panel's row edits and [`Config::set_interpreter_number`]) so even
/// "toggle away and back" persists.
///
/// Never serialized: it describes this process, not the file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OneRunOverrides(std::collections::BTreeMap<&'static str, OneRunValue>);

impl OneRunOverrides {
    /// Record that a one-run source put `value` in `key` for this launch. The
    /// caller sets the [`Config`] field itself — this only says where it came from.
    pub fn pin(&mut self, key: &'static str, value: impl Into<OneRunValue>) {
        self.0.insert(key, value.into());
    }

    /// End the one-run hold on `key`: from here it persists like any other setting.
    /// A deliberate user edit is exactly this event.
    pub fn release(&mut self, key: &str) {
        self.0.remove(key);
    }

    /// Is `key` still pinned? (SQ-0860: the settings panel's row edit calls
    /// [`Self::release`], so "the pin is gone" is how the ConfigSave handler
    /// recognises a deliberate edit of that row and ends the holds that live on
    /// `AppState` rather than in here.)
    pub fn holds(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    /// The integer a one-run source pinned on `key`, if any. (The `interpreter_number`
    /// resolution order in `boot_story` needs to tell a CLI value apart from the
    /// global config's, and both live in the same field.)
    pub fn int(&self, key: &str) -> Option<i64> {
        match self.0.get(key) {
            Some(OneRunValue::Int(n)) => Some(*n),
            _ => None,
        }
    }

    /// True while `live` — the value [`write_config_at`] is about to write — is still
    /// the one the one-run source pinned, i.e. nothing has changed it since.
    fn still_holds(&self, key: &str, live: &toml_edit::Value) -> bool {
        match (self.0.get(key), live) {
            (Some(OneRunValue::Bool(p)), toml_edit::Value::Boolean(v)) => p == v.value(),
            (Some(OneRunValue::Int(p)), toml_edit::Value::Integer(v)) => p == v.value(),
            (Some(OneRunValue::Str(p)), toml_edit::Value::String(v)) => p == v.value(),
            _ => false,
        }
    }
}

// ── Entropy ───────────────────────────────────────────────────────────────────

/// A fresh, unpredictable 32-bit seed for an engine's PRNG (SQ-0811).
///
/// No dependency and no `unsafe`: `std::collections::hash_map::RandomState` is
/// the hasher std seeds `HashMap` with, and the OS randomises its keys once per
/// process (each later `RandomState::new()` in a thread steps them again), so
/// hashing a fixed salt through one yields a value that differs between launches
/// AND between calls. That second property is what lets a restart deal a new
/// game. Cross-platform for free — std does the platform work.
///
/// Never returns 0: every engine here runs xorshift32, which is an absorbing
/// state at zero and would then return zero for the rest of the session.
pub fn entropy_seed() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u32(0x9E37_79B9); // fixed salt; the entropy is in the random keys
    // Mix both halves rather than truncating: RandomState's low word carries the
    // per-call step, and folding the high word in keeps the whole hash's spread.
    let full = h.finish();
    let s = (full as u32) ^ ((full >> 32) as u32);
    if s == 0 { 0x2BAD_C0DE } else { s }
}

/// Current `config.toml` schema version. Bump when a config change means an
/// older hand-written file may behave unexpectedly. `write_config` stamps this
/// as `version = N`; a future lanthorn can compare a file's `version` against
/// this to flag an out-of-date config. A file with no `version` reads as 0.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// User preferences loaded from TOML.  Every field has a default so a missing
/// config file (or a file with only some fields) is always valid.
///
/// ADDING A PERSISTED FIELD — touch ALL of these or it silently half-works:
///   1. this struct (with a `#[serde(default …)]`);
///   2. `impl Default for Config` AND the test-builder `Config { … }` literal
///      further down (both are full literals — a new field must be listed in
///      each or the crate won't compile, which is the good case);
///   3. `resolve`'s field-by-field merge from `from_file` — MISS THIS and a
///      value in the file is ignored (default always wins on load);
///   4. `write_config`'s `doc.put("…", …)` — MISS THIS and a settings-panel edit
///      is never written, so it reverts to the default on the next launch.
///
/// Steps 3 and 4 fail SILENTLY (they still compile); the round-trip test
/// `write_config_persists_panel_editable_scalars_round_trip` guards the class.
/// Runtime-only fields (`#[serde(skip)]`, e.g. `acceleration`) skip 3 and 4.
///
/// AND if anything can set the field for ONE LAUNCH — a CLI flag, this game's
/// sidecar, a value inferred from the story or its artwork — pin it as it lands
/// (see [`OneRunOverrides`], and add the key to [`keys`]). Miss that and the next
/// settings save bakes the throwaway choice into the user's global config.
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    /// Schema stamp (see [`CONFIG_SCHEMA_VERSION`]). A file written before
    /// versioning has no `version` key and reads as 0.
    #[serde(default)]
    pub version: u32,
    /// Root directory for lanthorn data (maps, saves, exports).
    /// Sub-directories: maps/ — where per-story map files live.
    #[serde(default = "default_user_dir")]
    pub user_dir: PathBuf,
    /// Directory (or story file) opened when lanthorn is launched with no path
    /// argument. `None` (default) means a path is required on the command line.
    #[serde(default)]
    pub default_story_dir: Option<PathBuf>,
    /// When true (default), restore the game state from the archive on startup so
    /// play resumes where it left off. Set false to start a fresh playthrough while
    /// retaining the accumulated map.
    #[serde(default = "default_true")]
    pub auto_load: bool,
    /// When true, save the archive after every game turn (in addition to the
    /// exit-save and Ctrl+S quick-save). Default false.
    #[serde(default)]
    pub auto_save: bool,
    /// When true, invert mouse-wheel scroll direction (for terminals reporting
    /// "natural" scrolling). Default false = conventional direction.
    #[serde(default)]
    pub mouse_wheel_invert: bool,
    /// When true, capture the mouse (click-to-select in the story browser and
    /// map, wheel scrolling, and Glk mouse input to games that request it).
    /// Default true (SQ-0298): in-app mouse support is on out of the box. Set
    /// `mouse = false` to disable it — mouse capture puts the terminal in
    /// any-motion reporting mode (every movement drives a redraw) and overrides
    /// the terminal's native text selection.
    #[serde(default = "default_true")]
    pub mouse: bool,
    /// When true, edit story commands in a persistent command bar instead of
    /// the inline story-text prompt. Default false: the inline prompt.
    #[serde(default)]
    pub command_bar: bool,
    /// When true (default) and auto_save is off, prompt the user to save on quit.
    #[serde(default = "default_true")]
    pub prompt_save_on_quit: bool,
    /// When true (default) and auto_load is off, prompt the user to resume a found save on launch.
    #[serde(default = "default_true")]
    pub prompt_load_on_launch: bool,
    /// When true, record a per-turn rewind/replay history (Quetzal save + map
    /// snapshots) into the `.lanthorn` archive. Default false (opt-in: it grows
    /// the archive and keeps per-turn blobs in memory).
    #[serde(default)]
    pub record_turn_history: bool,
    /// How many of the most recent turns `record_turn_history` retains before
    /// evicting the oldest (SQ-1185) — bounds memory on a long session rather
    /// than growing without limit. No "unbounded" setting: a value below 1 is
    /// clamped to 1, since the whole point of this key is a guaranteed bound.
    #[serde(default = "default_history_turns")]
    pub history_turns: usize,
    /// When true (default), auto-skip the InvisiClues "your screen is only N
    /// characters wide…" banner that izm hint files print at startup, landing the
    /// player straight on the topic menu. Set false to see the banner and dismiss
    /// it yourself.
    #[serde(default = "default_true")]
    pub hint_skip_screen_warning: bool,
    /// Lanthorn's Guiding Light: whether lanthorn offers the player help *while
    /// they play* — a vocabulary the parser knows, a completed noun, a caution
    /// before a move that cannot be undone (SQ-1045).
    ///
    /// On by default, and one switch for the whole set rather than one per
    /// feature: a player who does not want the interpreter talking over the story
    /// wants none of them, and a player who does is not going to enumerate five.
    /// `crate::state::AppState::push_assist` is where it is read, which is the one
    /// door every assist goes through.
    ///
    /// `--guidance on|off` says it for a launch, `/set-guidance` for a session,
    /// and the settings screen persists it. The ●/○ control in the pane border is
    /// the switch a player actually finds, which is why the introduction line no
    /// longer spends words pointing at a menu.
    #[serde(default = "default_true")]
    pub guidance: bool,
    /// Vet a vocabulary offer before it is shown, by trying each candidate in a
    /// silent throwaway copy of the game and keeping only what actually did
    /// something (SQ-1121).
    ///
    /// On by default, and a switch of its own rather than part of `guidance`,
    /// because it is a different KIND of thing: `guidance` decides whether
    /// lanthorn speaks, this decides whether it runs the player's game a few
    /// extra turns in private first. Someone will want the light on and the
    /// speculation off, and they are entitled to.
    ///
    /// Off, an offer still appears — it just makes the weaker claim it can
    /// support, naming what the story's dictionary holds rather than
    /// recommending anything (see `crate::vocab`).
    #[serde(default = "default_true")]
    pub guidance_probe: bool,
    /// After a move, discover the way BACK in a silent copy of the game, so the
    /// automap closes one-way gaps without inventing reciprocity (SQ-0785).
    ///
    /// **On by default** since the probe seam got cheap (SQ-1177/SQ-1178: the
    /// snapshot is shared with the turn's own bookkeeping, refused before it is
    /// paid for when the worker is busy, and cloned by Arc per direction) — a
    /// map that closes its own gaps is the automap working as advertised, and a
    /// probe that lands anywhere but the room it left records nothing at all.
    ///
    /// Not part of [`guidance`](Self::guidance), and not part of
    /// [`guidance_probe`](Self::guidance_probe) either: neither of those speaks to
    /// the player, and this one does not speak at all. It edits the MAP.
    ///
    /// `/set-return-probe` says it mid-game and persists it per-game, which is
    /// where a preference about how much work a particular story is worth
    /// belongs.
    #[serde(default = "default_true")]
    pub return_probe: bool,
    /// Keep [`adult_words`](Self::adult_words) out of any panel that ENUMERATES
    /// a story's vocabulary unprompted (SQ-1122). Default true.
    ///
    /// The switch is separate from the list so that turning the filter off does
    /// not destroy it: `hide_adult_words = false` restores the full column and
    /// leaves the words where the player can still read them, and a
    /// settings-screen row has a boolean to flip rather than a list it could not
    /// edit.
    #[serde(default = "default_true")]
    pub hide_adult_words: bool,
    /// The words themselves — see [`DEFAULT_ADULT_WORDS`] for what the default
    /// holds and why. Shipped UNCOMMENTED in the seeded `config.toml`: a filter
    /// nobody can inspect is censorship, one written out in the player's own
    /// config file is a default. Emptying it restores the full column exactly as
    /// `hide_adult_words = false` does.
    #[serde(default = "default_adult_words")]
    pub adult_words: Vec<String>,
    /// Controls automatic background re-tidy when new rooms are discovered.
    /// Default: EveryRoom (re-tidy on each turn that finds a new room).
    #[serde(default)]
    pub background_tidy: BackgroundTidy,
    /// Where to persist v5 auxiliary save data. Default: Ask.
    #[serde(default)]
    pub aux_storage: AuxStorage,
    /// How the v6 graphical story pane is rendered. Default: Hybrid.
    #[serde(default, deserialize_with = "deserialize_v6_render")]
    pub v6_render: V6RenderMode,
    /// Fuse a 640-wide rendition's colour dither, because the card's pixels were
    /// half as wide as the unit screen's (SQ-0797). Default: true.
    ///
    /// EGA's sixteen colours were fixed in the silicon, so its artists dithered
    /// for the ones they did not have — Zork Zero's bronze arch is a
    /// column-by-column alternation of brown and bright red, and on a 640x200
    /// screen those columns fused in the eye into a colour the palette does not
    /// hold. lanthorn keeps all 640 columns (that is what makes an EGA plate
    /// cover exactly the rectangle a 320-wide one does), so it fuses them itself,
    /// in `crate::graphics::blend_half_width_columns`.
    ///
    /// Set false to see the archive's own pixels instead — every column distinct,
    /// dither and all (SQ-0816). Nothing else changes: this is a preference about
    /// one filter, not about which artwork loads. Two-colour CGA line work is
    /// never fused either way, and 320-wide MCGA and Amiga art has nothing at this
    /// frequency to fuse.
    #[serde(default = "default_true")]
    pub fuse_art_dither: bool,
    /// Divisor applied to the terminal's reported cell size before a Glulx game
    /// sees it, so a game's fixed pixel sizes keep their proportions on a scaled
    /// display or with a large font (SQ-0593). Default: Auto.
    #[serde(default)]
    pub glk_pixel_scale: GlkPixelScale,
    /// When true, forward arrow keypresses to a v6 story as ZSCII cursor codes
    /// (129-132), so a game that binds arrows to movement can read them.
    /// Default false (SQ-1087) — arrows are lanthorn's own scrollback and map
    /// panning everywhere else, and a v6 story is the one place that stopped
    /// being true, which reads as the app going deaf rather than as a setting.
    /// Withholding only ever costs a shortcut for a command the player can
    /// still type; forwarding costs a key that works in every other story.
    ///
    /// Only affects v6 — v1-5/Glulx stories always get arrows regardless — and
    /// only at a line (`>`) prompt: v6 menus and "press any key" screens get
    /// arrows whatever this says, or they are unnavigable (SQ-0483).
    #[serde(default)]
    pub v6_arrow_keys: bool,
    /// Snap the v6 hybrid letterbox magnification to the ladder the ARTWORK
    /// implies, so one art pixel is always a whole number of device pixels
    /// (SQ-0936). Default false — free scaling uses the pane better, and this
    /// trades screen area for crispness.
    ///
    /// The ladder's step is `1 / gcd(art_scale)` (see
    /// `crate::render::v6_layout::scale_ladder_step`), which is half-steps for a
    /// 320-wide rendition and whole steps for the standard Macintosh's mono plate
    /// and the 640-wide EGA/CGA ones. Arbitrary fractional scaling is what produces
    /// the resample softness and the ceil-vs-round seams; a rung of the ladder is
    /// nearest-neighbour-exact and makes every tiled flank's repeat integral too.
    ///
    /// A pane too small for even the smallest rung falls back to free scaling
    /// rather than blocking.
    ///
    /// **It is a device-pixel guarantee, so it only binds a backend that has
    /// device pixels** (SQ-0978). Kitty, iTerm2 and sixel put the composite on the
    /// screen at the pane's real resolution. Half-blocks resolves it into CELLS —
    /// one sample per column, two per row — and the font size the rung was counted
    /// in is `Picker::halfblocks`'s hardcoded 10x20, so there is nothing there for
    /// an art pixel to land a whole number of. The lock is inert on that backend
    /// and `/dump-terminal` reports it as inert; see
    /// `crate::render::graphics::v6_pixel_lock_applies` for the measurement.
    #[serde(default)]
    pub v6_pixel_lock: bool,
    /// Which of the player's own boot media under `~/.lanthorn/` answers first
    /// when several carry the machine's system typeface (SQ-1037, SQ-1053).
    ///
    /// **Media, not only disks**: an Amiga Kickstart ROM (`*.rom`) is read here
    /// too, and it is the only place topaz 8 exists — the Amiga's real Version 6
    /// body face is in ROM and on no floppy. One key covers both because the rule
    /// is the same one, a substring of the FILENAME, and a second key would be a
    /// second rule to keep in step.
    ///
    /// A case-insensitive SUBSTRING of the file's name — `1.3` picks the
    /// Workbench 1.3 floppy out of a directory holding both. **Empty is the
    /// default and means no preference**, which is why this is a `String` rather
    /// than an `Option`: the template documents `system_font_disk = ""`, and a
    /// key whose written default cannot round-trip to the default VALUE is what
    /// `config_template`'s own test exists to catch.
    ///
    /// # It breaks a tie; it does not choose the disk
    ///
    /// Every disk of the right kind is read and its faces pool, because every
    /// pick-one rule is bad in a way a player can see: first-found is filesystem
    /// order, newest-version needs a version parsed off a name they may have
    /// renamed, most-fonts is arbitrary. So this only orders the pool, and a
    /// preferred disk that does not carry the face being asked for falls through
    /// to the rest rather than losing it. Absent a preference the pool is ordered
    /// by filename, which is stable and visible.
    ///
    /// Worth setting when two disks carry the same face from different releases of
    /// the operating system — a System 7 Geneva is not the 1988 one — and worth
    /// leaving alone otherwise: Workbench 1.2 and 1.3 ship IDENTICAL font drawers,
    /// so on those two the key changes nothing but the name in the report.
    #[serde(default)]
    pub system_font_disk: String,
    /// Keymap overrides: command_name → key-spec string(s).
    #[serde(default)]
    pub keymap: KeymapConfig,
    /// Hotkey dialog configuration: prefix key, direct commands, dialog groups.
    #[serde(default)]
    pub hotkeys: HotkeysConfig,
    /// Style-file pointer: a built-in name, a file path, or absent (use
    /// `user_dir/style.toml` if present, else the built-in default).
    #[serde(default)]
    pub style: Option<String>,
    /// Watch the resolved style.toml and live-reload it on change (default false).
    #[serde(default)]
    pub watch_style: bool,
    /// The font check is still owed: a launch that would have asked could not
    /// (SQ-1112). Written by lanthorn, never hand-set — see `write_config_file`.
    ///
    /// **The default must stay `false`**, and that is the whole design. "There is
    /// no config.toml" is the first-run flag, so the test harnesses seed an EMPTY
    /// one to make themselves not-a-first-run — and an empty file parses with
    /// every key at its default. A key defaulting to "still owed" would put the
    /// prompt straight back in front of fourteen group binaries, `gallery` and
    /// `pty_capture`, which is exactly the guard SQ-1104 had to build. Owing the
    /// question is therefore opt-IN: absence means nothing is owed, and only a
    /// launch that actually failed to ask ever writes it.
    #[serde(default)]
    pub font_check_pending: bool,
    /// Undo depth: max retained in-memory undo snapshots (default 16; 0 disables).
    #[serde(default = "default_undo_levels")]
    pub undo_levels: usize,
    /// The prefix character that triggers slash-command routing (default: '/').
    /// Stored as a single-character string in TOML: command_prefix = "/".
    #[serde(default = "default_command_prefix", deserialize_with = "deserialize_char_from_str")]
    pub command_prefix: char,
    /// When true, room numbers (#id) are shown in Boxes-zoom room boxes.
    /// Default false (hidden); toggled at runtime by ToggleRoomNumbers.
    #[serde(default)]
    pub show_room_numbers: bool,
    /// Show the status/score bar (top row of the story pane). Default true.
    /// The v3 status line (location/score/moves) is only meaningful for v3
    /// games; for v4+ (which draw their own upper-window status) it reads
    /// garbage globals, so this can be toggled off (ToggleStatusBar).
    #[serde(default = "default_true")]
    pub show_status_bar: bool,
    /// Search configuration: start direction, nav keys.
    #[serde(default)]
    pub search: SearchConfig,
    /// OVERRIDE for the screen width (in characters) reported to the Z-machine in
    /// header byte $21. Unset (the default) means "follow the story pane": ZMSD
    /// §8.4 requires the interpreter to "write the current height (in lines) and
    /// width (in characters) into bytes $20 and $21", and it "may change the exact
    /// dimensions whenever it likes", so lanthorn reports the pane's real measured
    /// size and re-reports it on every terminal resize. Set this key only to pin a
    /// fixed virtual screen (e.g. to reproduce a game's original 80-column
    /// layout); a pinned width no longer matches what the transcript wraps at, so
    /// the upper window will be drawn centred inside the pane. (SQ-0532/A-F1)
    #[serde(default)]
    pub virtual_screen_cols: Option<u16>,
    /// OVERRIDE for the screen height (in lines) reported in header byte $20.
    /// Unset (the default) follows the story pane — see `virtual_screen_cols`.
    #[serde(default)]
    pub virtual_screen_rows: Option<u16>,
    /// Story pane's share of the story/map Split, as a percentage (default 50).
    #[serde(default = "default_split_ratio")]
    pub split_ratio: u16,
    /// The `[command_panel]` section: the command panel's height, whether
    /// it auto-opens, and its verb grammar / quick row. (SQ-0664 retired the
    /// old `verb_dock_pct` key along with the left dock it sized.) The Rust
    /// field keeps its `command_band` name (an internal identifier); only the
    /// TOML section it (de)serialises to is `command_panel` (SQ-1237).
    #[serde(default, rename = "command_panel")]
    pub command_band: CommandBandConfig,
    /// Inventory panel height cap as a percentage of screen height (default 33,
    /// ≈ the old fixed 1/3 cap).
    #[serde(default = "default_inv_dock_pct")]
    pub inv_dock_pct: u16,
    /// Room panel height as a percentage of screen height (default 33). The
    /// panel is carved out of the MAP pane's bottom, but its size is measured
    /// against the frame so both panels share one unit (SQ-0692).
    #[serde(default = "default_room_dock_pct")]
    pub room_dock_pct: u16,
    /// Inner margin reserved inside the text-buffer (transcript) window, in
    /// character cells: `text_margin_x` blank columns on each side,
    /// `text_margin_y` blank rows top and bottom. Default 0. Populated from
    /// garglk `tmarginx`/`tmarginy` (converted px→cells) when a garglk config
    /// is imported (SQ-0344).
    #[serde(default)]
    pub text_margin_x: u16,
    #[serde(default)]
    pub text_margin_y: u16,
    /// Animation engine settings: enable switch, easing curve, scroll duration.
    #[serde(default)]
    pub animation: AnimationConfig,
    /// When true (default), honor game-set colours in the transcript and upper
    /// window. Set false to use only the configured color scheme.
    #[serde(default = "default_honor_game_colours")]
    pub honor_game_colours: bool,
    /// When true (default), paint a **v1–v4** story the way its machine's own
    /// interpreter did — its page and ink, its status band, its cursor (SQ-0873).
    ///
    /// Narrower than `honor_game_colours` and deliberately separate from it. A
    /// v1–v4 story has no colour concept at all (`set_colour` and the `$2C`/`$2D`
    /// bytes are v5+), so what a machine drew for one is presentation rather than
    /// a fact the story can read — and declining the presentation must not also
    /// cost a v5+ story the colours it asked for. `honor_game_colours = false`
    /// still takes this with it; the reverse does not hold. See
    /// [`crate::period`], which holds the whole gate and the reasoning.
    ///
    /// On by default here and OFF in `zvm-cli`: lanthorn paints a pane it owns,
    /// and opening Zork I off an `.adf` and having it look like an Amiga is the
    /// point. The CLI writes into a terminal belonging to the user.
    #[serde(default = "default_period_look")]
    pub period_look: bool,
    /// When true (default), honor the Z-machine's timed-input (`read`/`read_char`
    /// `time`+`routine` operands). Set false to treat all reads as untimed.
    #[serde(default = "default_honor_timed_input")]
    pub honor_timed_input: bool,
    /// Interpreter number to advertise (header 0x1E). `None` = auto (Frotz's rule:
    /// 1 for v1-5, 6 for v6). Set to override, e.g. 6 for BeyondZork's IBM PC
    /// character-graphics instead of colour. The full ZMSD §11.1.3 value table is
    /// on `Cli::interpreter_number`, which overrides this for one run.
    #[serde(default)]
    pub interpreter_number: Option<u8>,
    /// The seed every engine's random-number generator starts from (SQ-0811).
    ///
    /// `None` — the default — means "a different game every launch": lanthorn
    /// draws a fresh seed from the OS at boot, which is what Frotz, Glulxe and
    /// Git all do and what a randomised game like Kerkerkruip needs to be a
    /// different game twice. Set it to any number to PIN the run instead, so the
    /// same story replays the same shuffles, the same dice and the same dungeon —
    /// the seed lanthorn reports on the console at startup is exactly the value
    /// to put here to play that run again.
    ///
    /// A pinned seed does not gag a game that asks for entropy itself: Glulx's
    /// `setrandom(0)` is specified to reseed from the system and still does.
    #[serde(default)]
    pub random_seed: Option<u32>,
    /// The config file this `Config` was READ from, so a later save goes back to the
    /// same file rather than to `user_dir` (SQ-0574). Set by [`resolve`]; never
    /// persisted, and not part of the file's schema.
    #[serde(skip, default = "default_config_file")]
    pub config_file: PathBuf,
    /// Set when the config file exists but does not LOAD: the error, carried so
    /// startup can say what broke instead of running on defaults in silence
    /// (SQ-0580). One bad line costs the user the WHOLE file — TOML is parsed as one
    /// document, so there is no partial load to fall back to. Never persisted, and not
    /// part of the file's schema.
    ///
    /// Both failure modes land here (SQ-0645): a *syntax* error (`style = "neon`) and
    /// a *type* error (`volume = 300`, `auto_load = "yes"`) are the same event as far
    /// as this struct is concerned — every field is at its default and none of the
    /// file's values are in memory. Only the syntax case is visible to `toml_edit`,
    /// which is why [`write_config_at`] gates on this field rather than on re-parsing.
    #[serde(skip)]
    pub config_error: Option<String>,
    /// Which keys above a one-run source pinned for this launch, and to what —
    /// see [`OneRunOverrides`] (SQ-0646, SQ-0806, SQ-0807). Never persisted, and
    /// not part of the file's schema.
    #[serde(skip)]
    pub one_run: OneRunOverrides,
    /// The machine lanthorn presents itself to this story as (SQ-0719).
    ///
    /// Not a config key — it is INFERRED per story at boot by
    /// [`crate::interpreter::InterpreterProfile::resolve`] (an explicit
    /// `interpreter_number` names it outright, else the medium the story came
    /// out of decides, else IBM PC) and parked here because it rides with the
    /// story for the rest of the session: the restart path rebuilds the engine
    /// from it, and the per-tick default-colour poller has to know whether the
    /// profile pins those colours or the terminal supplies them. Never persisted.
    #[serde(skip)]
    pub interpreter_profile: crate::interpreter::InterpreterProfile,
    /// How that profile was arrived at, which decides whether the machine's own
    /// colours may be presented (SQ-0928). Inferred with it, never persisted.
    #[serde(skip)]
    pub interpreter_source: crate::interpreter::ProfileSource,

    /// Present the MACHINE's own colours even when the story did not come off its
    /// original media (SQ-0928).
    ///
    /// Off by default, and the default is the whole point. A machine's §8.3.3 pair
    /// is a fact about a machine — the IBM PC's is blue under white, the Amiga's a
    /// grey under white — and running a story off its release disk makes that fact
    /// true of the launch. Opening a bare `.z5` does not: `InterpreterProfile`
    /// answers `IbmPc` there because nothing named a machine, and painting every
    /// modern Inform story blue on the strength of that would be absurd.
    ///
    /// Turning this on says "I know what I am asking for": with `--interpreter 4`
    /// it gets you the Amiga's page on a bare file, which is what
    /// `docs/internals/interpreter.md` used to promise unconditionally.
    ///
    /// It cannot conjure a machine out of nothing — see
    /// [`ProfileSource::Fallback`](crate::interpreter::ProfileSource::Fallback).
    #[serde(default = "default_system_colours")]
    pub system_colours: bool,
    /// Which of the three default-colour sources this launch draws its page and
    /// ink from — `--colour terminal|theme|machine` (SQ-1082).
    ///
    /// Not a config key: it is an instruction for the launch you typed it on,
    /// like `--interpreter`, and the persisted half of the same subject is
    /// `system_colours` above. [`ColourSource::Machine`] is the default because
    /// it IS the chain that runs when nothing is said; naming it on the command
    /// line additionally sets `system_colours` for the run.
    #[serde(skip)]
    pub colour_source: ColourSource,
    /// The picture archive named for THIS launch — `--pictures`, or a choice the
    /// launch-options dialog made and the user did not persist (SQ-0789/0791).
    ///
    /// Parked here for the same reason `interpreter_profile` is: it rides with
    /// the story for the session. The restart path re-resolves the picture source
    /// from the sidecar, and without this a session-only choice would silently
    /// revert to the Blorb the moment the player restarted — which is exactly the
    /// "plausible but wrong art, with nothing on screen to say so" failure the
    /// tier policy exists to prevent, caused by us. Never persisted; the dialog's
    /// checkbox is what writes a choice down.
    #[serde(skip)]
    pub pictures_override: Option<String>,
    /// Which story on the disk image this launch opened — the browser row's own
    /// name, as [`blorb::medium::DiskStory`] spells it (SQ-0876).
    ///
    /// Not a config key, and parked here for the reason `interpreter_profile`
    /// and `pictures_override` are: it rides with the story for the session. The
    /// restart path re-resolves the picture source, and on a disc that keeps its
    /// games in folders the answer is this story's rather than the platter's —
    /// without it a restart of Journey off the Masterpieces CD would come back
    /// drawing Arthur's plates. `None` for a loose story file and for a
    /// single-game floppy, which is every release but a compilation. Never
    /// persisted.
    #[serde(skip)]
    pub disk_entry: Option<String>,
    /// When true (default), play audio for `sound_effect` (bleeps + Blorb samples).
    #[serde(default = "default_enable_sound")]
    pub enable_sound: bool,
    /// Master audio volume 0..=100 (default 100). Combined with the game's per-sound
    /// Z-scale volume.
    #[serde(default = "default_volume")]
    pub volume: u8,
    /// Whether Glulx accel interception is active. Runtime-only (set from the
    /// --accel off CLI flag); intentionally not persisted or user-facing.
    #[serde(skip, default = "default_acceleration")]
    pub acceleration: bool,
    /// Cover-art image protocol. Runtime-only (set from --image-protocol);
    /// not persisted or user-facing.
    #[serde(skip, default = "default_image_protocol")]
    pub image_protocol: ImageProtocol,
    /// Whether image rendering (in-game graphics + cover art) is enabled.
    /// Runtime-only (set from --images); not persisted.
    #[serde(skip, default = "default_images")]
    pub images: bool,
    /// Active debug-trace sections. Runtime-only (from --trace / /trace); not persisted.
    #[serde(skip)]
    pub trace: crate::trace::TraceSections,
}

impl Config {
    /// The words no unprompted enumeration may show — the adult list when the
    /// switch is on, and nothing at all when it is off or the list is empty
    /// (SQ-1122).
    ///
    /// Both off-switches answer the same empty slice on purpose, so a caller has
    /// one question to ask and cannot honour one of them and forget the other.
    pub fn hidden_display_words(&self) -> &[String] {
        if self.hide_adult_words { &self.adult_words } else { &[] }
    }

    /// The command band's VERB column as the band is BORN — `[command_panel]`'s
    /// own resolution with [`for_display`](Self::for_display) applied to
    /// whatever came out.
    ///
    /// This and [`layer_band_verbs`](Self::layer_band_verbs) are the only two
    /// ways a `VerbTable` reaches the screen, and they live on `Config` rather
    /// than on `CommandBandConfig` because the list is a top-level key: the band
    /// section cannot see it, and a filter applied by each call site in turn is
    /// a filter the next call site forgets.
    /// `every_verb_table_in_src_is_assembled_through_config` in
    /// `tests/suites/adult_words.rs` fails if `src/` grows a third one.
    pub fn resolve_band_verbs(
        &self,
    ) -> (crate::render::command_band::VerbTable, Vec<String>) {
        let (table, warnings) = self.command_band.resolve_verbs();
        (self.for_display(table), warnings)
    }

    /// The same for the table the story's own grammar produces a tick later —
    /// `[command_panel] extra_verbs` layered on, then
    /// [`for_display`](Self::for_display).
    ///
    /// The filter runs AFTER `extra_verbs`, so it catches a word the player's own
    /// list re-added as surely as one the story's grammar named. Somebody who
    /// deliberately types a word into `extra_verbs` and wants it shown has the
    /// two off-switches; the alternative reading would make `extra_verbs` a way
    /// past a default nothing announces.
    pub fn layer_band_verbs(
        &self,
        table: crate::render::command_band::VerbTable,
    ) -> crate::render::command_band::VerbTable {
        self.for_display(self.command_band.layer_extra_verbs(table))
    }

    /// The Guiding Light's offer line, minus any word lanthorn would be saying
    /// in its OWN voice against this config (SQ-1145).
    ///
    /// The offer answers a word the player reached for, and SQ-1115 rules that
    /// half: a correction is never censored, so `molst` → `molest` survives
    /// every setting here. But not every pick is a correction. The meaning table
    /// proposes a DIFFERENT word from what the typed one means — `sod` → `fuck`
    /// on Zork I — and that is lanthorn choosing the word, not the player. It is
    /// nearer the band's unprompted enumeration than the near miss it sits
    /// beside, so it answers to the same list.
    ///
    /// [`Pick::proposed`](crate::vocab::Pick::proposed) is the whole test, which
    /// is why the filter lives here and the provenance travels: `vocab.rs` holds
    /// no judgement about words and reads no configuration, and a source-level
    /// case in `tests/suites/adult_words.rs` fails it if it ever starts.
    ///
    /// A line emptied by this says nothing at all, which is the caller's
    /// existing answer to an empty offer and needs no new rule.
    pub fn spoken_offer(&self, picks: Vec<crate::vocab::Pick>) -> Vec<String> {
        let hidden = self.hidden_display_words();
        picks
            .into_iter()
            .filter(|p| !p.proposed || !hidden.iter().any(|h| h.eq_ignore_ascii_case(&p.word)))
            .map(|p| p.word)
            .collect()
    }

    /// Everything an assembled VERB column is filtered through before it
    /// reaches a screen — the ONE place, so that the two wrappers above cannot
    /// drift apart and a third rule has somewhere obvious to go.
    ///
    /// Two rules, deliberately kept apart in kind:
    ///
    /// * [`without_sigil_verbs`](crate::render::command_band::VerbTable::without_sigil_verbs)
    ///   is STRUCTURE — Infocom's `#record`/`$verify` test rig is not part of
    ///   the game in any story, needs no vocabulary, and takes no switch
    ///   (SQ-1126).
    /// * [`hiding`](crate::render::command_band::VerbTable::hiding) is a
    ///   JUDGEMENT — the adult list, which is therefore shipped visibly in
    ///   `config.toml` with two off-switches (SQ-1122).
    ///
    /// Both are display-only: every word either one removes still parses, and
    /// the Guiding Light still offers it when the player REACHED for it — see
    /// [`spoken_offer`](Self::spoken_offer) for the one half that now answers to
    /// the adult list as well, which is lanthorn proposing a word of its own.
    fn for_display(
        &self,
        table: crate::render::command_band::VerbTable,
    ) -> crate::render::command_band::VerbTable {
        table.without_sigil_verbs().hiding(self.hidden_display_words())
    }

    /// The machine's §8.3.3 default page and ink for THIS launch, or `None` when
    /// this launch has not earned them (SQ-0928).
    ///
    /// [`InterpreterProfile::default_colours`](crate::interpreter::InterpreterProfile::default_colours)
    /// is the machine's own fact and answers the same way for everybody; this is
    /// the question every renderer and the header seeder should actually ask,
    /// because a machine reached by falling through is not a machine the player
    /// is on.
    ///
    /// **The story READS this** in `$2C`/`$2D`, so it is not a paint: whatever
    /// this answers has to be what the screen shows, or a v5+ game asking for the
    /// default pair is told one thing and shown another.
    /// The number to advertise in header `$1E`, for THIS launch (SQ-0930).
    ///
    /// [`InterpreterProfile::interpreter_number`] answers `None` for the IBM PC on
    /// purpose, so zvm's own version rule (Frotz's 6-for-v6, 1-otherwise) stays in
    /// force and naming the profile cannot change what the corpus advertises. That
    /// is right for the FALLBACK — a story with no medium is not on any machine —
    /// and wrong when a **DOS medium named it**, where 1 tells the story it is a
    /// DECSystem-20 on the one disk that unambiguously says otherwise.
    ///
    /// **This is not inert and the change is deliberate.** `blorb::medium`'s DOS
    /// row records the reason it declined to state 6: *Beyond Zork* swaps Font 3's
    /// arrows for CP437 character graphics when it believes it is on an IBM PC, and
    /// `BEYONDZO.DAT` sits on `floppy1.ima`. That is a visible rendering change on
    /// real media — and it is what the Lost Treasures DOS release actually did, so
    /// a launch off that floppy should have it.
    pub fn advertised_interpreter_number(&self) -> Option<u8> {
        if let Some(n) = self.interpreter_number {
            return Some(n);
        }
        if self.interpreter_profile == crate::interpreter::InterpreterProfile::IbmPc
            && self.interpreter_source == crate::interpreter::ProfileSource::Medium
        {
            return Some(crate::interpreter::IBM_PC_INTERPRETER_NUMBER);
        }
        self.interpreter_profile.interpreter_number()
    }

    /// May this launch present its machine at all? (SQ-0928)
    ///
    /// The medium named the machine, or the player named it and opted in. Both
    /// [`Self::machine_default_colours`] and `crate::period::resolve` ask this —
    /// a `$2C`/`$2D` pair and a period look are the same kind of claim about the
    /// same machine, and it would be incoherent to license one and not the other.
    ///
    /// **And `--colour` decides the REGIME before the medium is asked** (SQ-1154).
    /// [`ColourSource`] names which of the three sources this launch draws its
    /// default page and ink from, and the user's rule is symmetrical: `machine`
    /// on a bare story file is the media path applied to a raw file — SQ-0928's
    /// opt-in, which is what `system_colours` still carries — and
    /// `theme`/`terminal` on original media is the raw path applied to a medium.
    /// So those two arms answer **no** here however the profile was arrived at:
    /// this launch does not present its machine, exactly as a bare `.z6` does
    /// not.
    ///
    /// Withholding the licence is the WHOLE of that change, because every
    /// machine-colour question already hangs off this one predicate — the
    /// `$2C`/`$2D` pair, the two-colour card's pair (and with it
    /// [`crate::graphics::PictSource::two_colour_card_screen`], so
    /// `Palette::IbmCga` is never installed), the machine's colour-number table
    /// in `startup.rs`, and the period look. The artwork is untouched: pictures
    /// resolve through `graphics`'s own per-picture palette, read from the
    /// archive, and never through `zvm::screen`'s.
    ///
    /// A story's own `set_colour` then resolves through §8.3.1's table rather
    /// than the machine's, which is what it does on the raw path and is the
    /// regime being read consistently, not a colour lost: `honor_game_colours`
    /// is the axis that decides whether those requests are obeyed at all.
    pub fn machine_colours_licensed(&self) -> bool {
        match self.colour_source {
            // Named on the command line, `Machine` additionally sets
            // `system_colours` for the run — see `resolve`.
            ColourSource::Machine => {
                self.interpreter_source.licenses_machine_colours(self.system_colours)
            }
            ColourSource::Theme | ColourSource::Terminal => false,
        }
    }

    /// The table this launch's colour NUMBERS resolve through — the machine's
    /// when it is licensed to present one, and §8.3.1's when it is not
    /// (SQ-0939, SQ-1154).
    ///
    /// `zversion` is the story's header byte 0, because Infocom shipped two IBM
    /// interpreters whose tables differ (XZIP's white is EGA 7, YZIP's is 15) —
    /// see [`zvm::interpreter::palette_for`], which is the machine's own answer
    /// and the only thing this adds the licence to.
    ///
    /// One function because two callers must not drift: `startup.rs` installs it
    /// process-wide before the session constructor runs the story, and the suites
    /// that measure a booted frame have to boot under the same table. A harness
    /// that re-derived this would keep passing while the shipped path regressed.
    pub fn machine_text_palette(&self, zversion: Option<u8>) -> zvm::screen::Palette {
        if self.machine_colours_licensed() {
            zvm::interpreter::palette_for(self.interpreter_profile.row_number(), zversion)
        } else {
            zvm::screen::Palette::Standard
        }
    }

    pub fn machine_default_colours(&self) -> Option<(u8, u8)> {
        self.machine_colours_licensed()
            .then(|| self.interpreter_profile.default_colours())
            .flatten()
    }

    /// The same claim about a narrower screen: the pair this launch's machine
    /// states when its display is showing **two colours** (SQ-0956).
    ///
    /// Licensed by the same rule and for the same reason —
    /// [`crate::interpreter::InterpreterProfile::two_colour_colours`] is a fact
    /// about a machine, so a launch that never named one gets `None` here as it
    /// gets `None` above, and a `.cg1` opened beside a bare `.z6` keeps SQ-0806's
    /// behaviour exactly.
    ///
    /// Whether the launch is actually SHOWING that display is the archive's to
    /// say, not the machine's: see
    /// [`crate::graphics::PictSource::two_colour_card_screen`], the only caller.
    pub fn machine_two_colour_colours(&self) -> Option<(u8, u8)> {
        self.machine_colours_licensed()
            .then(|| self.interpreter_profile.two_colour_colours())
            .flatten()
    }

    /// True while `interpreter_number` is still the one a one-run source pinned
    /// — `--interpreter`, the launch-options dialog, or this game's own
    /// sidecar — and nothing has changed it (SQ-0646/0789). A convenience over
    /// [`OneRunOverrides`] for the callers that ask about this one key by name.
    pub fn interpreter_number_from_cli(&self) -> bool {
        self.interpreter_number
            .is_some_and(|n| self.one_run.int(keys::INTERPRETER_NUMBER) == Some(i64::from(n)))
    }

    /// The seed to start this engine's PRNG from: the pinned [`Config::random_seed`]
    /// when the user set one, else a fresh draw from [`entropy_seed`] (SQ-0811).
    ///
    /// Called once per engine construction, which makes the two cases behave the
    /// way each is meant to: a pinned seed replays the same game after a restart,
    /// and an unpinned one deals a new one.
    pub fn effective_random_seed(&self) -> u32 {
        self.random_seed.unwrap_or_else(entropy_seed)
    }

    /// The interpreter number a one-run source pinned for this launch, if any.
    /// `boot_story` needs it because the CLI's value and the global config's value
    /// live in the same field and outrank the per-game sidecar differently.
    pub fn interpreter_number_one_run(&self) -> Option<u8> {
        self.one_run.int(keys::INTERPRETER_NUMBER).map(|n| n as u8)
    }

    /// Set `interpreter_number` as a deliberate user edit (the settings panel), which
    /// ends the one-run hold on the key — including the case where the user picks the
    /// very number `--interpreter` supplied. `None` means "default" (the
    /// per-version auto rule) and REMOVES the key on the next save.
    pub fn set_interpreter_number(&mut self, n: Option<u8>) {
        self.interpreter_number = n;
        self.one_run.release(keys::INTERPRETER_NUMBER);
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_SCHEMA_VERSION,
            user_dir: default_user_dir(),
            default_story_dir: None,
            auto_load: true,
            auto_save: false,
            mouse_wheel_invert: false,
            mouse: true,
            command_bar: false,
            prompt_save_on_quit: true,
            prompt_load_on_launch: true,
            record_turn_history: false,
            history_turns: default_history_turns(),
            hint_skip_screen_warning: true,
            guidance: true,
            guidance_probe: true,
            return_probe: true,
            hide_adult_words: true,
            adult_words: default_adult_words(),
            background_tidy: BackgroundTidy::EveryRoom,
            aux_storage: AuxStorage::Ask,
            v6_render: V6RenderMode::Hybrid,
            fuse_art_dither: true,
            glk_pixel_scale: GlkPixelScale::Native,
            v6_arrow_keys: false,
            v6_pixel_lock: false,
            system_font_disk: String::new(),
            keymap: KeymapConfig::default(),
            hotkeys: HotkeysConfig::default(),
            style: None,
            watch_style: false,
            font_check_pending: false,
            undo_levels: default_undo_levels(),
            command_prefix: default_command_prefix(),
            show_room_numbers: false,
            show_status_bar: true,
            search: SearchConfig::default(),
            virtual_screen_cols: None,
            virtual_screen_rows: None,
            split_ratio: default_split_ratio(),
            command_band: CommandBandConfig::default(),
            inv_dock_pct: default_inv_dock_pct(),
            room_dock_pct: default_room_dock_pct(),
            text_margin_x: 0,
            text_margin_y: 0,
            animation: AnimationConfig::default(),
            honor_game_colours: default_honor_game_colours(),
            period_look: default_period_look(),
            honor_timed_input: default_honor_timed_input(),
            config_file: default_config_file(),
            config_error: None,
            interpreter_number: None,
            random_seed: None,
            one_run: OneRunOverrides::default(),
            interpreter_profile: crate::interpreter::InterpreterProfile::default(),
            interpreter_source: crate::interpreter::ProfileSource::Fallback,
            system_colours: default_system_colours(),
            colour_source: ColourSource::default(),
            pictures_override: None,
            disk_entry: None,
            enable_sound: default_enable_sound(),
            volume: default_volume(),
            acceleration: default_acceleration(),
            image_protocol: default_image_protocol(),
            images: default_images(),
            trace: crate::trace::TraceSections::default(),
        }
    }
}

/// The config file path `resolve` reads from, most specific first: the `--config`
/// override, else `--user-dir`'s `config.toml`, else the default home's.
///
/// SQ-0574: `--user-dir` used to be ignored here, so it relocated every WRITE
/// (`write_config` and the template seed both took `cfg.user_dir`) while reads still
/// came from the default home — `lanthorn --user-dir /tmp/x` seeded and saved
/// `/tmp/x/config.toml` and then loaded `~/.lanthorn/config.toml`, silently
/// discarding everything it had just written.
///
/// Deliberately driven by the CLI alone: the `user_dir` KEY inside the file names the
/// data root (maps, saves, exports) and cannot also name the file it was read from
/// without being circular. `--user-dir` moves both; the key moves only the data.
pub fn config_path(cli: &Cli) -> std::path::PathBuf {
    match &cli.config {
        Some(p) => p.clone(),
        None => cli.user_dir.clone().unwrap_or_else(default_user_dir).join("config.toml"),
    }
}

/// True if a raw config.toml still contains a top-level `[colors]` or `[symbols]`
/// table. Those style sections moved to style.toml and are no longer read; the
/// caller warns once so users can migrate.
pub fn config_has_style_sections(raw: &str) -> bool {
    match raw.parse::<toml::Value>() {
        Ok(toml::Value::Table(t)) => t.contains_key("colors") || t.contains_key("symbols"),
        _ => false,
    }
}

// ── Load order ────────────────────────────────────────────────────────────────

/// Resolve configuration with precedence: defaults < config file < CLI flags.
///
/// A missing config file is silently ignored (not an error).
/// Returns the merged Config.  The Cli is returned by the caller via
/// `Cli::parse()` before calling this; pass a reference here.
pub fn resolve(cli: &Cli) -> Config {
    // Determine which config file to read.
    let config_path = config_path(cli);

    // Start from defaults.
    let mut cfg = Config { config_file: config_path.clone(), ..Config::default() };

    // Layer in the config file if it exists.
    if let Ok(text) = std::fs::read_to_string(&config_path) {
        let parsed = toml::from_str::<Config>(&text);
        // A file that exists but doesn't load used to be dropped in silence, so one
        // stray character reverted every setting to its default with nothing said —
        // and the next settings save then overwrote the user's file (SQ-0580). Keep
        // the error for startup to show; `write_config_at` refuses to clobber.
        //
        // This fires for a TYPE error (`volume = 300`, `auto_load = "yes"`) exactly as
        // it does for a syntax error: `from_str` fails either way, so either way the
        // whole file is lost to memory. That distinction used to matter, because the
        // write side re-parsed with toml_edit — which accepts a type error happily —
        // and then stamped in-memory defaults over every key the file already had
        // (SQ-0645). The write side now gates on THIS field instead.
        if let Err(e) = &parsed {
            cfg.config_error = Some(e.to_string());
        }
        if let Ok(from_file) = parsed {
            // NOTE: this is a field-by-field merge — every persisted field must
            // be copied here or the file's value is ignored on load. See the
            // checklist on `struct Config`. (Also mirror it in `write_config`.)
            // Carry the file's own version stamp (0 if the file predates
            // versioning) so a future check can flag an out-of-date config.
            cfg.version = from_file.version;
            cfg.user_dir = from_file.user_dir;
            cfg.default_story_dir = from_file.default_story_dir;
            cfg.auto_load = from_file.auto_load;
            cfg.auto_save = from_file.auto_save;
            cfg.mouse_wheel_invert = from_file.mouse_wheel_invert;
            cfg.mouse = from_file.mouse;
            cfg.command_bar = from_file.command_bar;
            cfg.prompt_save_on_quit = from_file.prompt_save_on_quit;
            cfg.prompt_load_on_launch = from_file.prompt_load_on_launch;
            cfg.record_turn_history = from_file.record_turn_history;
            cfg.history_turns = from_file.history_turns;
            cfg.hint_skip_screen_warning = from_file.hint_skip_screen_warning;
            cfg.guidance = from_file.guidance;
            cfg.guidance_probe = from_file.guidance_probe;
            cfg.return_probe = from_file.return_probe;
            cfg.hide_adult_words = from_file.hide_adult_words;
            cfg.adult_words = from_file.adult_words;
            cfg.background_tidy = from_file.background_tidy;
            cfg.aux_storage = from_file.aux_storage;
            cfg.v6_render = from_file.v6_render;
            cfg.fuse_art_dither = from_file.fuse_art_dither;
            cfg.glk_pixel_scale = from_file.glk_pixel_scale;
            cfg.v6_arrow_keys = from_file.v6_arrow_keys;
            cfg.v6_pixel_lock = from_file.v6_pixel_lock;
            cfg.system_font_disk = from_file.system_font_disk;
            cfg.keymap = from_file.keymap;
            cfg.hotkeys = from_file.hotkeys;
            cfg.style = from_file.style;
            cfg.watch_style = from_file.watch_style;
            cfg.font_check_pending = from_file.font_check_pending;
            cfg.undo_levels = from_file.undo_levels;
            cfg.command_prefix = from_file.command_prefix;
            cfg.show_room_numbers = from_file.show_room_numbers;
            cfg.show_status_bar = from_file.show_status_bar;
            cfg.honor_game_colours = from_file.honor_game_colours;
            cfg.period_look = from_file.period_look;
            cfg.system_colours = from_file.system_colours;
            cfg.honor_timed_input = from_file.honor_timed_input;
            cfg.interpreter_number = from_file.interpreter_number;
            cfg.random_seed = from_file.random_seed;
            cfg.enable_sound = from_file.enable_sound;
            cfg.volume = from_file.volume;
            cfg.search = from_file.search;
            cfg.virtual_screen_cols = from_file.virtual_screen_cols;
            cfg.virtual_screen_rows = from_file.virtual_screen_rows;
            cfg.split_ratio = from_file.split_ratio;
            cfg.command_band = from_file.command_band;
            cfg.inv_dock_pct = from_file.inv_dock_pct;
            cfg.room_dock_pct = from_file.room_dock_pct;
            cfg.text_margin_x = from_file.text_margin_x;
            cfg.text_margin_y = from_file.text_margin_y;
            cfg.animation = from_file.animation;
        }
        // A malformed file leaves every field at its default — TOML is parsed as one
        // document, so there is no half-loaded config to salvage.
    }

    // CLI overrides beat the file — and every one of them that lands on a key
    // `write_config_at` persists is PINNED as it lands, so a later settings save
    // cannot bake this run's instruction into the file (SQ-0807; see
    // `OneRunOverrides`). `--accel`, `--image-protocol`, `--images`,
    // `--trace` and `--pictures` need no pin: their fields are `#[serde(skip)]`
    // and never written at all.
    if let Some(dir) = &cli.user_dir {
        cfg.user_dir = dir.clone();
        // `--user-dir` relocates BOTH the file and the data root for one run,
        // which is not the same thing as the `user_dir` key (that moves the data
        // only). With `--config` naming a different file, writing it back would
        // pin this run's temporary root into the user's real config.
        cfg.one_run.pin(keys::USER_DIR, dir.to_string_lossy().into_owned());
    }

    // SQ-1082: every switch below is `Option<OnOff>`, and the `Option` is the
    // point. These were negative-only — `--no-sound`, `--no-images` — which made
    // them ONE-WAY: they could force a setting off for a run and nothing on the
    // command line could force it on, so a config carrying `enable_sound = false`
    // could only be overridden by editing the file. A bare `bool` cannot carry
    // the third answer that fixes it, because "not mentioned" then reads as
    // "off" and a flag's absence starts turning persisted `true` values off,
    // which is the same defect facing the other way.
    //
    // `acceleration` and `images` are `#[serde(skip)]`, so they need no one-run
    // pin — there is no key for a settings save to bake them into.
    if let Some(v) = cli.accel {
        cfg.acceleration = v.into();
    }
    cfg.image_protocol = cli.image_protocol;
    if let Some(v) = cli.images {
        cfg.images = v.into();
    }
    if let Some(v) = cli.sound {
        cfg.enable_sound = v.into();
        cfg.one_run.pin(keys::ENABLE_SOUND, bool::from(v));
    }

    // Pinned like the rest: `guidance` is a persisted key, so one `--guidance off`
    // launch plus any settings save would otherwise bake this run's instruction
    // into the user's file for good.
    if let Some(v) = cli.guidance {
        cfg.guidance = v.into();
        cfg.one_run.pin(keys::GUIDANCE, bool::from(v));
    }

    // `--game-colours` is set on the LIVE value every render site reads, so a
    // mid-game `/set-game-colours` still wins — the pin releases the moment the
    // value stops being ours, exactly as `--interpreter`'s does (SQ-0855).
    //
    // `boot_story` keeps the two per-game layers that come later — a `garglk.ini`
    // beside the story, and this game's sidecar — from overriding it, now in
    // BOTH directions: a flag is an instruction for the launch you typed it on
    // whichever way it points.
    if let Some(v) = cli.game_colours {
        cfg.honor_game_colours = v.into();
        cfg.one_run.pin(keys::HONOR_GAME_COLOURS, bool::from(v));
    }

    // SQ-1079: the two v6 render settings, said before the game boots. Both are
    // persisted keys, so both are pinned as they land — `--v6-render raster` for
    // one capture must not make raster the mode every story opens in after the
    // next settings save.
    if let Some(mode) = cli.v6_render {
        cfg.v6_render = mode;
        cfg.one_run.pin(keys::V6_RENDER, v6_render_key(mode));
    }
    if let Some(lock) = cli.v6_pixel_lock {
        cfg.v6_pixel_lock = lock.into();
        cfg.one_run.pin(keys::V6_PIXEL_LOCK, bool::from(lock));
    }

    // SQ-1082: which of the three default-colour sources answers for this launch.
    // `--colour machine` subsumes what `--system-colours` was (SQ-0928): the
    // opt-in that licenses a machine's own §8.3.3 pair when the MEDIUM did not
    // name the machine but you did, with `--interpreter`. The other two arms
    // narrow the chain instead, which nothing could ask for before.
    //
    // Pinned, unlike `--system-colours`, which set a persisted key and left no
    // record that this run had done it — so one `--system-colours` launch plus
    // any settings save (the browser's "remember this directory?" is enough)
    // wrote `system_colours = true` into the user's file for good.
    if let Some(src) = cli.colour {
        cfg.colour_source = src;
        if src == ColourSource::Machine {
            cfg.system_colours = true;
            cfg.one_run.pin(keys::SYSTEM_COLOURS, true);
        }
    }

    // `--interpreter N` beats the file's `interpreter_number`; absent, the
    // file's value (or the auto rule, when it too is unset) stands.
    if let Some(n) = cli.interpreter_number {
        cfg.interpreter_number = Some(n);
        cfg.one_run.pin(keys::INTERPRETER_NUMBER, n);
    }

    if let Some(list) = &cli.trace {
        let (sections, unknown) = crate::trace::TraceSections::parse(list);
        cfg.trace = sections;
        for u in unknown {
            eprintln!("warning: unknown --trace section '{u}' (valid: screen, map, hostio, v6, all, none)");
        }
    }

    cfg
}

// ── Write helpers ─────────────────────────────────────────────────────────────

/// The `config.toml` document being written, plus the one-run pins that must not
/// reach it (SQ-0807). Every top-level key goes through [`ConfigDoc::put`] or
/// [`ConfigDoc::put_or_remove`], which is the single place the one-run rule is
/// applied — there is no per-key guard anywhere below. Derefs to the document so
/// `doc["version"] = …`, `doc.contains_key(…)` and `doc.remove(…)` read as before.
struct ConfigDoc<'a> {
    doc: toml_edit::DocumentMut,
    one_run: &'a OneRunOverrides,
}

impl std::ops::Deref for ConfigDoc<'_> {
    type Target = toml_edit::DocumentMut;
    fn deref(&self) -> &Self::Target { &self.doc }
}
impl std::ops::DerefMut for ConfigDoc<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.doc }
}

impl ConfigDoc<'_> {
    /// Set `key` only when it is worth persisting (SQ-0573).
    ///
    /// A setting at its default belongs in the seeded commented template, not as a
    /// live key: `write_config` used to stamp all ~36 keys unconditionally, so the
    /// first settings save — the story browser's "remember this directory?" prompt is
    /// enough — appended the whole flat key list to a freshly seeded config.toml,
    /// pinning every default and burying the comments (they land BELOW the inserted
    /// keys, since an all-comment file parses as trailing trivia).
    ///
    /// So: add a key only when its value differs from the default, but ALWAYS update
    /// one the file already has. That second half matters twice over — it keeps a
    /// setting the user flipped back to its default from silently reverting on the
    /// next launch, and it means nothing is ever removed, so a comment the user wrote
    /// above their own key (which toml_edit attaches to that key as decor) is never
    /// dropped.
    ///
    /// …and neither half applies while a one-run source still owns the value: this
    /// run's `--sound off` is not a setting, so the file keeps whatever it said.
    fn put(&mut self, key: &str, value: toml_edit::Value, is_default: bool) {
        if self.one_run.still_holds(key, &value) {
            return;
        }
        if !is_default || self.doc.contains_key(key) {
            self.doc[key] = toml_edit::Item::Value(value);
        }
    }

    /// [`ConfigDoc::put`] for a key whose "default" is ABSENCE rather than a value:
    /// `Some` writes it whatever it is, `None` REMOVES it. Leaving a `None` in place
    /// meant a reset-to-default in the settings panel held for exactly as long as the
    /// session, since an absent key and a present one are different states here.
    fn put_or_remove(&mut self, key: &str, value: Option<toml_edit::Value>) {
        match value {
            Some(v) if self.one_run.still_holds(key, &v) => {}
            Some(v) => { self.doc[key] = toml_edit::Item::Value(v); }
            None => { self.doc.remove(key); }
        }
    }
}

/// [`ConfigDoc::put`] for a key inside a table. No one-run source pins a table key
/// (`[search]`, `[animation]`, `[command_panel]` have no CLI flag, sidecar key or
/// inferred value), so this stays the plain default-elision rule.
fn put_in(tbl: &mut toml_edit::Item, key: &str, value: toml_edit::Value, is_default: bool) {
    let present = tbl.get(key).is_some();
    if !is_default || present {
        tbl[key] = toml_edit::Item::Value(value);
    }
}

/// Save `cfg` back to the file it was loaded from ([`Config::config_file`]).
///
/// Production code should use this rather than [`write_config`]: writing to
/// `cfg.user_dir` meant a `--user-dir` (or a `user_dir` key) sent the save somewhere
/// `resolve` would never read it back from (SQ-0574).
pub fn write_config_file(cfg: &Config) -> std::io::Result<()> {
    write_config_at(&cfg.config_file, cfg)
}

/// Write the functional config fields (and the `style` pointer) to `dir/config.toml`
/// using toml_edit (format-preserving). Creates the file and parent directory if absent.
/// Does NOT emit `[colors]`/`[symbols]` — those now live in the style file.
/// Preserves all other content (comments, `[keymap]`, `[hotkeys]`, any visual sections, etc.).
///
/// NOTE: every persisted field needs a `doc.put("…", …)` line below — a field
/// that's missing here is never written, so a settings-panel edit silently
/// reverts on the next launch. See the checklist on `struct Config` (and mirror
/// any new field in `resolve`). Going through `put`/`put_or_remove` rather than
/// assigning `doc[key]` directly is also what applies the one-run rule
/// ([`OneRunOverrides`]), so a key written by hand quietly loses that guard.
pub fn write_config(dir: &std::path::Path, cfg: &Config) -> std::io::Result<()> {
    write_config_at(&dir.join("config.toml"), cfg)
}

/// [`write_config`] to an exact file path — the form [`write_config_file`] uses so a
/// save lands on the file `resolve` read (SQ-0574). `write_config(dir, …)` remains for
/// callers that genuinely mean "this directory's config.toml", chiefly tests.
pub fn write_config_at(config_path: &std::path::Path, cfg: &Config) -> std::io::Result<()> {
    // The file didn't LOAD, so `cfg` is all defaults with none of the user's values in
    // it — writing it back would replace their settings with ours. The syntax half of
    // that was closed by SQ-0580 below; this closes the type half (SQ-0645), which
    // walked straight past it: `volume = 300` or `auto_load = "yes"` is valid TOML, so
    // toml_edit parsed the doc happily and `put` then "updated" every key the file
    // already had to the in-memory default. Same event, same refusal, same message
    // path — startup already tells the user settings won't save until it's fixed.
    if let Some(err) = &cfg.config_error {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "{} could not be loaded ({err}) — refusing to overwrite it. Fix the file, \
                 or move it aside and lanthorn will seed a fresh one.",
                cfg.config_file.display(),
            ),
        ));
    }

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let existing = std::fs::read_to_string(config_path).unwrap_or_default();
    // A file that doesn't parse is NOT a blank slate to build on: `unwrap_or_default`
    // here meant the first settings save (the story browser's "remember this
    // directory?" prompt is enough) replaced the user's entire file — every key and
    // every comment — with a fresh doc, destroying the very text they need to see to
    // fix the typo. Refuse instead, and say why (SQ-0580). An absent or empty file
    // parses fine, so this only ever fires on real syntax errors.
    let parsed: toml_edit::DocumentMut = match existing.parse() {
        Ok(doc) => doc,
        Err(e) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "{} is not valid TOML ({e}) — refusing to overwrite it. Fix the file, \
                     or move it aside and lanthorn will seed a fresh one.",
                    config_path.display(),
                ),
            ));
        }
    };
    // Defaults to compare against, so a setting nobody changed is not written out —
    // and the one-run pins, so a value this launch was handed is not written out at
    // all (SQ-0807).
    let def = Config::default();
    let mut doc = ConfigDoc { doc: parsed, one_run: &cfg.one_run };

    // Top-level scalar fields. Always stamp the current schema version — writing
    // the file brings it up to the format this build produces.
    doc["version"] = toml_edit::value(CONFIG_SCHEMA_VERSION as i64);
    doc.put("user_dir", cfg.user_dir.to_string_lossy().as_ref().into(), cfg.user_dir == def.user_dir);
    doc.put_or_remove(
        "default_story_dir",
        cfg.default_story_dir.as_ref().map(|p| p.to_string_lossy().as_ref().into()),
    );
    doc.put("auto_load", cfg.auto_load.into(), cfg.auto_load == def.auto_load);
    doc.put("auto_save", cfg.auto_save.into(), cfg.auto_save == def.auto_save);
    doc.put("mouse_wheel_invert", cfg.mouse_wheel_invert.into(), cfg.mouse_wheel_invert == def.mouse_wheel_invert);
    doc.put("mouse", cfg.mouse.into(), cfg.mouse == def.mouse);
    doc.put("command_bar", cfg.command_bar.into(), cfg.command_bar == def.command_bar);
    doc.put("prompt_save_on_quit", cfg.prompt_save_on_quit.into(), cfg.prompt_save_on_quit == def.prompt_save_on_quit);
    doc.put("prompt_load_on_launch", cfg.prompt_load_on_launch.into(), cfg.prompt_load_on_launch == def.prompt_load_on_launch);
    let bg_str = match cfg.background_tidy {
        BackgroundTidy::Off => "off",
        BackgroundTidy::EveryRoom => "every_room",
        BackgroundTidy::OnOverlap => "on_overlap",
        BackgroundTidy::Debounced => "debounced",
    };
    doc.put("background_tidy", bg_str.into(), cfg.background_tidy == def.background_tidy);
    let aux_str = match cfg.aux_storage {
        AuxStorage::Ask => "ask",
        AuxStorage::Archive => "archive",
        AuxStorage::Global => "global",
    };
    doc.put("aux_storage", aux_str.into(), cfg.aux_storage == def.aux_storage);
    doc.put("v6_render", v6_render_key(cfg.v6_render).into(), cfg.v6_render == def.v6_render);
    doc.put("fuse_art_dither", cfg.fuse_art_dither.into(), cfg.fuse_art_dither == def.fuse_art_dither);
    let scale_val: toml_edit::Value = match cfg.glk_pixel_scale {
        GlkPixelScale::Native => "native".into(),
        GlkPixelScale::Auto => "auto".into(),
        GlkPixelScale::Fixed(n) => (n as i64).into(),
    };
    doc.put("glk_pixel_scale", scale_val, cfg.glk_pixel_scale == def.glk_pixel_scale);
    doc.put("v6_arrow_keys", cfg.v6_arrow_keys.into(), cfg.v6_arrow_keys == def.v6_arrow_keys);
    doc.put("v6_pixel_lock", cfg.v6_pixel_lock.into(), cfg.v6_pixel_lock == def.v6_pixel_lock);
    doc.put(
        "system_font_disk",
        cfg.system_font_disk.as_str().into(),
        cfg.system_font_disk == def.system_font_disk,
    );
    doc.put("show_room_numbers", cfg.show_room_numbers.into(), cfg.show_room_numbers == def.show_room_numbers);
    doc.put("show_status_bar", cfg.show_status_bar.into(), cfg.show_status_bar == def.show_status_bar);
    doc.put("hint_skip_screen_warning", cfg.hint_skip_screen_warning.into(), cfg.hint_skip_screen_warning == def.hint_skip_screen_warning);
    doc.put("guidance", cfg.guidance.into(), cfg.guidance == def.guidance);
    doc.put("guidance_probe", cfg.guidance_probe.into(), cfg.guidance_probe == def.guidance_probe);
    doc.put("return_probe", cfg.return_probe.into(), cfg.return_probe == def.return_probe);
    doc.put("hide_adult_words", cfg.hide_adult_words.into(), cfg.hide_adult_words == def.hide_adult_words);
    // The LIST is the one setting lanthorn seeds LIVE rather than commented
    // (SQ-1122), so `put`'s "always update a key the file already has" half keeps
    // it in step — and its default-elision half means a player who deleted the
    // line outright never gets it back uninvited.
    let words = cfg.adult_words.iter().fold(toml_edit::Array::new(), |mut a, w| {
        a.push(w.as_str());
        a
    });
    doc.put("adult_words", words.into(), cfg.adult_words == def.adult_words);
    doc.put("watch_style", cfg.watch_style.into(), cfg.watch_style == def.watch_style);
    // Written only while it is true, like every other key `put` skips at its
    // default — so answering the check does not leave `font_check_pending = false`
    // behind as a permanent line in a file the player reads (SQ-1112).
    doc.put(
        "font_check_pending",
        cfg.font_check_pending.into(),
        cfg.font_check_pending == def.font_check_pending,
    );
    doc.put("record_turn_history", cfg.record_turn_history.into(), cfg.record_turn_history == def.record_turn_history);
    doc.put("history_turns", (cfg.history_turns as i64).into(), cfg.history_turns == def.history_turns);
    // Three one-run sources reach this key and `put` skips all three the same way:
    // a discovered garglk.ini, this game's own sidecar, and two-colour ARTWORK,
    // which has no colours to give and so declares the interpreter colourless for
    // one story (SQ-0806). Opening Zork Zero's CGA rendition once must not teach
    // the global config to never honour game colours again.
    doc.put("honor_game_colours", cfg.honor_game_colours.into(), cfg.honor_game_colours == def.honor_game_colours);
    doc.put("period_look", cfg.period_look.into(), cfg.period_look == def.period_look);
    doc.put("system_colours", cfg.system_colours.into(), cfg.system_colours == def.system_colours);
    doc.put("honor_timed_input", cfg.honor_timed_input.into(), cfg.honor_timed_input == def.honor_timed_input);
    doc.put("enable_sound", cfg.enable_sound.into(), cfg.enable_sound == def.enable_sound);
    doc.put("volume", (cfg.volume as i64).into(), cfg.volume == def.volume);
    doc.put("undo_levels", (cfg.undo_levels as i64).into(), cfg.undo_levels == def.undo_levels);
    // `--interpreter`, a launch-options choice and this game's sidecar all
    // pin this key for one run, and `put_or_remove` skips all three — but ONLY while
    // the value is still theirs. Once the settings panel changes it, it is the user's
    // choice and persists like anything else (SQ-0646); the old "from CLI" flag made
    // a `--interpreter` session ignore panel edits forever, reporting success
    // and saving nothing.
    doc.put_or_remove(
        "interpreter_number",
        cfg.interpreter_number.map(|n| (n as i64).into()),
    );
    // Written only when the user pinned one. An absent key is "a fresh seed every
    // launch", and writing back the seed THIS session happened to draw would turn
    // one entropy draw into a permanent pin — every later launch replaying the one
    // game the user happened to get today (SQ-0811).
    doc.put_or_remove("random_seed", cfg.random_seed.map(|n| i64::from(n).into()));
    // Written only when the user pinned one: an absent key means "follow the
    // story pane" (ZMSD §8.4), and emitting the measured size would silently turn
    // this session's terminal into a permanent override. (SQ-0532/A-F1)
    doc.put_or_remove("virtual_screen_cols", cfg.virtual_screen_cols.map(|n| i64::from(n).into()));
    doc.put_or_remove("virtual_screen_rows", cfg.virtual_screen_rows.map(|n| i64::from(n).into()));
    doc.put("split_ratio", i64::from(cfg.split_ratio).into(), cfg.split_ratio == def.split_ratio);
    doc.put("inv_dock_pct", i64::from(cfg.inv_dock_pct).into(), cfg.inv_dock_pct == def.inv_dock_pct);
    doc.put("room_dock_pct", i64::from(cfg.room_dock_pct).into(), cfg.room_dock_pct == def.room_dock_pct);
    doc.put("text_margin_x", i64::from(cfg.text_margin_x).into(), cfg.text_margin_x == def.text_margin_x);
    doc.put("text_margin_y", i64::from(cfg.text_margin_y).into(), cfg.text_margin_y == def.text_margin_y);

    // style pointer — the only visual key written to config.toml. The actual
    // colors/symbols live in the style file ([colors]/[symbols] are no longer
    // emitted here). Visual override sections, if present, are preserved as-is.
    doc.put_or_remove("style", cfg.style.as_deref().map(|s| s.into()));

    // [search] table — only materialized once something in it is non-default, so a
    // seeded config keeps the commented block instead of gaining an all-defaults table.
    if doc.contains_key("search")
        || cfg.search.start_backward != def.search.start_backward
        || cfg.search.key_back != def.search.key_back
        || cfg.search.key_forward != def.search.key_forward
    {
        let tbl = doc["search"].or_insert(toml_edit::table());
        put_in(tbl, "start_backward", cfg.search.start_backward.into(), cfg.search.start_backward == def.search.start_backward);
        put_in(tbl, "key_back", cfg.search.key_back.to_string().into(), cfg.search.key_back == def.search.key_back);
        put_in(tbl, "key_forward", cfg.search.key_forward.to_string().into(), cfg.search.key_forward == def.search.key_forward);
    }

    // [animation] table — same rule as [search].
    if doc.contains_key("animation")
        || cfg.animation.enabled != def.animation.enabled
        || cfg.animation.easing != def.animation.easing
        || cfg.animation.scroll_ms != def.animation.scroll_ms
        || cfg.animation.scrollbar_hide_ms != def.animation.scrollbar_hide_ms
        || cfg.animation.scrollbar_fade_ms != def.animation.scrollbar_fade_ms
    {
        let tbl = doc["animation"].or_insert(toml_edit::table());
        put_in(tbl, "enabled", cfg.animation.enabled.into(), cfg.animation.enabled == def.animation.enabled);
        put_in(tbl, "easing", crate::anim::easing_token(cfg.animation.easing).into(), cfg.animation.easing == def.animation.easing);
        put_in(tbl, "scroll_ms", (cfg.animation.scroll_ms as i64).into(), cfg.animation.scroll_ms == def.animation.scroll_ms);
        put_in(tbl, "scrollbar_hide_ms", (cfg.animation.scrollbar_hide_ms as i64).into(), cfg.animation.scrollbar_hide_ms == def.animation.scrollbar_hide_ms);
        put_in(tbl, "scrollbar_fade_ms", (cfg.animation.scrollbar_fade_ms as i64).into(), cfg.animation.scrollbar_fade_ms == def.animation.scrollbar_fade_ms);
    }

    // [command_panel] table — same rule as [search]. The verb/quick LISTS are
    // hand-authored grammar, never written back by the app: resize mode edits
    // `height` and nothing else touches this section, so re-emitting a list here
    // could only ever damage what the user wrote.
    if doc.contains_key("command_panel")
        || cfg.command_band.height != def.command_band.height
        || cfg.command_band.auto_open != def.command_band.auto_open
    {
        let tbl = doc["command_panel"].or_insert(toml_edit::table());
        put_in(tbl, "height", i64::from(cfg.command_band.height).into(), cfg.command_band.height == def.command_band.height);
        put_in(tbl, "auto_open", cfg.command_band.auto_open.into(), cfg.command_band.auto_open == def.command_band.auto_open);
    }

    // Atomic (SQ-0644): `fs::write` truncated config.toml before writing a byte, so a
    // crash (or a full disk) mid-save left the user with an empty or half-written
    // config — every setting AND every comment gone, which is precisely what SQ-0580
    // went to such lengths to protect.
    crate::storage::atomic_write(config_path, doc.to_string().as_bytes())
}

// ── Tests ─────────────────────────────────────────────────────────────────────
    /// SQ-0885: `--interpreter-version` takes a number or one ASCII character.
    ///
    /// Both because the corpus renders the byte both ways — Shogun r295 prints
    /// it as a decimal, Nord and Bert r19 as a letter — so a person can type
    /// what they SAW. The digit rule is the one that could surprise: `8` is
    /// eight, never 56, because nobody reproducing a banner wants the ASCII code
    /// of a digit.
    #[test]
    fn interpreter_version_accepts_a_number_or_a_character() {
        assert_eq!(parse_interpreter_version("8"), Ok(8), "a digit is a NUMBER");
        assert_eq!(parse_interpreter_version("65"), Ok(65));
        assert_eq!(parse_interpreter_version("0"), Ok(0));
        assert_eq!(parse_interpreter_version("255"), Ok(255));
        assert_eq!(parse_interpreter_version("A"), Ok(b'A'), "a letter is its code");
        assert_eq!(parse_interpreter_version("C"), Ok(b'C'), "Nord and Bert's");
        assert!(parse_interpreter_version("256").is_err(), "past a byte");
        assert!(parse_interpreter_version("AB").is_err(), "two characters is neither");
        assert!(parse_interpreter_version("").is_err());
        assert!(parse_interpreter_version("\u{e9}").is_err(), "non-ASCII has no byte");
    }


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_levels_defaults_to_16() {
        assert_eq!(Config::default().undo_levels, 16);
    }

    // ── random_seed (SQ-0811) ─────────────────────────────────────────────────

    /// THE defect: with no `random_seed` set, two fresh boots must not be handed
    /// the same seed. Both VM cores construct with a fixed constant, so before
    /// this key existed every launch of a game that never seeds itself replayed
    /// one identical sequence — for a roguelike, the same dungeon forever.
    ///
    /// Twenty draws rather than two: a single unlucky collision between two
    /// 32-bit values would be a flaky failure, and this asserts the whole run is
    /// not one repeated value, which is what the old behaviour looked like.
    #[test]
    fn an_unpinned_seed_is_different_on_every_boot() {
        let cfg = Config::default();
        assert_eq!(cfg.random_seed, None, "unset is the default: a new game every launch");
        let draws: std::collections::HashSet<u32> =
            (0..20).map(|_| cfg.effective_random_seed()).collect();
        assert!(draws.len() > 1, "every boot drew the same seed {draws:?}");
        assert!(!draws.contains(&0), "0 absorbs xorshift32: random() would freeze");
    }

    /// The other direction: a pinned key makes the run reproducible, which is the
    /// whole point of exposing it. Without this, "pinned" could be drawing fresh
    /// entropy and the test above would still pass.
    #[test]
    fn a_pinned_seed_is_handed_to_every_boot_unchanged() {
        let cfg = Config { random_seed: Some(0xC0FF_EE00), ..Config::default() };
        assert_eq!(cfg.effective_random_seed(), 0xC0FF_EE00);
        assert_eq!(cfg.effective_random_seed(), 0xC0FF_EE00);
    }

    #[test]
    fn random_seed_loads_from_the_file_and_round_trips_through_a_save() {
        let parsed: Config = toml::from_str("random_seed = 20250811\n").unwrap();
        assert_eq!(parsed.random_seed, Some(20250811));

        let dir = std::env::temp_dir().join(format!("bm-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");

        // Unset writes no key at all. Emitting the seed this session happened to
        // draw would silently pin one entropy draw for every later launch.
        let mut cfg = Config { config_file: cfg_path.clone(), ..Config::default() };
        write_config_at(&cfg_path, &cfg).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(!back.contains("random_seed"), "an unset seed must stay unset: {back}");
        assert_eq!(toml::from_str::<Config>(&back).unwrap().random_seed, None);

        // Pinned survives the save, and `resolve` carries it off disk — the merge
        // step that fails silently when a new field is missed.
        cfg.random_seed = Some(20250811);
        write_config_at(&cfg_path, &cfg).unwrap();
        let reloaded = resolve(&cli_with_config(&cfg_path, None));
        assert_eq!(reloaded.random_seed, Some(20250811));
        assert_eq!(reloaded.effective_random_seed(), 20250811);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn record_turn_history_defaults_false_and_round_trips() {
        assert!(!Config::default().record_turn_history);
        let cfg: Config = toml::from_str("record_turn_history = true\n").unwrap();
        assert!(cfg.record_turn_history);
    }

    /// SQ-1185: the history cap defaults generously (deep enough that the
    /// feature still reaches "further back than the game's own UNDO"), an
    /// absent key reads as that default, and a set value round-trips.
    #[test]
    fn history_turns_defaults_to_500_and_round_trips() {
        assert_eq!(Config::default().history_turns, 500);
        let absent: Config = toml::from_str("").unwrap();
        assert_eq!(absent.history_turns, 500, "an absent key is the default, not 0");
        let cfg: Config = toml::from_str("history_turns = 50\n").unwrap();
        assert_eq!(cfg.history_turns, 50);
    }

    #[test]
    fn watch_style_defaults_false_and_detector_works() {
        let c = Config::default();
        assert!(!c.watch_style);
        assert!(config_has_style_sections("[colors]\n\"room\" = { fg = \"red\" }\n"));
        assert!(config_has_style_sections("[symbols]\nbox_style = \"thick\"\n"));
        assert!(!config_has_style_sections("style = \"s.toml\"\n"));
    }

    #[test]
    fn virtual_screen_defaults_to_following_the_pane() {
        let cfg = Config::default();
        // ZMSD §8.4 wants the REAL screen size in $20/$21, so an unset key means
        // "measure the story pane", not a fixed 80x24 guess. (SQ-0532/A-F1)
        assert_eq!(cfg.virtual_screen_cols, None);
        assert_eq!(cfg.virtual_screen_rows, None);
    }

    #[test]
    fn virtual_screen_parses_from_toml() {
        let cfg: Config = toml::from_str("virtual_screen_cols = 64\nvirtual_screen_rows = 20").unwrap();
        assert_eq!(cfg.virtual_screen_cols, Some(64));
        assert_eq!(cfg.virtual_screen_rows, Some(20));
    }

    #[test]
    fn pane_size_pcts_default_and_parse() {
        let d = Config::default();
        assert_eq!(d.split_ratio, 50);
        assert_eq!(d.inv_dock_pct, 33);

        let cfg: Config = toml::from_str("split_ratio = 70\ninv_dock_pct = 25\n").unwrap();
        assert_eq!(cfg.split_ratio, 70);
        assert_eq!(cfg.inv_dock_pct, 25);
    }

    /// SQ-0664: `verb_dock_pct` sized the left verb dock, which no longer
    /// exists. Pre-release, retired keys are dropped outright (no back-compat
    /// shims) — a file still carrying it must load, with the key ignored.
    #[test]
    fn retired_verb_dock_pct_is_no_longer_a_key() {
        let cfg: Config = toml::from_str("verb_dock_pct = 40\nsplit_ratio = 70\n")
            .expect("a stale key does not break the file");
        assert_eq!(cfg.split_ratio, 70);
        assert_eq!(
            cfg.command_band.height,
            crate::render::command_band::DEFAULT_BAND_ROWS,
            "the band's height is the successor knob"
        );
    }

    // ── [command_panel] ────────────────────────────────────────────────────────

    #[test]
    fn command_band_defaults_and_round_trips() {
        let d = Config::default();
        assert_eq!(d.command_band.height, crate::render::command_band::DEFAULT_BAND_ROWS);
        assert!(!d.command_band.auto_open);
        assert!(d.command_band.verbs.is_empty());
        assert!(d.command_band.quick.is_empty());

        let cfg: Config = toml::from_str(
            "[command_panel]\nheight = 10\nauto_open = true\nquick = [\"n\", \"s\"]\n",
        )
        .unwrap();
        assert_eq!(cfg.command_band.height, 10);
        assert!(cfg.command_band.auto_open);
        assert_eq!(cfg.command_band.resolve_quick(), vec!["n".to_string(), "s".to_string()]);
    }

    /// `verbs` REPLACES the whole column; `extra_verbs` is additive. Both keep
    /// their pre-SQ-1111 behaviour to the letter — that is the compatibility
    /// promise, and the grammar-fed case is pinned alongside it below.
    #[test]
    fn command_band_verbs_replace_and_extra_verbs_add() {
        use crate::render::command_band::{VerbLine, VerbSource};

        // Replace: only what the file lists survives.
        let cfg: Config = toml::from_str(
            "[command_panel]\nverbs = [{ word = \"polish\", arity = \"object\" }]\n",
        )
        .unwrap();
        let (table, warn) = cfg.command_band.resolve_verbs();
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(table.entries.len(), 1);
        assert_eq!(table.entries[0].word, "polish");
        assert_eq!(table.source, VerbSource::Configured, "the player's own list says so");
        assert!(!table.entries.iter().any(|v| v.word == "look"), "the built-ins are replaced");

        // …and it replaces the STORY's grammar too, not just the built-ins.
        let story = vec![crate::render::command_band::VerbEntry::new(
            "gaze",
            vec![VerbLine::bare()],
        )];
        let (table, _) = cfg.command_band.resolve_verbs_with(Some(story.clone()));
        assert_eq!(table.entries.len(), 1);
        assert_eq!(table.entries[0].word, "polish");
        assert_eq!(table.source, VerbSource::Configured);

        // Additive: the built-ins stay and the extra joins them.
        let cfg: Config = toml::from_str(
            "[command_panel]\nextra_verbs = [{ word = \"xyzzy\", arity = \"solo\" }]\n",
        )
        .unwrap();
        let (table, warn) = cfg.command_band.resolve_verbs();
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(table.source, VerbSource::Builtin, "no story asked, no story answered");
        assert!(table.entries.iter().any(|v| v.word == "look"), "the built-ins survive");
        let x = table.entries.iter().find(|v| v.word == "xyzzy").expect("extra verb added");
        assert_eq!(x.lines, vec![VerbLine::bare()]);

        // …and the same key extends the STORY's list when there is one, which
        // is the whole change: `extra_verbs` now patches a real grammar.
        let (table, _) = cfg.command_band.resolve_verbs_with(Some(story));
        assert_eq!(table.source, VerbSource::Story, "still the story's, with one added");
        assert!(table.entries.iter().any(|v| v.word == "gaze"));
        assert!(table.entries.iter().any(|v| v.word == "xyzzy"));

        // Additive over a word that already exists RE-SHAPES it.
        let cfg: Config = toml::from_str(
            "[command_panel]\nextra_verbs = [{ word = \"take\", arity = \"pair\", prep = \"from\" }]\n",
        )
        .unwrap();
        let (table, _) = cfg.command_band.resolve_verbs();
        let take: Vec<_> = table.entries.iter().filter(|v| v.word == "take").collect();
        assert_eq!(take.len(), 1, "no duplicate entry");
        assert_eq!(take[0].lines, vec![VerbLine::pair("from")]);
        assert_eq!(take[0].joiner(), Some("from"));
    }

    /// SQ-1126: the sigil filter runs on BOTH assembly wrappers and takes no
    /// switch — including with the adult list off, which is the trap, because
    /// `hiding` short-circuits on an empty list and would carry a sigil word
    /// straight through if the two rules shared one pass.
    ///
    /// Falsifies against putting `#`/`$` in `adult_words` instead: turning the
    /// adult switch off would then put Infocom's test rig back on screen.
    #[test]
    fn sigil_verbs_are_filtered_by_both_wrappers_whatever_the_adult_switch_says() {
        use crate::render::command_band::{VerbEntry, VerbLine, VerbSource, VerbTable};

        let story = || {
            VerbTable::new(
                vec![
                    VerbEntry::new("$verify", vec![VerbLine::bare()]),
                    VerbEntry::new("#record", vec![VerbLine::bare()]),
                    VerbEntry::new("gaze", vec![VerbLine::bare()]),
                ],
                VerbSource::Story,
            )
        };
        for cfg in [Config::default(), Config { hide_adult_words: false, ..Config::default() }] {
            let words: Vec<String> =
                cfg.layer_band_verbs(story()).entries.into_iter().map(|e| e.word).collect();
            assert_eq!(words, vec!["gaze".to_string()], "hide_adult_words = {}", cfg.hide_adult_words);
        }

        // The born-on-the-fallback wrapper too, through a `verbs` list that
        // names one: a player cannot re-add the test rig by config either.
        let cfg: Config = toml::from_str(
            "hide_adult_words = false\n\
             [command_panel]\nverbs = [{ word = \"$verify\", arity = \"solo\" }, \
             { word = \"polish\", arity = \"object\" }]\n",
        )
        .unwrap();
        let (table, _) = cfg.resolve_band_verbs();
        let words: Vec<&str> = table.entries.iter().map(|e| e.word.as_str()).collect();
        assert_eq!(words, vec!["polish"]);
    }

    /// `object_opt` was the one arity the old enum needed a variant for and a
    /// real grammar simply writes as two lines. Pinned because it is the case
    /// that proves the config spelling survived the model change intact.
    #[test]
    fn object_opt_lowers_to_the_two_lines_it_always_meant() {
        use crate::render::command_band::VerbLine;
        let cfg: Config = toml::from_str(
            "[command_panel]\nverbs = [{ word = \"search\", arity = \"object?\" }]\n",
        )
        .unwrap();
        let (table, warn) = cfg.command_band.resolve_verbs();
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(table.entries[0].lines, vec![VerbLine::bare(), VerbLine::object()]);
        assert!(table.entries[0].accepts(0) && table.entries[0].accepts(1));
    }

    /// A bad `arity` is reported and skipped — never silently reinterpreted as
    /// some other grammar, which would send the wrong command shape to the game.
    #[test]
    fn command_band_bad_arity_warns_and_skips() {
        let cfg: Config = toml::from_str(
            "[command_panel]\nextra_verbs = [{ word = \"frob\", arity = \"triple\" }]\n",
        )
        .unwrap();
        let (table, warn) = cfg.command_band.resolve_verbs();
        assert!(!table.entries.iter().any(|v| v.word == "frob"), "the bad entry is skipped");
        assert_eq!(warn.len(), 1);
        assert!(warn[0].contains("frob") && warn[0].contains("triple"), "{warn:?}");
    }

    /// The height knob resize mode writes must survive a save/reload cycle.
    #[test]
    fn command_band_height_persists_through_write_config() {
        let dir = std::env::temp_dir().join(format!("bm-band-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut cfg = Config::default();
        cfg.command_band.height = 11;
        write_config(&dir, &cfg).unwrap();

        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.command_band.height, 11);

        // A hand-authored verb list is NOT rewritten by a settings save.
        std::fs::write(
            dir.join("config.toml"),
            "[command_panel]\nheight = 11\nverbs = [{ word = \"polish\", arity = \"object\" }]\n",
        )
        .unwrap();
        let mut cfg2 = Config::default();
        cfg2.command_band.height = 9;
        write_config(&dir, &cfg2).unwrap();
        let after = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        assert!(after.contains("polish"), "the user's grammar survives a save: {after}");
        let back2: Config = toml::from_str(&after).unwrap();
        assert_eq!(back2.command_band.height, 9);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_show_room_numbers_default_false_and_round_trips() {
        assert!(!Config::default().show_room_numbers);
        let cfg: Config = toml::from_str("show_room_numbers = true\n").unwrap();
        assert!(cfg.show_room_numbers);
    }


    #[test]
    fn config_show_status_bar_default_true_and_round_trips() {
        assert!(Config::default().show_status_bar);
        let cfg: Config = toml::from_str("show_status_bar = false\n").unwrap();
        assert!(!cfg.show_status_bar);
    }

    #[test]
    fn config_reads_command_prefix() {
        let cfg: Config = toml::from_str("command_prefix = \";\"\n").unwrap();
        assert_eq!(cfg.command_prefix, ';');
        assert_eq!(Config::default().command_prefix, '/');
    }
    use std::io::Write;

    /// A `Cli` that reads (and therefore saves to) `path`, optionally carrying an
    /// `--interpreter`. Everything else is off/default.
    fn cli_with_config(path: &std::path::Path, interpreter_number: Option<u8>) -> Cli {
        Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(path.to_path_buf()),
            accel: None,
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: None,
            game_colours: None,
            colour: None,
            interpreter_number,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        }
    }

    /// Write a temp config file and return its path.  The directory is
    /// [`crate::scratch_dir`]'s, which is unique per CALL — the name alone kept two
    /// tests apart only so long as nobody spelled one twice, and the pid alone keeps
    /// two concurrent PROCESSES apart but not two threads of one (SQ-0812, SQ-1163).
    fn write_temp_config(name: &str, contents: &str) -> PathBuf {
        let path = crate::scratch_dir(&format!("cfg-{name}")).join("config.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "{}", contents).unwrap();
        path
    }

    #[test]
    fn default_config_has_lanthorn_dir() {
        let cfg = Config::default();
        // The default user_dir must end with ".lanthorn".
        assert_eq!(cfg.user_dir.file_name().unwrap(), ".lanthorn");
    }

    #[test]
    fn parse_toml_populates_user_dir() {
        let toml = r#"user_dir = "/tmp/mydata""#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.user_dir, PathBuf::from("/tmp/mydata"));
    }

    #[test]
    fn unspecified_fields_fall_back_to_defaults() {
        // An empty TOML file should give us the same user_dir as Config::default().
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.user_dir.file_name().unwrap(), ".lanthorn");
    }

    #[test]
    fn default_story_dir_defaults_none_parses_and_round_trips() {
        // Absent by default.
        assert!(Config::default().default_story_dir.is_none());
        let empty: Config = toml::from_str("").unwrap();
        assert!(empty.default_story_dir.is_none());
        // Parsed from the file when present.
        let cfg: Config = toml::from_str(r#"default_story_dir = "/tmp/stories""#).unwrap();
        assert_eq!(cfg.default_story_dir, Some(PathBuf::from("/tmp/stories")));
        // write_config persists it, and a Some value survives the round trip.
        let dir = std::env::temp_dir().join(format!("lanthorn-dsd-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_config(&dir, &cfg).unwrap();
        let written = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let back: Config = toml::from_str(&written).unwrap();
        assert_eq!(back.default_story_dir, Some(PathBuf::from("/tmp/stories")));
        // None removes the key rather than writing an empty value.
        let mut none_cfg = cfg.clone();
        none_cfg.default_story_dir = None;
        write_config(&dir, &none_cfg).unwrap();
        let doc: toml_edit::DocumentMut =
            std::fs::read_to_string(dir.join("config.toml")).unwrap().parse().unwrap();
        assert!(doc.get("default_story_dir").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_override_beats_file() {
        let cfg_path = write_temp_config("cli_override", r#"user_dir = "/tmp/from-file""#);

        let cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: Some(PathBuf::from("/tmp/from-cli")),
            data_dir: None,
            config: Some(cfg_path.clone()),
            accel: None,
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: None,
            game_colours: None,
            colour: None,
            interpreter_number: None,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        };

        let cfg = resolve(&cli);
        assert_eq!(cfg.user_dir, PathBuf::from("/tmp/from-cli"));
        let _ = std::fs::remove_file(&cfg_path);
    }

    #[test]
    fn missing_config_file_resolves_to_defaults() {
        let cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(PathBuf::from("/nonexistent/path/config.toml")),
            accel: None,
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: None,
            game_colours: None,
            colour: None,
            interpreter_number: None,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        };
        let cfg = resolve(&cli);
        assert_eq!(cfg.user_dir.file_name().unwrap(), ".lanthorn");
    }

    #[test]
    fn file_value_beats_default_when_no_cli_override() {
        let cfg_path = write_temp_config("file_beats_default", r#"user_dir = "/tmp/from-file""#);

        let cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(cfg_path.clone()),
            accel: None,
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: None,
            game_colours: None,
            colour: None,
            interpreter_number: None,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        };
        let cfg = resolve(&cli);
        assert_eq!(cfg.user_dir, PathBuf::from("/tmp/from-file"));
        let _ = std::fs::remove_file(&cfg_path);
    }

    #[test]
    fn stale_use_default_map_key_is_ignored() {
        let cfg: crate::config::Config = toml::from_str("use_default_map = true").unwrap();
        let _ = cfg; // unknown key ignored, no panic
    }

    #[test]
    fn keymap_config_parses_context_sections() {
        let toml = r#"
[keymap]
use_defaults = false
[keymap.global]
"ctrl+s" = "save-state"
[keymap.map]
"left" = "pan-map -1 0"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert!(!cfg.keymap.use_defaults);
        assert_eq!(cfg.keymap.global.get("ctrl+s").map(String::as_str), Some("save-state"));
        assert_eq!(cfg.keymap.map.get("left").map(String::as_str), Some("pan-map -1 0"));
        // Default keeps use_defaults true.
        assert!(Config::default().keymap.use_defaults);
    }

    #[test]
    fn auto_load_defaults_true() {
        let cfg = Config::default();
        assert!(cfg.auto_load, "auto_load must default to true");
    }

    #[test]
    fn auto_save_defaults_false() {
        let cfg = Config::default();
        assert!(!cfg.auto_save, "auto_save must default to false");
    }

    #[test]
    fn background_tidy_defaults_every_room() {
        let cfg = Config::default();
        assert_eq!(cfg.background_tidy, BackgroundTidy::EveryRoom);
    }

    #[test]
    fn auto_load_parses_false_from_toml() {
        let cfg: Config = toml::from_str("auto_load = false").unwrap();
        assert!(!cfg.auto_load);
    }

    #[test]
    fn auto_save_parses_true_from_toml() {
        let cfg: Config = toml::from_str("auto_save = true").unwrap();
        assert!(cfg.auto_save);
    }

    #[test]
    fn background_tidy_parses_on_overlap_from_toml() {
        let cfg: Config = toml::from_str("background_tidy = \"on_overlap\"").unwrap();
        assert_eq!(cfg.background_tidy, BackgroundTidy::OnOverlap);
    }

    #[test]
    fn background_tidy_parses_off_from_toml() {
        let cfg: Config = toml::from_str("background_tidy = \"off\"").unwrap();
        assert_eq!(cfg.background_tidy, BackgroundTidy::Off);
    }

    #[test]
    fn background_tidy_parses_debounced_from_toml() {
        let cfg: Config = toml::from_str("background_tidy = \"debounced\"").unwrap();
        assert_eq!(cfg.background_tidy, BackgroundTidy::Debounced);
    }

    #[test]
    fn aux_storage_defaults_to_ask() {
        assert_eq!(Config::default().aux_storage, AuxStorage::Ask);
    }

    #[test]
    fn aux_storage_parses_variants_from_toml() {
        let c: Config = toml::from_str("aux_storage = \"archive\"").unwrap();
        assert_eq!(c.aux_storage, AuxStorage::Archive);
        let c: Config = toml::from_str("aux_storage = \"global\"").unwrap();
        assert_eq!(c.aux_storage, AuxStorage::Global);
    }

    /// What advent.blb's toolbar needs (SQ-0593). `Auto` reports a cell of exactly
    /// the reference height at every font size, so the game's fixed 36px request always
    /// buys the same 3 rows and its artwork scales WITH the text.
    #[test]
    fn glk_pixel_scale_auto_normalises_every_cell_to_the_reference() {
        use GlkPixelScale::*;
        // A conventional 1x cell is reported unchanged — auto is a no-op there.
        assert_eq!(Auto.apply((7, 14)), (7, 14));
        // A 2x-scaled cell reports the same pixel space as the 1x one.
        assert_eq!(Auto.apply((14, 28)), (7, 14), "a 2x display sees what 1x sees");
        // ...and so does every other scale, including fractional ones. This is the
        // whole point of dropping the integer divisor: `round(cell/14)` sent 21px
        // (a 1.5x-scaled 14px cell, the Windows/GNOME default) to a divisor of 2,
        // which doubled the toolbar against its 20px neighbour.
        assert_eq!(Auto.apply((10, 21)), (7, 14), "1.5x — the cliff the divisor rule had");
        assert_eq!(Auto.apply((11, 20)), (8, 14), "and its neighbour lands beside it");
        assert_eq!(Auto.apply((21, 42)), (7, 14), "3x");
        // A SMALL cell is normalised up rather than left alone, which is the change
        // in behaviour: artwork stays proportional to the text instead of holding a
        // fixed pixel size on a tiny font.
        assert_eq!(Auto.apply((4, 8)), (7, 14));
        // Aspect ratio is preserved, not assumed 1:2.
        assert_eq!(Auto.apply((10, 20)), (7, 14));
        assert_eq!(Auto.apply((20, 20)), (14, 14), "a square cell stays square");
        assert_eq!(Auto.apply((0, 0)), (1, 14), "nonsense input still yields usable pixels");

        // Fixed is a plain divisor, and 1 restores the terminal's own cell.
        assert_eq!(Fixed(1).apply((14, 28)), (14, 28), "the author's literal pixels");
        // Native is what an unconfigured lanthorn does: hand the game exactly what the
        // terminal reports, so no game's pixel constants are moved under it.
        assert_eq!(Native.apply((14, 28)), (14, 28));
        assert_eq!(Native.apply((7, 14)), (7, 14));
        assert_eq!(Fixed(2).apply((14, 28)), (7, 14));
        assert_eq!(Fixed(0).apply((14, 28)), (14, 28), "0 must not divide by zero");
        assert_eq!(Fixed(99).apply((14, 28)), (1, 1), "never reports a zero-size cell");
    }

    /// The toolbar row count that falls out of it: advent asks for 36px, and the point
    /// of normalising is that the answer stops depending on the font.
    #[test]
    fn glk_pixel_scale_auto_gives_the_same_row_count_at_every_font_size() {
        for cell_h in [8u32, 10, 12, 14, 17, 20, 21, 24, 28, 34, 42] {
            let (_, seen) = GlkPixelScale::Auto.apply((cell_h / 2, cell_h));
            let rows = 36u32.div_ceil(seen);
            assert_eq!(
                rows, 3,
                "a {cell_h}px cell must still give advent's 36px toolbar 3 rows (saw {seen}px)"
            );
        }
    }

    #[test]
    fn glk_pixel_scale_defaults_to_native_and_round_trips() {
        assert_eq!(
            Config::default().glk_pixel_scale,
            GlkPixelScale::Native,
            "normalisation is OPT-IN: it fixes a game with too-small pixel constants \
             and breaks one with too-large (SQ-0593)"
        );
        let c: Config = toml::from_str("glk_pixel_scale = 2").unwrap();
        assert_eq!(c.glk_pixel_scale, GlkPixelScale::Fixed(2));
        let c: Config = toml::from_str("glk_pixel_scale = \"auto\"").unwrap();
        assert_eq!(c.glk_pixel_scale, GlkPixelScale::Auto);
        let c: Config = toml::from_str("glk_pixel_scale = \"native\"").unwrap();
        assert_eq!(c.glk_pixel_scale, GlkPixelScale::Native);
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c.glk_pixel_scale, GlkPixelScale::Native, "absent is native");
    }

    #[test]
    fn v6_render_defaults_to_hybrid() {
        assert_eq!(Config::default().v6_render, V6RenderMode::Hybrid);
    }

    #[test]
    fn v6_render_parses_raster_from_toml() {
        let c: Config = toml::from_str("v6_render = \"raster\"").unwrap();
        assert_eq!(c.v6_render, V6RenderMode::Raster);
    }

    #[test]
    fn v6_render_absent_is_hybrid() {
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c.v6_render, V6RenderMode::Hybrid);
    }

    /// SQ-0895 removed the `frameless` mode. A config still naming it — and any
    /// other unrecognised token — must launch the game on the default rather
    /// than refuse to parse, which is what `deserialize_v6_render` buys.
    #[test]
    fn v6_render_unknown_mode_falls_back_to_hybrid_silently() {
        let c: Config = toml::from_str("v6_render = \"frameless\"").unwrap();
        assert_eq!(c.v6_render, V6RenderMode::Hybrid, "the removed mode reads as the default");
        let c: Config = toml::from_str("v6_render = \"rastr\"").unwrap();
        assert_eq!(c.v6_render, V6RenderMode::Hybrid, "a typo reads as the default too");
        // …and the fallback must not swallow the modes that DO still exist.
        let c: Config = toml::from_str("v6_render = \"raster\"").unwrap();
        assert_eq!(c.v6_render, V6RenderMode::Raster);
        let c: Config = toml::from_str("v6_render = \"extended\"").unwrap();
        assert_eq!(c.v6_render, V6RenderMode::Extended);
    }

    /// SQ-1032: the third mode has to survive the file the same way, and its token
    /// is the one [`v6_render_key`] writes.
    #[test]
    fn v6_render_extended_round_trips_through_writer() {
        let dir = std::env::temp_dir().join(format!("lanthorn-v6-ext-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = Config::default();
        cfg.v6_render = V6RenderMode::Extended;
        write_config(&dir, &cfg).unwrap();
        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        assert!(text.contains("v6_render = \"extended\""), "the file must hold the token: {text}");
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.v6_render, V6RenderMode::Extended, "extended must survive save→load");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Retargeted from `v6_render_frameless_round_trips_through_writer` (SQ-0895):
    /// the property is that a NON-DEFAULT mode survives save→load, and raster is
    /// now the only non-default there is.
    #[test]
    fn v6_render_raster_round_trips_through_writer() {
        let dir = std::env::temp_dir().join(format!("lanthorn-v6-raster-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = Config::default();
        cfg.v6_render = V6RenderMode::Raster;
        write_config(&dir, &cfg).unwrap();
        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.v6_render, V6RenderMode::Raster, "raster must survive save→load");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0816: the dither preference defaults to FUSED — what the card did to
    /// the eye, and what SQ-0797 measured as correct — and the off position
    /// survives a save→load, because a preference that silently reverts is worse
    /// than no preference at all.
    #[test]
    fn fuse_art_dither_defaults_on_and_round_trips_off() {
        assert!(Config::default().fuse_art_dither, "the shipped default fuses");
        let absent: Config = toml::from_str("").unwrap();
        assert!(absent.fuse_art_dither, "an absent key is the default, not false");
        let off: Config = toml::from_str("fuse_art_dither = false\n").unwrap();
        assert!(!off.fuse_art_dither);

        let dir = std::env::temp_dir().join(format!("lanthorn-fuse-dither-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = Config::default();
        cfg.fuse_art_dither = false;
        write_config(&dir, &cfg).unwrap();
        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert!(!back.fuse_art_dither, "keeping the dither must survive save→load");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_config_round_trips_scalars_and_preserves_keymap() {
        let dir = std::env::temp_dir().join(format!("lanthorn_write_config_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Write initial config with a [keymap] section and a comment.
        let initial = "# lanthorn config\n[keymap]\nzoom_in = \"z\"\n";
        std::fs::write(dir.join("config.toml"), initial).unwrap();

        let cfg = Config {
            version: CONFIG_SCHEMA_VERSION,
            user_dir: dir.clone(),
            default_story_dir: None,
            auto_load: false,
            auto_save: true,
            mouse_wheel_invert: false,
            mouse: true,
            command_bar: false,
            prompt_save_on_quit: true,
            prompt_load_on_launch: true,
            record_turn_history: false,
            history_turns: default_history_turns(),
            hint_skip_screen_warning: true,
            guidance: true,
            guidance_probe: true,
            return_probe: true,
            hide_adult_words: true,
            adult_words: default_adult_words(),
            background_tidy: BackgroundTidy::OnOverlap,
            aux_storage: AuxStorage::Ask,
            v6_render: V6RenderMode::Hybrid,
            fuse_art_dither: false,
            glk_pixel_scale: GlkPixelScale::Native,
            v6_arrow_keys: true,
            v6_pixel_lock: false,
            system_font_disk: "Workbench 1.3".into(),
            keymap: KeymapConfig::default(),
            hotkeys: HotkeysConfig::default(),
            style: Some("neon".into()),
            watch_style: false,
            font_check_pending: false,
            undo_levels: 16,
            command_prefix: '/',
            show_room_numbers: false,
            show_status_bar: true,
            honor_game_colours: true,
            period_look: true,
            honor_timed_input: true,
            config_file: default_config_file(),
            config_error: None,
            interpreter_number: None,
            random_seed: None,
            one_run: OneRunOverrides::default(),
            interpreter_profile: crate::interpreter::InterpreterProfile::default(),
            interpreter_source: crate::interpreter::ProfileSource::Fallback,
            system_colours: default_system_colours(),
            colour_source: ColourSource::default(),
            pictures_override: None,
            disk_entry: None,
            enable_sound: true,
            volume: 100,
            search: SearchConfig::default(),
            virtual_screen_cols: None,
            virtual_screen_rows: None,
            split_ratio: 70,
            command_band: CommandBandConfig::default(),
            inv_dock_pct: 25,
            room_dock_pct: 25,
            text_margin_x: 0,
            text_margin_y: 0,
            animation: AnimationConfig::default(),
            acceleration: true,
            image_protocol: ImageProtocol::Auto,
            images: true,
            trace: crate::trace::TraceSections::default(),
        };
        write_config(&dir, &cfg).unwrap();

        let content = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let doc: toml_edit::DocumentMut = content.parse().unwrap();

        // Scalars are set.
        assert_eq!(doc["auto_load"].as_bool(), Some(false));
        assert_eq!(doc["auto_save"].as_bool(), Some(true));
        assert_eq!(doc["background_tidy"].as_str(), Some("on_overlap"));
        assert_eq!(doc["split_ratio"].as_integer(), Some(70));
        assert_eq!(doc["inv_dock_pct"].as_integer(), Some(25));
        assert_eq!(doc["room_dock_pct"].as_integer(), Some(25), "the room panel's height persists too");
        // SQ-0573: `mouse` is at its DEFAULT and the pre-existing file did not carry
        // it, so it is deliberately not written — a default belongs in the commented
        // template, not as a live key. `user_dir` here is the test's temp dir, so it
        // differs from the default and IS written.
        assert!(doc.get("mouse").is_none(), "a default absent from the file stays absent");
        assert_eq!(doc["user_dir"].as_str(), Some(dir.to_string_lossy().as_ref()));
        // Style pointer is written; visual sections are NOT.
        assert_eq!(doc["style"].as_str(), Some("neon"));
        assert!(!content.contains("[colors]"));
        assert!(!content.contains("[symbols]"));
        // Keymap is preserved.
        assert_eq!(doc["keymap"]["zoom_in"].as_str(), Some("z"));
        // Comment is in the raw text.
        assert!(content.contains("# lanthorn config"), "comment must be preserved");

        // SQ-0573, the other half: a key the file ALREADY has is always rewritten, even
        // when it now holds the default — otherwise flipping a setting back to its
        // default would leave the old value in the file and silently revert on the next
        // launch. Nothing is removed, so a comment the user wrote above their own key
        // (toml_edit attaches it to that key) survives too.
        let with_key = format!("# mine\nauto_load = false\n{initial}");
        std::fs::write(dir.join("config.toml"), &with_key).unwrap();
        let mut back_to_default = cfg.clone();
        back_to_default.auto_load = true; // the default
        write_config(&dir, &back_to_default).unwrap();
        let content = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let doc: toml_edit::DocumentMut = content.parse().unwrap();
        assert_eq!(doc["auto_load"].as_bool(), Some(true), "a present key is updated to its default");
        assert!(content.contains("# mine"), "the user's own comment above it survives");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_config_creates_file_and_dir_when_missing() {
        // Settings-save must create config.toml (and its parent) from scratch.
        let dir = std::env::temp_dir()
            .join(format!("lanthorn_write_config_new_{}", std::process::id()))
            .join("nested");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!dir.exists());
        let mut cfg = Config::default();
        cfg.user_dir = dir.clone();
        write_config(&dir, &cfg).unwrap();
        assert!(dir.join("config.toml").exists(), "config.toml must be created when missing");
        let content = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        assert!(!content.contains("[colors]"), "config.toml must not carry style sections");
        assert!(!content.contains("[symbols]"));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn write_config_persists_panel_editable_scalars_round_trip() {
        // Regression: undo_levels / watch_style / record_turn_history /
        // hint_skip_screen_warning are settings-panel-editable but were absent
        // from write_config, so a saved edit reverted to the default on restart.
        // Round-trip NON-default values through the writer and a fresh parse.
        let dir = std::env::temp_dir().join(format!("lanthorn-cfg-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut cfg = Config::default();
        cfg.undo_levels = 3; // default 16
        cfg.watch_style = true; // default false
        cfg.record_turn_history = true; // default false
        cfg.hint_skip_screen_warning = false; // default true
        write_config(&dir, &cfg).unwrap();

        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.undo_levels, 3, "undo_levels must survive save→load");
        assert!(parsed.watch_style, "watch_style must survive save→load");
        assert!(parsed.record_turn_history, "record_turn_history must survive save→load");
        assert!(!parsed.hint_skip_screen_warning, "hint_skip_screen_warning must survive save→load");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-1112: the owed-question note round-trips, and — the load-bearing half —
    /// an EMPTY config reads as owing nothing.
    ///
    /// The test harnesses seed an empty `config.toml` to make themselves not a
    /// first run, and an empty file parses with every key at its default. If this
    /// key ever defaulted to "owed", the font-check prompt would reappear in front
    /// of fourteen group binaries, `gallery` and `pty_capture` — which is SQ-1104's
    /// guard 2, and the reason SQ-1112 was filed rather than fixed in place.
    /// Owing is opt-in, and this is the case that says so.
    #[test]
    fn an_empty_config_owes_no_font_check_and_the_note_survives_a_round_trip() {
        let empty: Config = toml::from_str("").unwrap();
        assert!(
            !empty.font_check_pending,
            "an empty config.toml must read as owing nothing, or the harness guard breaks"
        );
        assert!(!Config::default().font_check_pending, "and so must the default");

        let dir = std::env::temp_dir()
            .join(format!("lanthorn-cfg-fcp-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut cfg = Config::default();
        cfg.font_check_pending = true;
        write_config(&dir, &cfg).unwrap();
        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert!(parsed.font_check_pending, "the note must survive save→load");

        // …and answering the question takes the line back out rather than leaving
        // `font_check_pending = false` behind in a file the player reads.
        cfg.font_check_pending = false;
        write_config(&dir, &cfg).unwrap();
        let cleared = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let reparsed: Config = toml::from_str(&cleared).unwrap();
        assert!(!reparsed.font_check_pending, "clearing it must reload as cleared");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_reads_style_pointer() {
        let cfg: Config = toml::from_str("style = \"neon\"\n").unwrap();
        assert_eq!(cfg.style.as_deref(), Some("neon"));
    }

    #[test]
    fn mouse_capture_defaults_off_and_opts_in_from_file() {
        // Absent from the file → mouse capture stays off (the responsive default).
        let default: Config = toml::from_str("").unwrap();
        assert!(default.mouse, "mouse capture defaults on");
        // Explicit opt-in is honored.
        let on: Config = toml::from_str("mouse = true\n").unwrap();
        assert!(on.mouse, "mouse = true must enable capture");
    }

    #[test]
    fn command_bar_defaults_off_and_opts_in_from_file() {
        // Absent from the file → command bar stays off (inline prompt is the default).
        let default: Config = toml::from_str("").unwrap();
        assert!(!default.command_bar, "command_bar must default off");
        // Explicit opt-in is honored.
        let on: Config = toml::from_str("command_bar = true\n").unwrap();
        assert!(on.command_bar, "command_bar = true must enable the command bar");
    }

    #[test]
    fn command_bar_round_trips_through_toml() {
        let dir = std::env::temp_dir().join(format!("lanthorn_command_bar_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut cfg = Config::default();
        cfg.user_dir = dir.clone();
        cfg.command_bar = true;
        write_config(&dir, &cfg).unwrap();

        let content = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let doc: toml_edit::DocumentMut = content.parse().unwrap();
        assert_eq!(doc["command_bar"].as_bool(), Some(true));

        let reparsed: Config = toml::from_str(&content).unwrap();
        assert!(reparsed.command_bar, "command_bar = true must round-trip");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prompt_flags_default_true_and_round_trip() {
        assert!(Config::default().prompt_save_on_quit);
        assert!(Config::default().prompt_load_on_launch);
        // Setting one to false parses correctly, other keeps default true.
        let cfg: Config = toml::from_str("prompt_save_on_quit = false\n").unwrap();
        assert!(!cfg.prompt_save_on_quit);
        assert!(cfg.prompt_load_on_launch);
    }

    #[test]
    fn search_config_defaults_and_round_trip() {
        let d = Config::default();
        assert!(d.search.start_backward);
        assert_eq!(d.search.key_back, 'n');
        assert_eq!(d.search.key_forward, 'N');
        let cfg: Config = toml::from_str("[search]\nstart_backward = false\nkey_forward = \"j\"\n").unwrap();
        assert!(!cfg.search.start_backward);
        assert_eq!(cfg.search.key_forward, 'j');
        assert_eq!(cfg.search.key_back, 'n'); // default kept
    }

    #[test]
    fn write_config_does_not_emit_style_sections() {
        let dir = std::env::temp_dir().join(format!(
            "lanthorn_write_config_no_style_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // seed a config with functional + a [keymap] to confirm preservation
        std::fs::write(dir.join("config.toml"), "auto_save = true\n[keymap]\nquit = \"q\"\n").unwrap();
        let mut cfg = Config::default();
        cfg.auto_save = true;
        write_config(&dir, &cfg).unwrap();
        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        assert!(!text.contains("[colors]"));
        assert!(!text.contains("[symbols]"));
        assert!(text.contains("[keymap]")); // functional sections preserved

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn animation_config_defaults() {
        let c = Config::default();
        assert!(c.animation.enabled);
        assert_eq!(c.animation.easing, Easing::EaseOut);
        assert_eq!(c.animation.scroll_ms, 120);
    }

    #[test]
    fn animation_config_absent_uses_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert!(cfg.animation.enabled);
        assert_eq!(cfg.animation.easing, Easing::EaseOut);
        assert_eq!(cfg.animation.scroll_ms, 120);
        assert_eq!(cfg.animation.scrollbar_hide_ms, 1500);
        assert_eq!(cfg.animation.scrollbar_fade_ms, 300);
    }

    /// SQ-0782: the story-pane scrollbar's hide delay and fade are config keys,
    /// not constants — they must parse and round-trip like every other one.
    #[test]
    fn scrollbar_auto_hide_keys_parse_and_round_trip() {
        let cfg: Config = toml::from_str(
            "[animation]\nscrollbar_hide_ms = 400\nscrollbar_fade_ms = 0\n",
        )
        .unwrap();
        assert_eq!(cfg.animation.scrollbar_hide_ms, 400);
        assert_eq!(cfg.animation.scrollbar_fade_ms, 0);

        let dir = std::env::temp_dir().join(format!("bm_sbcfg_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut written = Config::default();
        written.animation.scrollbar_hide_ms = 400;
        written.animation.scrollbar_fade_ms = 0;
        write_config(&dir, &written).unwrap();
        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let doc: toml_edit::DocumentMut = text.parse().unwrap();
        assert_eq!(doc["animation"]["scrollbar_hide_ms"].as_integer(), Some(400));
        assert_eq!(doc["animation"]["scrollbar_fade_ms"].as_integer(), Some(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn animation_config_parses_table() {
        let cfg: Config = toml::from_str(
            "[animation]\nenabled = false\neasing = \"linear\"\nscroll_ms = 200\n",
        )
        .unwrap();
        assert!(!cfg.animation.enabled);
        assert_eq!(cfg.animation.easing, Easing::Linear);
        assert_eq!(cfg.animation.scroll_ms, 200);
    }

    #[test]
    fn animation_config_unknown_easing_falls_back_to_ease_out() {
        let cfg: Config = toml::from_str("[animation]\neasing = \"wobble\"\n").unwrap();
        assert_eq!(cfg.animation.easing, Easing::EaseOut);
    }

    #[test]
    fn write_config_round_trips_animation() {
        let dir = std::env::temp_dir().join(format!(
            "lanthorn_write_config_anim_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config::default();
        cfg.animation = AnimationConfig {
            enabled: false,
            easing: Easing::EaseInOut,
            scroll_ms: 250,
            ..Default::default()
        };
        write_config(&dir, &cfg).unwrap();
        let content = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let doc: toml_edit::DocumentMut = content.parse().unwrap();
        assert_eq!(doc["animation"]["enabled"].as_bool(), Some(false));
        assert_eq!(doc["animation"]["easing"].as_str(), Some("ease-in-out"));
        assert_eq!(doc["animation"]["scroll_ms"].as_integer(), Some(250));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn honor_game_colours_defaults_true() {
        let c = Config::default();
        assert!(c.honor_game_colours);
        // round-trips through TOML: absent key keeps the default true
        let back: Config = toml::from_str("").unwrap();
        assert!(back.honor_game_colours);
        // explicit false overrides the default
        let off: Config = toml::from_str("honor_game_colours = false\n").unwrap();
        assert!(!off.honor_game_colours);
    }

    #[test]
    fn acceleration_defaults_true_and_accel_off_disables() {
        assert!(Config::default().acceleration);

        let cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(PathBuf::from("/nonexistent/path/config.toml")),
            accel: Some(OnOff::Off),
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: None,
            game_colours: None,
            colour: None,
            interpreter_number: None,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        };
        let cfg = resolve(&cli);
        assert!(!cfg.acceleration);
    }

    #[test]
    fn the_sound_flag_moves_enable_sound_both_ways() {
        let base = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(PathBuf::from("/nonexistent/path/config.toml")),
            accel: None,
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: None,
            game_colours: None,
            colour: None,
            interpreter_number: None,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        };
        // Absent flag: sound stays on (config default).
        assert!(resolve(&base).enable_sound);
        // Flag present: sound forced off for this run.
        let muted = Cli { sound: Some(OnOff::Off), ..base };
        assert!(!resolve(&muted).enable_sound);
        // The other direction — which `--no-sound` had no way to say (SQ-1082) —
        // is pinned against a persisted `false` in
        // `the_sound_flag_moves_enable_sound_both_ways_for_one_run_only`.
    }

    /// SQ-1079: `--v6-render` and `--v6-pixel-lock` beat the file for the launch
    /// they were typed on, and — because both land on keys `write_config_at`
    /// persists — a settings save afterwards must leave the file exactly as it
    /// was. One capture in raster must not make raster the mode every story
    /// opens in.
    #[test]
    fn the_v6_render_flags_beat_the_file_and_are_not_written_back() {
        let dir = std::env::temp_dir().join(format!("bm-v6cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        // Both keys PRESENT, and each the opposite of what the flags will ask
        // for — the case `put` rewrites regardless of whether it is the default.
        std::fs::write(&cfg_path, "v6_render = \"hybrid\"\nv6_pixel_lock = true\n").unwrap();
        let base = Cli {
            story: Some(PathBuf::from("foo.z6")),
            user_dir: None,
            data_dir: None,
            config: Some(cfg_path.clone()),
            accel: None,
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: None,
            game_colours: None,
            colour: None,
            interpreter_number: None,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        };
        // Absent flags: the file governs, as it always did.
        let plain = resolve(&base);
        assert_eq!(plain.v6_render, V6RenderMode::Hybrid);
        assert!(plain.v6_pixel_lock);

        let mut cfg = resolve(&Cli {
            v6_render: Some(V6RenderMode::Raster),
            v6_pixel_lock: Some(OnOff::Off),
            ..base
        });
        assert_eq!(cfg.v6_render, V6RenderMode::Raster, "the flag governs the run");
        assert!(!cfg.v6_pixel_lock, "the flag governs the run");

        write_config_at(&cfg_path, &cfg).unwrap();
        let back: Config = toml::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
        assert_eq!(back.v6_render, V6RenderMode::Hybrid, "the FILE still says hybrid");
        assert!(back.v6_pixel_lock, "the FILE still says true");

        // …and a deliberate edit of either row (which releases the pin) persists
        // like any other setting.
        cfg.one_run.release(keys::V6_RENDER);
        cfg.one_run.release(keys::V6_PIXEL_LOCK);
        write_config_at(&cfg_path, &cfg).unwrap();
        let back: Config = toml::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
        assert_eq!(back.v6_render, V6RenderMode::Raster);
        assert!(!back.v6_pixel_lock);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The pin only holds while the value is still the flag's, so the token the
    /// pin carries has to be the one `write_config_at` writes. Two copies of
    /// that `match` is how they would drift, which is why there is one
    /// ([`v6_render_key`]) — asserted here rather than left to inspection.
    #[test]
    fn the_pinned_v6_render_token_is_the_one_the_file_holds() {
        for mode in [V6RenderMode::Hybrid, V6RenderMode::Raster, V6RenderMode::Extended] {
            let mut cfg = Config::default();
            cfg.v6_render = mode;
            cfg.one_run.pin(keys::V6_RENDER, v6_render_key(mode));
            let dir = std::env::temp_dir()
                .join(format!("bm-v6token-{}-{}", std::process::id(), v6_render_key(mode)));
            std::fs::create_dir_all(&dir).unwrap();
            let cfg_path = dir.join("config.toml");
            // Present and set to the OTHER mode, so an ineffective pin is visible
            // as a rewrite rather than as an absence.
            let other = if mode == V6RenderMode::Raster { "hybrid" } else { "raster" };
            std::fs::write(&cfg_path, format!("v6_render = \"{other}\"\n")).unwrap();
            write_config_at(&cfg_path, &cfg).unwrap();
            let back = std::fs::read_to_string(&cfg_path).unwrap();
            assert!(
                back.contains(&format!("v6_render = \"{other}\"")),
                "the pin on {mode:?} did not hold: {back}"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn v6_arrow_keys_persists_from_config_file() {
        // SQ-1087: the shipped default WITHHOLDS arrows from a v6 story, so
        // they keep driving scrollback and map panning the way they do in every
        // other story. This is a config-only setting (the --no-v6-arrows CLI
        // flag was retired), so an opted-in `v6_arrow_keys = true` must survive
        // resolve, and an absent key must resolve to the withholding default —
        // by both routes into a Config, `Default` and serde.
        assert!(!Config::default().v6_arrow_keys, "the shipped default withholds");
        let absent_key: Config = toml::from_str("").unwrap();
        assert!(!absent_key.v6_arrow_keys, "an absent key is the default, not true");

        let dir = std::env::temp_dir().join(format!("bm-v6arrows-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, "v6_arrow_keys = true\n").unwrap();
        let base = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(cfg_path.clone()),
            accel: None,
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: None,
            game_colours: None,
            colour: None,
            interpreter_number: None,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        };
        assert!(resolve(&base).v6_arrow_keys, "persisted true must hold");
        let absent = Cli { config: Some(dir.join("missing.toml")), ..base };
        assert!(!resolve(&absent).v6_arrow_keys, "default withholds arrows from v6");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_parses_trace_flag() {
        let mut cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(PathBuf::from("/nonexistent/path/config.toml")),
            accel: None,
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: None,
            game_colours: None,
            colour: None,
            interpreter_number: None,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        };
        cli.trace = Some("screen,map".to_string());
        let cfg = resolve(&cli);
        assert!(cfg.trace.screen && cfg.trace.map && !cfg.trace.hostio);
    }

    #[test]
    fn images_defaults_true_and_images_off_disables() {
        assert!(Config::default().images);

        let cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(PathBuf::from("/nonexistent/path/config.toml")),
            accel: None,
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: Some(OnOff::Off),
            game_colours: None,
            colour: None,
            interpreter_number: None,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        };
        let cfg = resolve(&cli);
        assert!(!cfg.images);
    }

    #[test]
    fn honor_timed_input_defaults_true() {
        let c = Config::default();
        assert!(c.honor_timed_input);
        // round-trips through TOML: absent key keeps the default true
        let back: Config = toml::from_str("").unwrap();
        assert!(back.honor_timed_input);
        // explicit false overrides the default
        let off: Config = toml::from_str("honor_timed_input = false\n").unwrap();
        assert!(!off.honor_timed_input);
    }

    #[test]
    fn enable_sound_defaults_true() {
        assert!(Config::default().enable_sound);
        let back: Config = toml::from_str("").unwrap();
        assert!(back.enable_sound, "absent key keeps default true");
        let off: Config = toml::from_str("enable_sound = false\n").unwrap();
        assert!(!off.enable_sound);
    }

    #[test]
    fn volume_defaults_100_and_roundtrips() {
        assert_eq!(Config::default().volume, 100);
        let back: Config = toml::from_str("").unwrap();
        assert_eq!(back.volume, 100, "absent key keeps default 100");
        let set: Config = toml::from_str("volume = 40\n").unwrap();
        assert_eq!(set.volume, 40);
    }

    /// SQ-0574: `--user-dir` must move the config READ as well as the writes. It used
    /// to be ignored by `config_path`, so `lanthorn --user-dir /tmp/x` seeded and saved
    /// `/tmp/x/config.toml` while still loading `~/.lanthorn/config.toml` — every
    /// setting it wrote was silently discarded on the next launch.
    #[test]
    fn user_dir_moves_the_config_file_and_a_save_round_trips() {
        let dir = std::env::temp_dir().join(format!("bm-userdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: Some(dir.clone()),
            data_dir: None,
            config: None,
            accel: None,
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: None,
            game_colours: None,
            colour: None,
            interpreter_number: None,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        };
        // The read path follows --user-dir, and the resolved config remembers it.
        assert_eq!(config_path(&cli), dir.join("config.toml"));
        let mut cfg = resolve(&cli);
        assert_eq!(cfg.config_file, dir.join("config.toml"));

        // A save lands there — not in the default home — and loads back.
        cfg.volume = 42;
        write_config_file(&cfg).unwrap();
        assert!(dir.join("config.toml").is_file(), "the save went to the --user-dir");
        assert_eq!(resolve(&cli).volume, 42, "and the next launch reads it back");

        // --config still wins over --user-dir for the file's location.
        let elsewhere = dir.join("other.toml");
        std::fs::write(&elsewhere, "volume = 7\n").unwrap();
        let pinned = Cli { config: Some(elsewhere.clone()), ..cli };
        assert_eq!(config_path(&pinned), elsewhere);
        let pinned_cfg = resolve(&pinned);
        assert_eq!(pinned_cfg.volume, 7);
        assert_eq!(pinned_cfg.config_file, elsewhere);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The `user_dir` KEY inside the file names the data root; it must NOT redirect
    /// where the file itself is saved, or a save would land somewhere `resolve` never
    /// reads (the second half of SQ-0574, and the reason `write_config_file` exists).
    #[test]
    fn a_user_dir_key_in_the_file_does_not_move_the_file() {
        let home = std::env::temp_dir().join(format!("bm-udkey-{}", std::process::id()));
        let data = home.join("data-root");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        // A TOML *literal* string (single quotes) — no escape processing. A basic
        // double-quoted string would read the `\U` of a Windows `C:\Users\…` temp
        // path as TOML's 8-hex-digit unicode escape and fail to parse the file at
        // all, and `resolve` silently falls back to defaults on a parse error.
        std::fs::write(home.join("config.toml"), format!("user_dir = '{}'\n", data.display())).unwrap();

        let cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(home.join("config.toml")),
            accel: None,
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: None,
            game_colours: None,
            colour: None,
            interpreter_number: None,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        };
        let mut cfg = resolve(&cli);
        assert_eq!(cfg.user_dir, data, "the key still names the data root");
        assert_eq!(cfg.config_file, home.join("config.toml"), "but not the config's own location");

        cfg.volume = 33;
        write_config_file(&cfg).unwrap();
        assert!(!data.join("config.toml").exists(), "nothing is written into the data root");
        assert_eq!(resolve(&cli).volume, 33, "the save is where resolve reads");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// SQ-0580: a config file that doesn't parse costs the user every setting in it —
    /// TOML is one document, so there is no partial load. That much is unavoidable;
    /// doing it in silence is not. `resolve` must report the parse error so startup can
    /// tell the user which line broke instead of quietly running on defaults.
    #[test]
    fn a_malformed_config_is_reported_rather_than_swallowed() {
        let dir = std::env::temp_dir().join(format!("bm-badcfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        // `volume` would take effect if the file parsed; the unterminated string on the
        // next line means none of it does.
        std::fs::write(&path, "volume = 42\nstyle = \"neon\n").unwrap();

        let cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(path.clone()),
            accel: None,
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: None,
            game_colours: None,
            colour: None,
            interpreter_number: None,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        };
        let cfg = resolve(&cli);
        assert!(cfg.config_error.is_some(), "the parse failure is reported");
        assert_eq!(cfg.volume, Config::default().volume, "and nothing from the file loaded");

        // A file that parses leaves the field clear — the error must not be sticky.
        std::fs::write(&path, "volume = 42\n").unwrap();
        let good = resolve(&cli);
        assert_eq!(good.config_error, None, "a valid file reports no error");
        assert_eq!(good.volume, 42);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0580, the destructive half: `write_config_at` parsed the existing file into a
    /// toml_edit doc and fell back to an EMPTY doc when that failed, so the first
    /// settings save after a typo replaced the user's whole config — keys, comments and
    /// all — with a handful of defaults. Refuse the write and keep the file byte-exact.
    #[test]
    fn a_save_refuses_to_clobber_a_malformed_config() {
        let dir = std::env::temp_dir().join(format!("bm-badsave-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let original = "# my carefully commented config\nvolume = 42\nstyle = \"neon\n";
        std::fs::write(&path, original).unwrap();

        let mut cfg = Config { config_file: path.clone(), ..Config::default() };
        cfg.volume = 33;
        let err = write_config_file(&cfg).expect_err("a malformed file is not overwritten");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("not valid TOML"), "and it says why: {err}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "the user's file survives untouched, comments included",
        );

        // Once the file is well-formed again, saving works normally.
        std::fs::write(&path, "# my carefully commented config\nvolume = 42\n").unwrap();
        write_config_file(&cfg).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("# my carefully commented config"), "comments preserved: {after}");
        assert!(after.contains("volume = 33"), "and the new value landed: {after}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `--sound off` silences ONE run. Before SQ-0807 `write_config_at` put
    /// `enable_sound` unconditionally, so the first settings save of a `--sound off`
    /// session — the story browser's "remember this directory?" prompt is enough —
    /// wrote `enable_sound = false` into config.toml and every later launch was
    /// silent, with nothing on screen to say why.
    #[test]
    fn sound_off_flag_does_not_persist_enable_sound() {
        let dir = std::env::temp_dir().join(format!("bm-nosound-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        // Present AND at its default — the case `put` rewrites either way.
        std::fs::write(&cfg_path, "# mine\nenable_sound = true\n").unwrap();

        let cli = Cli { sound: Some(OnOff::Off), ..cli_with_config(&cfg_path, None) };
        let mut cfg = resolve(&cli);
        assert!(!cfg.enable_sound, "the flag silences this run");

        write_config(&dir, &cfg).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            toml::from_str::<Config>(&back).unwrap().enable_sound,
            "--sound off is for one run; the FILE must still say true: {back}"
        );
        assert!(back.contains("# mine"), "and the user's comment survives: {back}");

        // The settings panel turning sound off IS a decision, and it persists.
        cfg.one_run.release(keys::ENABLE_SOUND);
        write_config(&dir, &cfg).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(!toml::from_str::<Config>(&back).unwrap().enable_sound, "an explicit off persists");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `--user-dir` relocates the data root for one run. With `--config` naming a
    /// different file, a settings save used to stamp that temporary root into the
    /// user's real config — so a single `--user-dir /tmp/x` run left every later
    /// launch reading maps and saves out of `/tmp/x` (SQ-0807).
    #[test]
    fn user_dir_flag_does_not_persist_into_a_named_config() {
        // Tag distinct from `user_dir_moves_the_config_file_…`: the two share a pid
        // whenever the binary runs its tests in threads, and both wipe the dir first.
        let dir = std::env::temp_dir().join(format!("bm-userdir-named-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        let real_root = dir.join("real");
        let one_run_root = dir.join("scratch");
        std::fs::write(&cfg_path, format!("user_dir = {:?}\n", real_root.to_string_lossy())).unwrap();

        let cli = Cli {
            user_dir: Some(one_run_root.clone()),
            ..cli_with_config(&cfg_path, None)
        };
        let mut cfg = resolve(&cli);
        assert_eq!(cfg.user_dir, one_run_root, "the flag moves the data root for this run");

        write_config_at(&cfg_path, &cfg).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert_eq!(
            toml::from_str::<Config>(&back).unwrap().user_dir,
            real_root,
            "the FILE keeps the user's own root: {back}"
        );

        // Typing a path into the settings panel is a decision, and it persists.
        let chosen = dir.join("chosen");
        cfg.user_dir = chosen.clone();
        cfg.one_run.release(keys::USER_DIR);
        write_config_at(&cfg_path, &cfg).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert_eq!(toml::from_str::<Config>(&back).unwrap().user_dir, chosen, "an edit persists");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Promotion, the half that makes the guard safe to generalise: a value a one-run
    /// source pinned STOPS being one-run the moment something changes it, and from
    /// then on it persists like any other setting — including when the user changes
    /// it straight back to what the flag asked for, which is why the settings panel
    /// releases the pin outright rather than relying on the value differing.
    #[test]
    fn a_panel_edit_promotes_a_one_run_value_to_a_persisted_one() {
        let dir = std::env::temp_dir().join(format!("bm-promote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, "enable_sound = true\n").unwrap();

        let cli = Cli { sound: Some(OnOff::Off), ..cli_with_config(&cfg_path, None) };
        let mut cfg = resolve(&cli);

        // Merely differing from the pin is enough: the pin no longer describes it.
        cfg.enable_sound = true;
        write_config(&dir, &cfg).unwrap();
        assert!(
            std::fs::read_to_string(&cfg_path).unwrap().contains("enable_sound = true"),
            "a value that is no longer the pinned one is written normally"
        );

        // …and changing it BACK to the flag's value still persists, because the
        // panel released the pin when the row was edited.
        cfg.one_run.release(keys::ENABLE_SOUND);
        cfg.enable_sound = false;
        write_config(&dir, &cfg).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            !toml::from_str::<Config>(&back).unwrap().enable_sound,
            "toggling back to the flag's value is still the user's choice: {back}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Artwork that forces `honor_game_colours` off for ONE story must not be
    /// written back (SQ-0806): opening *Zork Zero*'s CGA rendition once would
    /// otherwise bake "never honour game colours" into the global config and
    /// silently strip every other game's colours from then on.
    #[test]
    fn artwork_forcing_game_colours_off_does_not_persist() {
        let dir = std::env::temp_dir().join(format!("bm-honor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        // The key is PRESENT and true — the case `put` would otherwise rewrite
        // regardless of whether the value is the default.
        std::fs::write(&cfg_path, "honor_game_colours = true\n").unwrap();

        let mut cfg = resolve(&Cli {
            story: Some(PathBuf::from("foo.z6")),
            user_dir: None,
            data_dir: None,
            config: Some(cfg_path.clone()),
            accel: None,
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: None,
            game_colours: None,
            colour: None,
            interpreter_number: None,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        });
        assert!(cfg.honor_game_colours, "the file's value loads");

        // Boot against two-colour artwork: off for this story, pinned as one-run.
        cfg.honor_game_colours = false;
        cfg.one_run.pin(keys::HONOR_GAME_COLOURS, false);

        write_config(&dir, &cfg).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        let reread: Config = toml::from_str(&back).unwrap();
        assert!(
            reread.honor_game_colours,
            "the FILE must still say true — the artwork spoke for one story, not forever"
        );

        // …and an ordinary off (the user's own choice) still persists.
        cfg.one_run.release(keys::HONOR_GAME_COLOURS);
        write_config(&dir, &cfg).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        let reread: Config = toml::from_str(&back).unwrap();
        assert!(!reread.honor_game_colours, "a user's own choice persists as always");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// SQ-0945: a per-game `v6_pixel_lock` is one story's preference, and a later
    /// settings save must not turn it into everyone's. Same guard as the artwork
    /// force above, reached from a different source — the game's own sidecar.
    #[test]
    fn a_per_game_v6_pixel_lock_does_not_persist_to_the_global_config() {
        let dir = std::env::temp_dir().join(format!("bm-pixellock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        // Present and false — the case `put` rewrites even at the default.
        std::fs::write(&cfg_path, "v6_pixel_lock = false
").unwrap();

        let mut cfg = resolve(&Cli {
            story: Some(PathBuf::from("foo.z6")),
            user_dir: None,
            data_dir: None,
            config: Some(cfg_path.clone()),
            accel: None,
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: None,
            game_colours: None,
            colour: None,
            interpreter_number: None,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        });
        assert!(!cfg.v6_pixel_lock, "the file's value loads");

        // Boot with this game's sidecar saying "lock it", pinned as one-run.
        cfg.v6_pixel_lock = true;
        cfg.one_run.pin(keys::V6_PIXEL_LOCK, true);

        write_config(&dir, &cfg).unwrap();
        let reread: Config = toml::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
        assert!(
            !reread.v6_pixel_lock,
            "the FILE must still say false — one game asked for the lock, not every game"
        );

        // …and the settings screen's own edit (which releases the pin) persists.
        cfg.one_run.release(keys::V6_PIXEL_LOCK);
        write_config(&dir, &cfg).unwrap();
        let reread: Config = toml::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
        assert!(reread.v6_pixel_lock, "a deliberate global edit persists as always");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `--interpreter N` overrides the config file for ONE run, and must not
    /// be written back: a later settings save would otherwise make a throwaway
    /// experiment permanent (and pin one machine for every story, defeating the
    /// per-version auto rule). The header values themselves are ZMSD §11.1.3's table
    /// (1 DECSystem-20 … 6 IBM PC … 11 Tandy Color).
    #[test]
    fn interpreter_number_cli_overrides_file_without_persisting() {
        let dir = std::env::temp_dir().join(format!("bm-interp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, "interpreter_number = 4\n").unwrap();

        let base = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(cfg_path.clone()),
            accel: None,
            sound: None,
            image_protocol: ImageProtocol::Auto,
            images: None,
            game_colours: None,
            colour: None,
            interpreter_number: None,
            interpreter_version: None,
            pictures: None,
            story_pick: None,
            fetch: None,
            import_metadata: None,
            v6_render: None,
            v6_pixel_lock: None,
            machines: false,
            trace: None,
            debug: false,
            guidance: None,
            font_check: None,
        };
        // No flag: the file's Amiga (4) stands, and it is provenance-clean.
        let from_file = resolve(&base);
        assert_eq!(from_file.interpreter_number, Some(4), "the file's value loads");
        assert!(!from_file.interpreter_number_from_cli());

        // Flag present: IBM PC (6) wins for this run, marked as CLI-sourced.
        let overridden = resolve(&Cli { interpreter_number: Some(6), ..base });
        assert_eq!(overridden.interpreter_number, Some(6), "the CLI beats the file");
        assert!(overridden.interpreter_number_from_cli());

        // Writing settings must leave the FILE on 4, not bake in the run's 6.
        write_config(&dir, &overridden).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        let reread: Config = toml::from_str(&back).unwrap();
        assert_eq!(
            reread.interpreter_number,
            Some(4),
            "a one-run --interpreter must not be persisted; file now: {back}"
        );

        // A value that came from the file DOES persist (the settings panel path).
        write_config(&dir, &from_file).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert_eq!(toml::from_str::<Config>(&back).unwrap().interpreter_number, Some(4));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0855. Every flag `lanthorn` accepts, parsed through the real clap surface —
    /// the test that catches a flag renamed in its doc comment and nowhere else, or
    /// renamed in the arg table and left stale in the docs that name it.
    ///
    /// `--interpreter` is the spelling, not `--interpreter-number`: `zvm-cli` has
    /// always called this `-I`/`--interpreter` and one concept under two names across
    /// two binaries is the whole defect. Pre-release, so there is deliberately no
    /// alias — the old spelling must be REJECTED, or nothing would ever have moved.
    #[test]
    fn every_flag_lanthorn_accepts_parses_and_the_old_spelling_is_gone() {
        use clap::Parser;
        for flag in ["--debug", "--machines"] {
            let cli = Cli::try_parse_from(["lanthorn", flag, "g.z5"])
                .unwrap_or_else(|e| panic!("{flag} should parse: {e}"));
            assert_eq!(
                cli.story.as_deref(),
                Some(std::path::Path::new("g.z5")),
                "{flag} should leave the story path alone"
            );
        }
        // Value-taking flags: the value must be consumed, not read as the story.
        for (flag, value) in [
            ("--interpreter", "6"),
            ("--user-dir", "/tmp/x"),
            ("--data-dir", "/tmp/x"),
            ("--config", "/tmp/x.toml"),
            ("--image-protocol", "kitty"),
            ("--trace", "screen"),
            ("--pictures", "g.mg1"),
            ("--story", "arthur"),
            ("--accel", "on"),
            ("--sound", "off"),
            ("--images", "on"),
            ("--game-colours", "off"),
            ("--colour", "machine"),
            ("--fetch", "missing"),
            ("--fetch", "all"),
            ("--import-metadata", "/tmp/rows.tsv"),
        ] {
            let cli = Cli::try_parse_from(["lanthorn", flag, value, "g.z5"])
                .unwrap_or_else(|e| panic!("{flag} {value} should parse: {e}"));
            assert_eq!(
                cli.story.as_deref(),
                Some(std::path::Path::new("g.z5")),
                "{flag} {value} swallowed the story path"
            );
        }
        // The flag sets the field the config key is named after — the FIELD keeps the
        // key's name because that is what it sets; only the spelling on the command
        // line moved.
        let cli = Cli::try_parse_from(["lanthorn", "--interpreter", "4", "g.z5"]).unwrap();
        assert_eq!(cli.interpreter_number, Some(4), "--interpreter sets interpreter_number");
        assert!(cli.game_colours.is_none(), "and is nothing to do with colours");

        // SQ-1082: every negative-only switch is now `--<noun> on|off`, the value
        // is REQUIRED (a bare `--sound` invites "is that on, or a toggle?"), and
        // the old spelling is gone outright rather than surviving as an alias
        // nobody maintains. `--system-colours` went with them: `--colour machine`
        // is the same request said on the axis it belongs to.
        for old in [
            "--no-accel",
            "--no-sound",
            "--no-images",
            "--no-game-colours",
            "--system-colours",
            "--system-colors",
            "--interpreter-number",
        ] {
            assert!(
                Cli::try_parse_from(["lanthorn", old, "g.z5"]).is_err(),
                "{old} is gone outright — no deprecated alias before release"
            );
        }
        for bare in ["--accel", "--sound", "--images", "--game-colours", "--colour"] {
            assert!(
                Cli::try_parse_from(["lanthorn", bare, "g.z5"]).is_err(),
                "{bare} requires its value; a bare form is an ambiguity, not a shorthand"
            );
        }
        // And the help text a user reads names the new spelling, not the old one.
        let help = <Cli as clap::CommandFactory>::command().render_long_help().to_string();
        assert!(help.contains("--interpreter <N>"), "help offers --interpreter: {help}");
        assert!(!help.contains("--interpreter-number"), "and never the old name: {help}");
        for new in ["--accel <ON|OFF>", "--sound <ON|OFF>", "--images <ON|OFF>",
                    "--game-colours <ON|OFF>", "--colour <SOURCE>"] {
            assert!(help.contains(new), "help offers {new}: {help}");
        }
        // No negative-only switch survives in the OPTION column. Matched on the
        // column rather than on the whole text, because prose in a doc comment may
        // legitimately name one (`--no-tap` is `ring_scout`'s, and this help
        // describes lanthorn's neighbours).
        for line in help.lines() {
            let t = line.trim_start();
            assert!(
                !t.starts_with("--no-") && !t.contains(" --no-"),
                "a negative-only switch survives: {line}"
            );
        }
    }

    /// SQ-1093. One wrap authority, and `lanthorn` is one of the four front-ends
    /// that must answer with the same number.
    ///
    /// The reported symptom was a `--help` showing two: prose wrapped at ~83
    /// columns beside a generated list that ran to the terminal's edge, so the
    /// right margin was ragged in a way that reads as a rendering fault. clap's
    /// `wrap_help` reflows to whatever the terminal happens to be, which is a
    /// second authority all by itself — the same paragraph came out at 80 columns
    /// in one window and 200 in another, and never matched `zvm-cli`, whose help
    /// is a string constant. `term_width` pins it; this asserts the pin.
    ///
    /// Rendered through `render_long_help`, which is what `--help` prints, so a
    /// doc comment long enough to overflow fails here rather than on a user's
    /// screen.
    #[test]
    fn every_help_line_fits_the_one_width_all_four_front_ends_share() {
        let cmd = <Cli as clap::CommandFactory>::command();
        for (which, text) in [
            ("--help", cmd.clone().render_long_help().to_string()),
            ("-h", cmd.clone().render_help().to_string()),
        ] {
            let over = cli_host::overlong_help_lines(&text);
            assert!(
                over.is_empty(),
                "{which} must wrap at {}, but {over:?} do not:\n{text}",
                cli_host::HELP_WIDTH
            );
        }
        // Non-vacuity: the help is long enough for the width to mean something.
        let long = cmd.clone().render_long_help().to_string();
        assert!(
            long.lines().filter(|l| l.chars().count() > cli_host::HELP_WIDTH - 10).count() > 5,
            "the text should be filling the width, not merely short of it"
        );
    }

    /// SQ-1078: `--story` names a game ON a volume, and the two spellings must
    /// not be confused — the POSITIONAL says which container to open, the FLAG
    /// says which story on it.
    ///
    /// They share a word on the command line and nothing else: the flag's field
    /// is `story_pick`, because clap ids must be unique, while the long name
    /// stays `--story` to match `zvm-cli`, which has spelled it that way since
    /// SQ-0834. And it requires a path, like `--pictures`: naming a story on
    /// nothing has no referent, and clap saying so beats a chooser that would
    /// have to answer "on what?".
    #[test]
    fn the_story_flag_names_a_story_on_the_path_and_needs_one() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["lanthorn", "disc.img", "--story", "arthur"]).unwrap();
        assert_eq!(cli.story.as_deref(), Some(std::path::Path::new("disc.img")), "the container");
        assert_eq!(cli.story_pick.as_deref(), Some("arthur"), "and which story on it");
        // A number is just as good a way to say it, and stays a string here —
        // `cli_host::story_pick::find` is the one place that decides what it
        // means, for both front-ends.
        let cli = Cli::try_parse_from(["lanthorn", "disc.img", "--story", "7"]).unwrap();
        assert_eq!(cli.story_pick.as_deref(), Some("7"));

        assert!(
            Cli::try_parse_from(["lanthorn", "--story", "arthur"]).is_err(),
            "a story on nothing is a usage error, exactly as --pictures on nothing is"
        );
        let help = <Cli as clap::CommandFactory>::command().render_long_help().to_string();
        assert!(help.contains("--story <N|NAME>"), "help offers the flag: {help}");
    }

    /// SQ-0960: `lanthorn --machines` answers without a story, and answers with
    /// the table `zvm` holds rather than one the TUI keeps of its own.
    ///
    /// Two halves, and the second is the one that matters. Every other flag here
    /// is an instruction ABOUT a story, so clap requires one; this describes the
    /// program, so `startup::resolve_launch` prints and exits before the story
    /// argument is ever looked for. And the string it prints is
    /// [`zvm::machines::table`] verbatim — asserted against
    /// `zvm::interpreter::MACHINES` here rather than against a copy of the
    /// expected output, because a literal pasted out of a passing run is exactly
    /// as wrong as the code it was pasted from.
    #[test]
    fn the_machines_flag_needs_no_story_and_prints_the_table_zvm_holds() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["lanthorn", "--machines"]).expect("no story required");
        assert!(cli.machines, "the flag is set");
        assert!(cli.story.is_none(), "and asks about no story in particular");
        // It is still a flag, so it composes with one.
        let with = Cli::try_parse_from(["lanthorn", "--machines", "g.z5"]).unwrap();
        assert_eq!(with.story.as_deref(), Some(std::path::Path::new("g.z5")));

        // The table itself: a row per modelled machine, from the table.
        let t = zvm::machines::table();
        for m in zvm::interpreter::MACHINES {
            assert!(
                t.lines().any(|l| l.trim_start().starts_with(&format!("{}  ", m.number))
                    && l.contains(m.name)),
                "{} ({}) has no row in what --machines prints:\n{t}",
                m.name,
                m.number,
            );
        }
        // …and the Version-dependent half, which is the part a per-machine row
        // cannot state: the IBM PC's graphics mode moves with the story's
        // Version, and lanthorn is the front-end that plays the Version 6 stories
        // it moves for.
        assert!(t.contains("EGA (XZIP)") && t.contains("EGA (YZIP)"), "both IBM modes named:\n{t}");
        assert!(t.contains("CGA card"), "and the card that is neither:\n{t}");

        // The help a user reads offers it.
        let help = <Cli as clap::CommandFactory>::command().render_long_help().to_string();
        assert!(help.contains("--machines"), "help offers --machines: {help}");
    }

    /// SQ-0855: `--game-colours` is an instruction for one launch, exactly as
    /// `--sound` and `--interpreter` are. It must reach the LIVE
    /// `honor_game_colours` every render site gates on — a flag that set some separate
    /// field would look right at launch and drift the moment `/set-game-colours`
    /// toggled it — and must never be written back to config.toml.
    ///
    /// SQ-1082: and it must move the key BOTH WAYS. `--no-game-colours` could only
    /// ever force them off, so a config carrying `honor_game_colours = false` had no
    /// command line that could override it — you had to edit the file. The three
    /// states are pinned here against both persisted values, which is the whole
    /// point of `Option<OnOff>`: absent has to leave a stored `true` alone as surely
    /// as a stored `false`.
    #[test]
    fn the_game_colours_flag_moves_the_key_both_ways_for_one_run_only() {
        let dir = std::env::temp_dir().join(format!("bm-gamecolours-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        let flagged = |v: Option<OnOff>| Cli { game_colours: v, ..cli_with_config(&cfg_path, None) };

        // ── a config that says TRUE ───────────────────────────────────────────
        // The key is PRESENT and at its default — the case `put` rewrites either way.
        std::fs::write(&cfg_path, "# mine\nhonor_game_colours = true\n").unwrap();
        assert!(resolve(&flagged(None)).honor_game_colours, "absent: the file's true stands");
        assert!(resolve(&flagged(Some(OnOff::On))).honor_game_colours, "on: agrees with it");
        let mut cfg = resolve(&flagged(Some(OnOff::Off)));
        assert!(!cfg.honor_game_colours, "off: the interpreter is declared colourless");

        write_config(&dir, &cfg).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            toml::from_str::<Config>(&back).unwrap().honor_game_colours,
            "--game-colours off is for one run; the FILE must still say true: {back}"
        );
        assert!(back.contains("# mine"), "and the user's comment survives: {back}");

        // The settings panel turning them off IS a decision, and it persists.
        cfg.one_run.release(keys::HONOR_GAME_COLOURS);
        write_config(&dir, &cfg).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            !toml::from_str::<Config>(&back).unwrap().honor_game_colours,
            "an explicit off persists: {back}"
        );

        // ── a config that says FALSE ──────────────────────────────────────────
        // The direction that had no command line at all before SQ-1082.
        std::fs::write(&cfg_path, "# mine\nhonor_game_colours = false\n").unwrap();
        assert!(!resolve(&flagged(None)).honor_game_colours, "absent: an off config is left off");
        assert!(!resolve(&flagged(Some(OnOff::Off))).honor_game_colours, "off: agrees with it");
        let mut cfg = resolve(&flagged(Some(OnOff::On)));
        assert!(cfg.honor_game_colours, "on: the flag OVERRIDES a persisted false");

        write_config(&dir, &cfg).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            !toml::from_str::<Config>(&back).unwrap().honor_game_colours,
            "--game-colours on is for one run too; the FILE must still say false: {back}"
        );
        cfg.one_run.release(keys::HONOR_GAME_COLOURS);
        write_config(&dir, &cfg).unwrap();
        assert!(
            toml::from_str::<Config>(&std::fs::read_to_string(&cfg_path).unwrap())
                .unwrap()
                .honor_game_colours,
            "and an explicit on persists, symmetrically"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-1082: `--sound`, the same three states against both persisted values.
    ///
    /// Sound is the flag the quest was reported on: `--no-sound` forced it off for
    /// a run and nothing could force it ON, so `enable_sound = false` in the file
    /// was only reachable by editing the file.
    #[test]
    fn the_sound_flag_moves_enable_sound_both_ways_for_one_run_only() {
        let dir = std::env::temp_dir().join(format!("bm-soundboth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        let flagged = |v: Option<OnOff>| Cli { sound: v, ..cli_with_config(&cfg_path, None) };

        std::fs::write(&cfg_path, "enable_sound = false\n").unwrap();
        assert!(!resolve(&flagged(None)).enable_sound, "absent: a persisted false survives");
        assert!(!resolve(&flagged(Some(OnOff::Off))).enable_sound, "off: agrees with it");
        let cfg = resolve(&flagged(Some(OnOff::On)));
        assert!(cfg.enable_sound, "on: the flag OVERRIDES a persisted false");
        write_config(&dir, &cfg).unwrap();
        assert!(
            !toml::from_str::<Config>(&std::fs::read_to_string(&cfg_path).unwrap())
                .unwrap()
                .enable_sound,
            "and it is still for one run only"
        );

        std::fs::write(&cfg_path, "enable_sound = true\n").unwrap();
        assert!(resolve(&flagged(None)).enable_sound, "absent: a persisted true survives");
        assert!(!resolve(&flagged(Some(OnOff::Off))).enable_sound, "off: overrides it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-1082, end to end: the real `argv`, through the real clap surface, onto
    /// a real `config.toml`.
    ///
    /// The two tests above build a `Cli` literal, which pins `resolve` and not
    /// the SPELLING; this one types the flag. Both persisted switches, both
    /// stored values, all three states — and the state that matters most is the
    /// flag being ABSENT, which must leave the file's value alone whichever way
    /// it points. That is the regression `Option<OnOff>` exists to prevent: a
    /// bare `bool` reads "not mentioned" as "off" and starts turning persisted
    /// `true` values off.
    #[test]
    fn the_typed_flags_move_both_persisted_switches_both_ways() {
        use clap::Parser;
        let dir = std::env::temp_dir().join(format!("bm-argvboth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        let path = cfg_path.to_string_lossy().into_owned();
        let run = |args: &[&str]| {
            let mut argv = vec!["lanthorn", "--config", &path];
            argv.extend_from_slice(args);
            argv.push("g.z5");
            resolve(&Cli::try_parse_from(argv).expect("the flags parse"))
        };

        for stored in [true, false] {
            std::fs::write(
                &cfg_path,
                format!("enable_sound = {stored}\nhonor_game_colours = {stored}\n"),
            )
            .unwrap();
            let absent = run(&[]);
            assert_eq!(absent.enable_sound, stored, "absent must not move enable_sound");
            assert_eq!(
                absent.honor_game_colours, stored,
                "absent must not move honor_game_colours"
            );
            for (want, sound, colours) in
                [(true, "on", "on"), (false, "off", "off")]
            {
                let cfg = run(&["--sound", sound, "--game-colours", colours]);
                assert_eq!(cfg.enable_sound, want, "--sound {sound} over a stored {stored}");
                assert_eq!(
                    cfg.honor_game_colours, want,
                    "--game-colours {colours} over a stored {stored}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-1082: `--colour machine` is what `--system-colours` was, and it is pinned
    /// now — the old flag set a PERSISTED key with no record that a one-run source
    /// had done it, so one `--system-colours` launch plus any settings save wrote
    /// `system_colours = true` into the user's file for good.
    #[test]
    fn colour_machine_licenses_the_machine_for_one_run_only() {
        let dir = std::env::temp_dir().join(format!("bm-colourmachine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, "# mine\n").unwrap();
        let flagged =
            |v: Option<ColourSource>| Cli { colour: v, ..cli_with_config(&cfg_path, None) };

        // Absent: the full chain runs, which IS `machine` — minus its opt-in.
        let plain = resolve(&flagged(None));
        assert_eq!(plain.colour_source, ColourSource::Machine, "the chain that already ran");
        assert!(!plain.system_colours, "but nothing licensed a machine nobody named");

        let cfg = resolve(&flagged(Some(ColourSource::Machine)));
        assert!(cfg.system_colours, "--colour machine is the opt-in --system-colours was");
        write_config(&dir, &cfg).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            !toml::from_str::<Config>(&back).unwrap().system_colours,
            "and it is for one run; the FILE must not learn it: {back}"
        );

        // The narrowing arms take no licence with them — they decline the machine.
        for src in [ColourSource::Theme, ColourSource::Terminal] {
            let cfg = resolve(&flagged(Some(src)));
            assert_eq!(cfg.colour_source, src);
            assert!(!cfg.system_colours, "{src:?} asks for a source, not for a machine");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-1154: and the narrowing arms decline a machine the MEDIUM named, too —
    /// which is the half that was missing, since `system_colours` was never what
    /// licensed a floppy in the first place.
    ///
    /// The CI-safe half of `tests/suites/colour_regime_media.rs`: that suite
    /// drives real presses and skips vacuously wherever the gitignored media are
    /// absent, which is every CI run. This one asks the same predicate of a
    /// `ProfileSource` set by hand, so the rule cannot quietly stop being tested
    /// on the machine that runs it most.
    #[test]
    fn the_host_colour_regimes_withhold_the_machine_however_it_was_named() {
        use crate::interpreter::{InterpreterProfile, ProfileSource};
        let cfg = |source, colour| Config {
            interpreter_profile: InterpreterProfile::Amiga,
            interpreter_source: source,
            colour_source: colour,
            system_colours: colour == ColourSource::Machine,
            ..Default::default()
        };
        for source in [ProfileSource::Medium, ProfileSource::Asked, ProfileSource::Fallback] {
            for colour in [ColourSource::Theme, ColourSource::Terminal] {
                let c = cfg(source, colour);
                assert!(!c.machine_colours_licensed(), "{source:?} under {colour:?}");
                assert_eq!(c.machine_default_colours(), None, "so it states no §8.3.3 pair");
                assert_eq!(c.machine_two_colour_colours(), None, "and no two-colour card");
                assert_eq!(
                    c.machine_text_palette(Some(6)),
                    zvm::screen::Palette::Standard,
                    "and its numbers resolve through §8.3.1"
                );
            }
        }
        // …while `machine` is the chain that already ran, medium and opt-in alike.
        let medium = cfg(ProfileSource::Medium, ColourSource::Machine);
        assert!(medium.machine_colours_licensed(), "a floppy is its own licence");
        assert_eq!(medium.machine_text_palette(Some(6)), zvm::screen::Palette::Amiga);
        assert_eq!(medium.machine_default_colours(), InterpreterProfile::Amiga.default_colours());
        let asked = cfg(ProfileSource::Asked, ColourSource::Machine);
        assert!(asked.machine_colours_licensed(), "--interpreter 4 plus the opt-in, SQ-0928");
        assert!(
            !cfg(ProfileSource::Fallback, ColourSource::Machine).machine_colours_licensed(),
            "and nothing rescues a fallback"
        );
    }

    /// SQ-0646: "default" in the settings panel is `None`, and `None` has to REMOVE
    /// the key. Leaving it meant the reset held for exactly as long as the session —
    /// the panel reported success, and the next launch read the old number back.
    #[test]
    fn resetting_interpreter_number_to_default_removes_the_key() {
        let dir = std::env::temp_dir().join(format!("bm-interp-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, "# mine\ninterpreter_number = 4\nvolume = 42\n").unwrap();

        let cli = cli_with_config(&cfg_path, None);
        let mut cfg = resolve(&cli);
        assert_eq!(cfg.interpreter_number, Some(4));

        // The panel cycles left from 1 to "default".
        cfg.set_interpreter_number(None);
        write_config_file(&cfg).unwrap();

        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(!back.contains("interpreter_number"), "the key is gone, not stale: {back}");
        assert!(back.contains("volume = 42"), "the rest of the file is untouched: {back}");
        assert_eq!(resolve(&cli).interpreter_number, None, "and the reset survives a relaunch");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0646: `--interpreter` is sticky for the run, not for the user. Once
    /// the settings panel sets a value the CLI didn't, that value persists — the old
    /// `!from_cli` flag made such a session drop every panel edit on the floor while
    /// telling the user it had saved.
    #[test]
    fn a_panel_edit_beats_the_cli_stickiness() {
        let dir = std::env::temp_dir().join(format!("bm-interp-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, "interpreter_number = 4\n").unwrap();

        // Launched with --interpreter 6.
        let cli = cli_with_config(&cfg_path, Some(6));
        let mut cfg = resolve(&cli);
        assert!(cfg.interpreter_number_from_cli(), "untouched, it is still the CLI's");

        // The panel moves it to 3 — a decision, not a flag.
        cfg.interpreter_number = Some(3);
        assert!(!cfg.interpreter_number_from_cli(), "an edited value is no longer the CLI's");
        write_config_file(&cfg).unwrap();
        assert_eq!(resolve(&cli_with_config(&cfg_path, None)).interpreter_number, Some(3));

        // …and so is picking the CLI's own number deliberately, via the setter.
        let mut cfg = resolve(&cli);
        cfg.set_interpreter_number(Some(6));
        write_config_file(&cfg).unwrap();
        assert_eq!(resolve(&cli_with_config(&cfg_path, None)).interpreter_number, Some(6));

        // Clearing it to default from a CLI session still removes the key.
        let mut cfg = resolve(&cli);
        cfg.set_interpreter_number(None);
        write_config_file(&cfg).unwrap();
        assert_eq!(resolve(&cli_with_config(&cfg_path, None)).interpreter_number, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0645: a file that is valid TOML but invalid *config* (`volume = 300`,
    /// `auto_load = "yes"`) loads as all-defaults with `config_error` set — exactly
    /// like a syntax error — but the write side only ever re-parsed with toml_edit,
    /// which is happy with both. So the next settings save (the aux-storage prompt is
    /// enough) round-tripped the doc and `put` "updated" every key the file already
    /// had to the in-memory DEFAULT: the user's whole config, rewritten to values
    /// they never chose. Same event as SQ-0580, same refusal.
    #[test]
    fn a_save_refuses_to_clobber_a_type_error_config() {
        let dir = std::env::temp_dir().join(format!("bm-typecfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        // Valid TOML throughout; `volume` is out of u8 range and `auto_load` is a
        // string, so `Config` deserialization fails while toml_edit parses it fine.
        let original = "# my carefully commented config\n\
                        volume = 300\n\
                        auto_load = \"yes\"\n\
                        undo_levels = 32\n";
        std::fs::write(&path, original).unwrap();

        let cli = cli_with_config(&path, None);
        let mut cfg = resolve(&cli);
        assert!(cfg.config_error.is_some(), "a type error is a load failure too");
        assert_eq!(cfg.volume, Config::default().volume, "nothing from the file is in memory");

        // A settings save from that state must not write memory back over the file.
        cfg.volume = 33;
        let err = write_config_file(&cfg).expect_err("a config that didn't load is not overwritten");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("could not be loaded"), "and it says why: {err}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "the user's values and comments survive byte for byte",
        );

        // Once the types are right again, saving works normally.
        std::fs::write(&path, "# my carefully commented config\nvolume = 30\nundo_levels = 32\n").unwrap();
        let mut fixed = resolve(&cli);
        fixed.volume = 33;
        write_config_file(&fixed).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("# my carefully commented config"), "comments preserved: {after}");
        assert!(after.contains("volume = 33"), "and the new value landed: {after}");
        assert!(after.contains("undo_levels = 32"), "and the untouched key is kept: {after}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0644: `fs::write` truncates config.toml before it writes a byte, so a crash
    /// (or a full disk) mid-save costs the user every setting AND every comment in it.
    /// The write now builds a temp sibling and renames, which a directory that admits
    /// no new files can prove: the save fails outright instead of half-succeeding.
    #[test]
    fn a_config_save_never_truncates_the_previous_file() {
        let dir = std::env::temp_dir().join(format!("bm-cfg-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let original = "# my carefully commented config\nvolume = 42\n";
        std::fs::write(&path, original).unwrap();

        let mut cfg = resolve(&cli_with_config(&path, None));
        cfg.volume = 33;
        if !crate::storage::deny_new_files_in(&dir) {
            let _ = std::fs::remove_dir_all(&dir);
            return; // platform can't enforce it (or we're root) — skip
        }
        let result = write_config_file(&cfg);
        crate::storage::allow_new_files_in(&dir);

        assert!(result.is_err(), "a write that cannot complete must fail, not half-happen");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "the previous config survives intact",
        );
        assert!(crate::storage::leftover_temps(&dir).is_empty(), "no temp left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn interpreter_number_defaults_none_and_parses_override() {
        // Default and absent key → None (auto).
        assert_eq!(Config::default().interpreter_number, None);
        let back: Config = toml::from_str("").unwrap();
        assert_eq!(back.interpreter_number, None, "absent key keeps None");
        // Explicit override parses.
        let over: Config = toml::from_str("interpreter_number = 6\n").unwrap();
        assert_eq!(over.interpreter_number, Some(6), "explicit override parses");
    }

    #[test]
    fn shipped_keymap_example_parses() {
        let toml = r#"
[keymap]
use_defaults = true
[keymap.map]
"+" = "zoom-map in"
"c" = "center-map"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let (km, warns) = crate::keymap::KeyMap::resolve(&cfg.keymap);
        assert!(warns.is_empty());
        let c: crate::keymap::KeySpec = "c".parse().unwrap();
        assert_eq!(km.lookup(&c, crate::keymap::Context::Map), Some("center-map"));
    }

    #[test]
    fn symbol_config_badge_glyph_defaults() {
        let s = SymbolConfig::default();
        assert_eq!(s.badge_save, "S");
        assert_eq!(s.badge_hint, "H");
        assert_eq!(s.badge_hint_available, "h");
    }

    #[test]
    fn symbol_config_badge_glyph_override_and_absent_default() {
        // Overriding one field parses; the others keep their defaults.
        let toml = r#"
            badge_save = "◆"
        "#;
        let s: SymbolConfig = toml::from_str(toml).unwrap();
        assert_eq!(s.badge_save, "◆");
        assert_eq!(s.badge_hint, "H");
    }

    /// SQ-1160 retired `badge_zcode`/`badge_glulx`/`badge_blorb`. Pre-release
    /// means no shim, but a config still naming one must LOAD: `SymbolConfig`
    /// carries no `deny_unknown_fields`, so serde drops the retired key rather
    /// than failing the file and taking every other symbol down with it.
    #[test]
    fn a_retired_badge_key_is_ignored_not_an_error() {
        let toml = r#"
            badge_zcode = "Z"
            badge_glulx = "G"
            badge_blorb = "B"
            badge_save = "★"
        "#;
        let s: SymbolConfig = toml::from_str(toml).expect("a retired key must not fail the load");
        assert_eq!(s.badge_save, "★", "the surviving key beside it still lands");
    }
}
