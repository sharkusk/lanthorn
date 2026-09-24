//! Boot a story to its first prompt with no terminal (SQ-1537).
//!
//! [`boot_story`] is the per-story build the TUI's `startup::boot_story` used to
//! do inline: mount the story, resolve the machine, the palette and the per-game
//! overrides, construct the engine, load the resume archive, seed the
//! [`AppState`], drain the banner, and observe the starting room. It stops where
//! the terminal begins — the TUI then enters raw mode and builds its `Terminal`
//! around the [`BootedStory`] this returns, and a headless host just keeps it.
//!
//! Three things a terminal supplied inline now cross the boundary as values:
//!
//! - **[`TerminalFacts`]** — the image-protocol picker, the OSC 10/11 default
//!   colours, the late-reply sweep and the terminal size. A headless host leaves
//!   all of them at their defaults, which is what a non-terminal stdout already
//!   produced (no picker, no colours, 80x24 fallback screen).
//! - **[`BootHooks`]** — the lines the TUI prints on the ordinary terminal before
//!   the alternate screen takes it, and the start/end of the engine build (where
//!   the TUI spins its loading indicator). Called in the order the TUI always
//!   printed them.
//! - **[`BootError`]** — an unrecoverable per-story failure, returned rather than
//!   ending the process; the TUI prints it and exits 1 exactly as before.

use std::path::{Path, PathBuf};

use mapper::mapper::Mapper;
use ratatui::layout::Rect;

use crate::archive::load_archive;
use crate::config::{Cli, Config};
use crate::engine::Engine;
use crate::engine_helpers::{restore_error_msg, zvm_session_opt_mut};
use crate::glulx_session::GlulxSession;
use crate::hints;
use crate::ifid::compute_ifid;
use crate::session::{apply_turn, GameSession};
use crate::state::AppState;
use crate::storage::{default_state_path, game_dir as story_game_dir, story_key_for, DiskBuild};

use super::{flush_screen_trace, flush_v6_trace};

/// Everything [`boot_story`] needs to build one story, as one value.
///
/// `cfg` is the pristine LAUNCH config, taken by value because the per-game
/// overlays (garglk.ini colours, per-game honor/borderless, one-run pins) mutate
/// it — each story must start from the launch config, never the last story's.
pub struct BootRequest<'a> {
    /// The story file, disk image or container to open.
    pub story_path: PathBuf,
    /// Which story on a multi-story image the browser row stood for (SQ-0859);
    /// `None` for every loose file and single-story image.
    pub disk_entry: Option<&'a str>,
    /// This launch's own choices (the launch-options dialog, `--pictures`).
    pub overrides: &'a crate::launch_options::LaunchOverrides,
    /// The launch config (see the type's own docs for why it is owned).
    pub cfg: Config,
    /// Where per-story saves and sidecars live: `<data_base>/<story-key>/`.
    pub data_base: PathBuf,
    /// The command-line facts boot reads ([`LaunchFlags::from`] a [`Cli`]).
    pub flags: LaunchFlags,
    /// What the terminal answered, or nothing for a headless host.
    pub terminal: TerminalFacts,
}

/// The command-line facts a story boot reads, lifted off [`Cli`] so a host with
/// no command line can say "none of them" by `LaunchFlags::default()`.
///
/// The `*_named` flags are "was this setting typed on the command line?": a flag
/// is an instruction for the run and outranks the per-game sidecar (SQ-0855,
/// SQ-1079, SQ-1082), so boot only needs to know THAT it was given — `config::
/// resolve` has already folded its value into the config.
#[derive(Debug, Clone, Default)]
pub struct LaunchFlags {
    /// `--debug` (SQ-0449): trace from the first boot instruction.
    pub debug: bool,
    /// `--game-colours on|off`, which outranks both per-game layers (SQ-0855).
    pub game_colours: Option<bool>,
    /// `--colour …` was given (SQ-1082/SQ-1532).
    pub colour_named: bool,
    /// `--v6-pixel-lock …` was given (SQ-1079).
    pub v6_pixel_lock_named: bool,
    /// `--guidance …` was given (SQ-1123).
    pub guidance_named: bool,
    /// `--v6-render …` was given.
    pub v6_render_named: bool,
    /// `--interpreter-version` (SQ-0885), header `$1F`.
    pub interpreter_version: Option<u8>,
    /// `--transcript-file` (SQ-0410).
    pub transcript_file: Option<PathBuf>,
}

impl From<&Cli> for LaunchFlags {
    fn from(cli: &Cli) -> Self {
        LaunchFlags {
            debug: cli.debug,
            game_colours: cli.game_colours.map(bool::from),
            colour_named: cli.colour.is_some(),
            v6_pixel_lock_named: cli.v6_pixel_lock.is_some(),
            guidance_named: cli.guidance.is_some(),
            v6_render_named: cli.v6_render.is_some(),
            interpreter_version: cli.interpreter_version,
            transcript_file: cli.transcript_file.clone(),
        }
    }
}

/// What only a terminal can answer, probed by the host before the boot. The
/// default is the honest answer for a host that is not one.
#[derive(Default)]
pub struct TerminalFacts {
    /// The in-game image-protocol picker (`None` with `--images off`, when the
    /// terminal draws no images, or when there is no terminal).
    pub game_picker: Option<ratatui_image::picker::Picker>,
    /// Whether the picker's query got any answer at all (SQ-1511).
    pub game_picker_query_answered: bool,
    /// The terminal's own default fg/bg (OSC 10/11, SQ-0510).
    pub term_default_colors: crate::term_colors::TermDefaultColors,
    /// The late-reply sweep that probe hands back (SQ-0769).
    pub query_sweep: crate::query_sweep::QuerySweep,
    /// The terminal's size, `(cols, rows)`, from which the story pane a v1–v8
    /// Z-machine story is BOOTED with is derived (SQ-0679/0680). `None` leaves
    /// the constructor's 80x24 fallback — a host that is not a terminal can also
    /// pass the cell grid it intends to draw the whole frame on.
    pub size: Option<(u16, u16)>,
}

/// The per-story result of [`boot_story`]: the running engine, its map, the
/// seeded state, and the paths/identity every later save/restore/reset call
/// threads through.
pub struct BootedStory {
    pub session: Box<dyn Engine>,
    pub mapper: Mapper,
    /// Headless-safe: `AppState` opens no audio device until a sound plays.
    pub state: AppState,
    pub game_dir: PathBuf,
    pub ifid: String,
    /// `<game_dir>/default.lanthorn`, the auto-save / resume archive.
    pub arc_file: PathBuf,
    pub story_bytes: Vec<u8>,
    pub story_path: PathBuf,
    pub data_base: PathBuf,
}

/// An unrecoverable per-story boot failure (unreadable or invalid story, an
/// engine that refused it). The message carries no `lanthorn:` prefix; the TUI
/// adds one and exits 1, as it always did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootError(pub String);

impl std::fmt::Display for BootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BootError {}

/// What the host is told while a boot runs. Every method has a do-nothing
/// default, so a headless host passes [`QuietBoot`].
pub trait BootHooks {
    /// A line the TUI prints on the ordinary terminal before the alternate screen
    /// takes it (it prefixes `lanthorn: `): warnings, the seed line, the resource
    /// summary. The warnings that matter to play are also pushed into the
    /// transcript, so a host that drops these loses nothing a player needs.
    fn console(&mut self, _line: &str) {}
    /// The engine is about to be built — the slow part of a boot, several seconds
    /// for a large Glulx story. `bytes` is the story's size.
    fn engine_starting(&mut self, _story: &Path, _bytes: usize) {}
    /// The engine is built; any indicator started above can stop.
    fn engine_ready(&mut self) {}
}

/// [`BootHooks`] that ignores everything — the headless default.
pub struct QuietBoot;

impl BootHooks for QuietBoot {}

/// Format the startup line naming the PRNG seed this launch handed the engine
/// (SQ-0811). `pinned` is whether it came from the `random_seed` config key.
///
/// The unpinned line says how to keep the run, because a fresh seed is the whole
/// point of the default and a player who has just had a remarkable game has no
/// other way to ask for it again.
pub fn random_seed_line(seed: u32, pinned: bool) -> String {
    if pinned {
        format!("random seed {seed} (pinned by random_seed in config.toml)")
    } else {
        format!("random seed {seed} (set random_seed = {seed} to replay this run)")
    }
}

/// The resource Blorb a Glulx or Scott story's pictures come from, or `None` with
/// `images` off.
///
/// **Through `graphics::resource_blorb`, not `blorb::resolve_resource_blorb`**
/// (SQ-1085), so the two arms resolve from the same tiers. The bare `blorb`
/// call knows the filesystem: a self-blorb, a same-stem sidecar, a directory
/// scan. It does not know about the ZIP a player downloaded the game in — so a
/// zipped `.gblorb` ran with no pictures and no sounds at all, which is the
/// worse half of the same defect, since Glulx is the engine whose games most
/// often ARE one big resource-carrying Blorb.
///
/// Nothing else moves: the extra tier only fires when `story_path` is a zip,
/// and the build-mismatch refusal `graphics::resource_blorb` adds is inert here
/// — it needs a story mounted off a release disk image with an identifiable
/// build, which no Glulx or Scott game is.
pub fn resolve_pict_blorb(story_path: &Path, images: bool) -> Option<blorb::Blorb> {
    if images {
        crate::graphics::resource_blorb(story_path).found.map(|(b, _)| b)
    } else {
        None
    }
}

/// The real story-pane `(rows, cols)` a v1–8 Z-machine session should be BOOTED
/// with — measured BEFORE the engine exists, so a v4/v5 story's boot-time
/// status-bar layout (Zork 1: paints its reverse bar once, at whatever width
/// header byte $21 held at that moment, then only re-cursors to the two field
/// columns it derived from it) already targets the real pane instead of the
/// zvm 80×24 fallback `init_caps` seeds absent a hint (SQ-0679/SQ-0680).
///
/// Runs the SAME split this frame's `compute_pane_layout`/`story_screen_dims`
/// would, against a throwaway [`AppState`] carrying only what those two
/// functions read: the resolved theme (border sides, for the upper-window
/// frame `story_screen_dims` insets), the resolved config (margins, the
/// `virtual_screen_cols`/`rows` pin — pinned wins here exactly as it wins in
/// the live pane measurement, since this reuses the very same call), the
/// garglk.ini margin overlay, and the pane-split sizes. Command panel /
/// inventory panel are left at their true boot-time state — closed; both open
/// only after this session already exists (`initial_panel`, further down) —
/// and neither affects the WIDTH this seeds anyway, only rows, which the
/// SQ-0679 floor never gates.
///
/// `None` when the frame is zero-area; the constructor then falls back to the
/// 80×24 boot default.
// Moved from the binary, where `AppState` is a foreign type and this lint is
// silent; the one-field-at-a-time seeding reads as the list of facts it is.
#[allow(clippy::field_reassign_with_default)]
fn pre_boot_host_screen(
    cfg: &Config,
    cs: &crate::colors::ColorScheme,
    garglk_overlay: &Option<crate::garglk_ini::GarglkOverlay>,
    layout: crate::state::Layout,
    terminal_size: (u16, u16),
) -> Option<(u16, u16)> {
    let mut boot_state = AppState::default();
    boot_state.colors = cs.clone();
    boot_state.config = cfg.clone();
    boot_state.garglk_overlay = garglk_overlay.clone();
    // SQ-1084: the fifth fact, and the one whose absence was invisible. Everything
    // above changes how the pane LOOKS; this changes how WIDE it is, and the width
    // is what the story is told. `compute_pane_layout` splits the frame for a
    // visible map unless the layout says otherwise, so a default-constructed state
    // declared half the terminal to every story whose map the player had hidden —
    // and a game centres on the number it is given, so its title screen came out
    // centred in the left half of a full-width pane. A `Layout` rather than a
    // `bool` because a bare boolean here is the positional fact this file has been
    // bitten by three times (SQ-1022, SQ-1061).
    boot_state.layout = layout;
    boot_state.pane_sizes = crate::state::PaneSizes {
        split_ratio: cfg.split_ratio,
        band_height: cfg.command_band.height,
        inv_dock_pct: cfg.inv_dock_pct,
        room_dock_pct: cfg.room_dock_pct,
    };
    story_screen_in(&boot_state, terminal_size)
}

/// The story pane a `(cols, rows)` terminal gives `state`, in character cells —
/// the `host_screen` a boot is seeded with.
///
/// The launch asks it of a throwaway state before the session exists; a RESTART
/// ([`super::reset::reset_game`]) asks the same question of the real `AppState`
/// (SQ-1061). The TUI passes its live terminal size to both. `None` for a
/// zero-area frame.
pub fn story_screen_in(state: &AppState, (term_cols, term_rows): (u16, u16)) -> Option<(u16, u16)> {
    let frame = Rect::new(0, 0, term_cols, term_rows);
    if frame.width == 0 || frame.height == 0 {
        return None;
    }
    let pane_layout = crate::layout::compute_pane_layout(frame, state, 0);
    crate::render::screen::story_screen_dims(pane_layout.story, state)
}

/// Build the per-story engine + mapper + UI state for `req.story_path`, up to the
/// point where a terminal would be set up (see the module docs).
///
/// Behaviourally the per-story half of the TUI's old `boot()`: load the story,
/// build the engine, load the mapper/archive, seed the state. What the TUI still
/// does after this returns is terminal-only, plus two launch-policy steps that
/// are not a host's business: the keep-it prompt for a story fetched from a URL,
/// and printing the transcript of a story that quit before asking for input.
// See `pre_boot_host_screen` for the lint.
#[allow(clippy::field_reassign_with_default)]
pub fn boot_story(req: BootRequest<'_>, hooks: &mut dyn BootHooks) -> Result<BootedStory, BootError> {
    let BootRequest { story_path, disk_entry, overrides, mut cfg, data_base, flags, terminal } = req;
    let TerminalFacts {
        game_picker,
        game_picker_query_answered,
        term_default_colors,
        query_sweep,
        size: terminal_size,
    } = terminal;

    // `disk_entry` is which story on the image the browser row stood for
    // (SQ-0859) — `None` for every loose file and every single-story floppy, and
    // then this is byte-for-byte the load it always was.
    //
    // `_full` rather than `_from` because a US S.A.G.A. release's pictures are
    // separate files on the same floppy (SQ-1475) and the mount does not
    // outlive this call: they come out with the story or not at all.
    let mounted = match hints::load_mounted_story_full(&story_path, disk_entry) {
        Ok(l) => l,
        Err(e) => return Err(BootError(format!("cannot read '{}': {}", story_path.display(), e))),
    };
    // `saga_pictures` is deliberately not read off the mount here: a Scott
    // story's `ScottPictureSources::resolve` (SQ-1485) re-derives the same
    // set from `story_path`/the story bytes, the one function `reset.rs`'s
    // Scott arm calls too, so both resolve it the same way rather than one
    // reading it off a live mount and the other re-deriving it by hand.
    let hints::MountedStory { story: loaded, disk_image, saga_pictures: _ } = mounted;
    // Raw executable bytes (for the IFID / map-dir key), independent of engine.
    let story_bytes = loaded.bytes().to_vec();
    // Read off `loaded` before it is consumed into a session below: which bundled
    // title table applies is an engine question (SQ-0766).
    let is_scott = matches!(loaded, hints::LoadedStory::Scott(_));

    // Storage (SQ-0284): saves/sidecars live in `<data_base>/<story-key>.save/`,
    // keyed by the story filename — or, for a story mounted out of a disk image,
    // by that story's own release and serial, because one image holds several
    // games and the filename cannot tell them apart (SQ-0850). Both inputs are
    // already in hand from the mount just above, so this costs no second read.
    // The PATH is needed this early because the per-game sidecar inside it
    // carries the `pictures` key, and that key decides the machine below; the
    // directory itself is created (and read from) further down, where it always
    // was.
    let disk_build = disk_image.and_then(|kind| DiskBuild::of(&story_bytes, kind));
    let game_dir = story_game_dir(
        &data_base,
        &story_key_for(crate::storage::StoryOrigin {
            path: &story_path,
            // The zip half of the same fact (SQ-1098): a container's entry is
            // what tells two of its games apart, and a zip has no build to be
            // keyed by, so leaving this out gave both of them one directory.
            entry: disk_entry,
            build: disk_build.as_ref(),
        }),
    );
    // SQ-0734 tier 3: has the user named a picture archive for this story? Read
    // and PARSED here, ahead of everything, because the flavour it turns out to
    // be is an input to the profile immediately below. The archive itself is
    // handed to the `PictSource` further down; nothing reads the file twice.
    //
    // SQ-0789/0791: three doors, one mechanism. `--pictures` and an un-persisted
    // choice from the launch-options dialog arrive as `overrides.pictures` and
    // outrank the sidecar key; parked on `cfg` so a restart re-resolves the same
    // archive instead of quietly reverting to the Blorb.
    cfg.pictures_override = overrides.pictures.clone();
    // SQ-1473: same mechanism, for the Scott C64 vector artwork's resolution —
    // a choice the launch-options dialog made and the player did not persist
    // rides with the story for the session, so `@restart` draws the same
    // resolution rather than quietly reverting to the default.
    cfg.scott_picture_resolution_override = overrides.scott_picture_resolution;
    let picture_override = if cfg.images {
        crate::graphics::PictureOverride::resolve_with_session(
            &story_path,
            &game_dir,
            cfg.pictures_override.as_deref(),
        )
    } else {
        crate::graphics::PictureOverride::Unset
    };
    // A named archive that is absent or will not decode must never pass in
    // silence — the player would believe they were looking at native art and
    // would be looking at the Blorb's. Said here, before the alternate screen is
    // entered, so it survives in the terminal's scrollback; also pushed into the
    // transcript as a warning line further down, where `state` exists.
    //
    // SQ-0866: and neither must a resource Blorb REFUSED for naming a different
    // build. Drawing nothing is the honest outcome, but it is only honest if the
    // player is told why their disk has no pictures — otherwise a silent screen
    // reads as a defect in lanthorn rather than as a Blorb that belongs to
    // another release. Asked only for a story that came off a disk image, which
    // is the only case the refusal can fire in, and only when no named archive
    // has already won; every ordinary boot pays nothing for it.
    //
    // SQ-0882: and only when the medium has no artwork of its own to draw, which
    // is the warrant above taken literally — see `unpaired_art_warning`.
    let picture_warning = picture_override.warning().or_else(|| {
        let unnamed = !matches!(picture_override, crate::graphics::PictureOverride::Loaded { .. });
        (cfg.images && disk_image.is_some() && unnamed)
            .then(|| crate::graphics::unpaired_art_warning(&story_path, disk_entry))
            .flatten()
    });
    if let Some(msg) = &picture_warning {
        hooks.console(&format!("warning: {msg}"));
    }
    // Read off before the archive itself moves into the `PictSource` below: a
    // native archive has no `Reso` chunk, so the standard window its coordinates
    // imply is the only thing standing in for one (SQ-0736).
    let named_art_std_window = picture_override.std_window();

    // SQ-0719: which machine are we presenting ourselves as? Resolved from the
    // launch (an explicit interpreter number, else the flavour of an archive the
    // user named, else the medium the story came out of, else IBM PC — today's
    // behaviour, named) and settled HERE, before the colour scheme resolves
    // below: the profile's palette is what `ColorScheme::terminal_default`'s
    // Standard 2..=9 seed reads, so selecting it after that point would leave the
    // terminal cells on one machine's colours and the v6 pixel path on another's.
    // Re-asserted on every story so a picker→play loop cannot carry one story's
    // machine into the next.
    //
    // SQ-0789: the interpreter number now has two more specific sources than the
    // global config — a value chosen in the launch-options dialog for THIS launch,
    // and one the dialog's checkbox wrote to this game's own sidecar. Most
    // specific first: this launch, then the CLI flag (a deliberate instruction
    // for the run), then the game's sidecar, then the global config.
    //
    // PINNED as one-run as well as set, which is what marks a value as belonging
    // to THIS RUN: `write_config_at` leaves the global config.toml's own key alone
    // while the live value is still the pinned one. Without that, opening the
    // settings screen during a game whose sidecar pins the Amiga would quietly
    // bake 4 into the GLOBAL config and hand every other story the wrong machine.
    if let Some(n) = overrides
        .interpreter_number
        .or_else(|| cfg.interpreter_number_one_run())
        .or_else(|| crate::styles::read_per_game_interpreter_number(&game_dir))
    {
        cfg.interpreter_number = Some(n);
        cfg.one_run.pin(crate::config::keys::INTERPRETER_NUMBER, n);
    }
    // Ride with the story for the session: the restart path re-resolves artwork
    // and has no other way to know which game on the disc this is (SQ-0876).
    cfg.disk_entry = disk_entry.map(str::to_string);
    // SQ-0928: and WHERE the answer came from, which is what decides whether this
    // launch may present the machine's own colours. `IbmPc` is two answers wearing
    // one name — the machine a DOS floppy names, and the thing every story with no
    // medium falls through to — and only the first has a machine to be faithful to.
    (cfg.interpreter_profile, cfg.interpreter_source) =
        crate::interpreter::InterpreterProfile::resolve_with_source(
            &story_path,
            cfg.interpreter_number,
            picture_override.flavour(),
            // The medium THIS story came off, already resolved by the mount above —
            // which on a hybrid disc is not the same as the image's own format
            // (SQ-0876).
            disk_image,
        );
    // SQ-0939: the palette, asked ONCE and asked HERE — before the style is
    // resolved, before the session constructor runs the story, and before the host
    // resolves a single colour.
    //
    // Which table, and why the story's Version is part of the question, lives on
    // `Config::machine_text_palette` — with the licence, because an unlicensed
    // launch resolves through §8.3.1's own table (SQ-0928's rule, and SQ-1154's
    // `--colour theme|terminal`, which withholds the licence on original media).
    // The suites that measure a booted frame call the same function.
    //
    // SQ-1393: a VALUE, carried from here to the two places that must agree — the
    // `MachineBoot` the session is built from (so the VM's own `true_value` for
    // window properties 17/18 and the two-colour card rule resolve through it) and
    // the `ColorScheme` (so the standard-colour seed, the greys, the v6 pixel path
    // and the IBM bold rule do). It used to be a process-wide atomic in `zvm`,
    // which is what made "set it late, or per-path" possible at all.
    //
    // It is refined once more below, where the archive turns out to name a
    // two-colour card — which cannot be known here, because nothing is mounted yet.
    let mut machine_palette = cfg.machine_text_palette(story_bytes.first().copied());
    // SQ-0885: an experiment knob for header `$1F`, carried beside the palette
    // because it is the same kind of fact — a property of the machine this launch
    // presents — and because the session constructor runs the story, so it has to
    // reach the `Machine` before the boot below. Parked on `cfg` so `reset.rs` can
    // re-ask for it on an `@restart`; it is a flag of this run, so nothing else
    // could tell that path about it.
    cfg.interpreter_version = flags.interpreter_version;

    // Booting a large story to its first prompt can take several seconds, and the
    // TUI runs this before the alternate screen is entered — so the normal
    // terminal would otherwise sit frozen. The host decides what to show for it
    // (the TUI spins a small indicator on stderr); `engine_ready` below ends it.
    hooks.engine_starting(&story_path, story_bytes.len());

    // The in-game graphics Picker (None when --images off, unavailable, or the
    // host is not a terminal) and the terminal's own default fg/bg (OSC 10/11,
    // SQ-0510) arrive in `TerminalFacts`, probed by the HOST before this call:
    // they are questions only a terminal can answer, and a headless host leaves
    // them at their defaults. The picker is reused both for the Glulx session's
    // char-cell pixel size and, below, `AppState.game_picker` (the render side
    // already tolerates None).
    let char_px = game_picker
        .as_ref()
        .map(|p| {
            let f = p.font_size();
            (f.width as u32, f.height as u32)
        })
        .unwrap_or((8, 16));
    // SQ-0593: divide out the terminal's scale before the game sees it. A Glk game's
    // graphics-window sizes are pixel constants its author picked against a
    // conventional screen; a cell twice the reference height turns the same request
    // into half the rows, shrinking the game's artwork against unchanged text. See
    // `GlkPixelScale::resolve` for why this keys off the cell size rather than the
    // display's DPI. No-op at `auto` on an unscaled display with a normal font.
    let char_px = cfg.glk_pixel_scale.apply(char_px);
    // Pixel-precise mouse reporting (SQ-0563) is NOT switched on here. The probe
    // works — terminals answer "set" — but the cell size to divide the reported
    // pixels by does not: the Picker's `font_size` above is in logical points,
    // while SGR-Pixels reports DEVICE pixels, so on a 2× display every click came
    // out at twice its true column and row. That broke click-drag selection and
    // made even cell-granular game buttons unhittable. Until the cell size is
    // derived from the same pixel space the mouse reports in, coordinates stay
    // cells and `pixel_mouse::normalise` is a no-op. Leaving the mode UNSET also
    // matters: a terminal left in PixelMode would report pixels that nothing
    // divides. See `pixel_mouse` for the plumbing, which is otherwise complete.

    // Create the per-game dir (its path was resolved at the top of this function)
    // and read the Glk file VFS sidecar BEFORE building the engine, so a Glulx
    // boot that reads or writes a Glk file (e.g. CM's init cache) sees the
    // sidecar in place (SQ-0290).
    let _ = std::fs::create_dir_all(&game_dir);
    let vfs_sidecar = crate::vfs_store::read_vfs(&game_dir);
    crate::trace::hostio(&cfg.user_dir, cfg.trace.hostio,
        format!("vfs_read({} bytes)", vfs_sidecar.len()));

    // Resolve the look from style.toml (the single styling source) BEFORE the
    // engine builds: a Glulx game may probe glk_style_measure for the host's
    // rendered colours during boot (SQ-0315; Kerkerkruip measures its style_User2
    // slot there and branches its whole presentation on the answer, SQ-0803), so
    // the theme pairs must be in the backend first — and the garglk.ini overlay
    // below must land in `cs` before they are derived. `state.colors` is assigned
    // from these below.
    let (style_doc, style_w1) = crate::style::load_style(cfg.style.as_deref(), &cfg.user_dir);
    let (mut cs, set, style_w2) = crate::style::resolve(&style_doc, &cfg.user_dir, machine_palette);
    // SQ-0319: discover a per-game garglk.ini beside the story and overlay its
    // colours onto the resolved theme BEFORE the backend snapshot below, so the
    // imported look is in the backend for glk_style_measure and painted from
    // turn one. The overlay is stashed in `state` further down so the post-IFID
    // reload_style (and any live /reload) re-applies it. `stylehint` gates
    // honor_game_colours, which the engine build below reads. Precedence: global
    // theme < garglk.ini < per-game <game_dir>/style.toml.
    // SQ-0318: the global config default is the honor base; garglk.ini's
    // `stylehint` gate and the user's per-game override layer on top (per-game
    // wins). Capture the base before garglk mutates `cfg` so `reload_style` can
    // recompute the precedence and `auto` can fall back to it.
    let honor_game_colours_base = cfg.honor_game_colours;
    let garglk_overlay = crate::garglk_ini::discover_with_entry(&story_path, disk_entry);
    let garglk_line = garglk_overlay.as_ref().map(|ov| {
        let summary = ov.apply(&mut cs);
        // …unless `--game-colours` was typed on this launch, which outranks both
        // per-game layers for the same reason `--interpreter` outranks the sidecar:
        // a flag is a deliberate instruction for the run, and a file beside the story
        // is not (SQ-0855). In BOTH directions since SQ-1082 — `--game-colours on`
        // is as much an instruction as `off` was.
        if let Some(h) = ov.honor_game_colours.filter(|_| flags.game_colours.is_none()) {
            // A garglk.ini found beside THIS story speaks for this story, so it is
            // pinned as one-run: the global config must not learn it (SQ-0807).
            cfg.honor_game_colours = h;
            cfg.one_run.pin(crate::config::keys::HONOR_GAME_COLOURS, h);
        }
        summary.console_line()
    });
    // SQ-0318: apply the user's persisted per-game honor override (if any) ON TOP
    // of garglk/global, so the engine builds — and turn one renders — with the
    // user's explicit choice in force. The IFID is computed here (from the raw
    // bytes) and reused for the map dir / identity below.
    let ifid = compute_ifid(&story_bytes);
    // SQ-1532: this launch's own choice (the dialog, un-persisted) outranks the
    // sidecar, same as every other launch-options field (`pictures_override`
    // etc. above) — `overrides.honor_game_colours` is `None` whenever the row
    // was untouched OR CLI-locked, so folding it into the SAME `.filter` as the
    // sidecar read below is safe: the two can never disagree about whether
    // `--game-colours` decided this run.
    if let Some(v) = overrides
        .honor_game_colours
        .or_else(|| crate::styles::read_per_game_honor(&game_dir))
        .filter(|_| flags.game_colours.is_none())
    {
        // The sidecar's key is this game's, not the global default's — pinned for
        // the same reason the garglk overlay above is (SQ-0807). `--game-colours`
        // outranks it, as above.
        cfg.honor_game_colours = v;
        cfg.one_run.pin(crate::config::keys::HONOR_GAME_COLOURS, v);
    }
    // SQ-1532: same precedence, for which of the three default-colour sources
    // this launch draws its page/ink from — this launch's own dialog choice,
    // else this game's sidecar, else the global default; `--colour` on this
    // launch outranks both (it already set `cfg.colour_source` in `resolve`,
    // so leaving it untouched here IS "CLI wins"). `system_colours` is set
    // alongside `Machine` exactly as `config::resolve`'s own `--colour machine`
    // arm does: picking Machine here is the SQ-0928 opt-in, for this one game.
    if let Some(v) = overrides
        .colour_source
        .or_else(|| crate::styles::read_per_game_colour_source(&game_dir))
        .filter(|_| !flags.colour_named)
    {
        cfg.colour_source = v;
        if v == crate::config::ColourSource::Machine {
            cfg.system_colours = true;
            cfg.one_run.pin(crate::config::keys::SYSTEM_COLOURS, true);
        }
    }
    // SQ-0341: per-game borderless-windows override (default off → honor the Glk
    // border hint). Applies to Glulx layout from the first relayout at boot.
    // SQ-0344: precedence mirrors honor_game_colours — an explicit per-game
    // `config.toml` value wins, else a discovered garglk.ini's `wborderx`/
    // `wbordery` (0 → borderless), else off.
    let borderless = crate::styles::read_per_game_borderless(&game_dir)
        .or_else(|| garglk_overlay.as_ref().and_then(|o| o.borderless))
        .unwrap_or(false);
    // SQ-0304: per-game map-panel visibility. `Some(false)` → start with the map
    // hidden (captured here before `cfg` is moved into the engine build below).
    let start_map_hidden = crate::styles::read_per_game_show_map(&game_dir) == Some(false);
    // SQ-0945: per-game v6 pixel lock. Which rung of the magnification ladder looks
    // right is a fact about this story's press, so the sidecar wins over the global
    // key — and, exactly like the honor override above, it is PINNED so this one
    // game's choice can never be written back into the user's global config.toml by
    // a later settings-screen save (`OneRunOverrides`). Editing the row itself
    // releases the pin, which is what a deliberate global edit looks like.
    // …and `--v6-pixel-lock` outranks the sidecar, exactly as `--game-colours`
    // outranks the two per-game layers above: a flag is an instruction for the
    // launch you typed it on, a file beside the story is not (SQ-1079).
    let v6_pixel_lock_base = cfg.v6_pixel_lock;
    if let Some(v) =
        crate::styles::read_per_game_v6_pixel_lock(&game_dir).filter(|_| !flags.v6_pixel_lock_named)
    {
        cfg.v6_pixel_lock = v;
        cfg.one_run.pin(crate::config::keys::V6_PIXEL_LOCK, v);
    }
    // SQ-1123: the border controls persist what they switch, so the two switches
    // that were session-only until now arrive with the game as well. Same
    // precedence and the same pin as the pixel lock above — a flag typed on this
    // launch outranks a file, and one game's choice can never be written back
    // into the user's global config.toml by a later settings-screen save.
    let guidance_base = cfg.guidance;
    if let Some(v) = crate::styles::read_per_game_guidance(&game_dir).filter(|_| !flags.guidance_named)
    {
        cfg.guidance = v;
        cfg.one_run.pin(crate::config::keys::GUIDANCE, v);
    }
    // SQ-0785: the return probe is off by default and per-game before it is
    // global, for the reason the pixel lock is — how much silent work a story is
    // worth is a fact about the story.
    let return_probe_base = cfg.return_probe;
    if let Some(v) = crate::styles::read_per_game_return_probe(&game_dir) {
        cfg.return_probe = v;
        cfg.one_run.pin(crate::config::keys::RETURN_PROBE, v);
    }
    let v6_render_base = cfg.v6_render;
    if let Some(m) = crate::styles::read_per_game_v6_render(&game_dir)
        .filter(|_| !flags.v6_render_named)
        .and_then(|t| crate::config::v6_render_from_key(&t))
    {
        cfg.v6_render = m;
        cfg.one_run.pin(crate::config::keys::V6_RENDER, crate::config::v6_render_key(m));
    }
    let theme_colours = crate::glk_backend::theme_style_colours(&cs);
    // ZMSD §8.3.3 (SQ-0532/A-F2): publish OUR default page + ink in header bytes
    // $2C/$2D, as the nearest §8.3.1 standard colour numbers, so a game that asks
    // "what does 'default' look like here?" gets an honest answer instead of a
    // fixed black-on-white. Resolved from the same layering the renderer uses
    // (theme when it supplies both channels concretely, else the OSC 10/11 probe)
    // and passed into the constructor so it is in force BEFORE the game boots.
    // `honor_game_colours = false` means the interpreter declares itself
    // colourless to the story, so the VM's §8.3.2 seed is left alone.
    // SQ-0719: unless the interpreter profile has defaults of its OWN. A machine
    // that claims to be an Amiga should be telling the game the Amiga's default
    // page and ink, not the user's terminal's. `honor_game_colours = false` still
    // wins over both — that declares the interpreter colourless (§8.3.2) and
    // leaves the VM's own black-on-white seed alone.
    // SQ-1082: which of the three sources answers is `--colour`'s to say, and
    // the chain lives in ONE place now — `reset.rs` kept its own copy of it, and
    // a third input would have had to be added to both.
    let mut host_default_colours = crate::colors::host_default_colours(
        &cfg,
        cfg.machine_default_colours(),
        cs.theme.get("transcript").style,
        term_default_colors.fg.map(|c| (c.0[0], c.0[1], c.0[2])),
        term_default_colors.bg.map(|c| (c.0[0], c.0[1], c.0[2])),
        machine_palette,
    );
    // SQ-0679/SQ-0680: the real story-pane `(rows, cols)`, measured before the
    // engine exists, so a v4/v5 story's boot-time status-bar layout already
    // targets it instead of the zvm 80×24 fallback. `None` (size query failed,
    // or a zero-area frame) leaves the constructor's existing fallback in place.
    // The map's visibility is resolved above, and it MUST reach the width the
    // story is told (SQ-1084) — see `pre_boot_host_screen`.
    let boot_layout = if start_map_hidden {
        crate::state::Layout::TranscriptFull
    } else {
        crate::state::Layout::Split
    };
    let host_screen = terminal_size
        .and_then(|size| pre_boot_host_screen(&cfg, &cs, &garglk_overlay, boot_layout, size));

    // SQ-0811: the seed every engine's PRNG starts from, drawn ONCE here and
    // handed to whichever engine builds below, so the console line further down
    // names the seed the story actually ran on. Unset `random_seed` means a fresh
    // draw per launch — without it a game that never calls the seeding opcode
    // replays one identical sequence forever, which for a roguelike is the whole
    // game. Every engine takes it in its CONSTRUCTOR: the boot run happens in
    // there, and a game's initialisation is exactly where the shuffling is done.
    let random_seed = cfg.effective_random_seed();

    // SQ-0860: whether the artwork this launch loaded declared the interpreter
    // colourless, escaped from the Z-code arm below so it can be handed to
    // `AppState`. The force-off there mutates `cfg` before the engine is built,
    // and the post-IFID `reload_style` recomputes the same key from the two
    // per-story files — so the fact has to travel with the state, not just the
    // value. Always `false` for a non-Z-code engine: no Infocom archive is in play.
    let mut artwork_declines_colours = false;
    // SQ-0936: and how dense the artwork it loaded is, escaped the same way. The
    // render's `v6_pixel_lock` ladder is derived from this pair and the screen model
    // does not carry it. `None` here means the uniform rule (a Blorb, or no v6 art
    // at all), which `AppState`'s own default already is.
    let mut launch_art_scale = None;
    // SQ-1009: the release's own typeface, the cell it declares and the pen that
    // draws with it, escaped the same way. Resolved inside the Z-code arm because
    // that is where the medium is known, and needed there too — the DECLARED cell
    // follows the face now, so the boot cannot be assembled without it.
    let mut launch_text_face: Option<crate::native_font::TextFace> = None;
    // The story's Version, for SQ-0873's period look — which belongs only to a
    // v1-v4 story, since colour arrives with v5 and anything shown before it is
    // presentation rather than a fact the story can read. `None` for Glulx and
    // Scott Adams, which have no §11.1.3 machine to have a look of.
    let mut story_zversion: Option<u8> = None;
    // Build the engine: a Z-machine GameSession for Z-code, a GlulxSession for
    // Glulx — both boxed behind the neutral Engine trait. Z-machine-specific
    // setup (screen dims, undo cap) runs in its arm before boxing.
    let mut session: Box<dyn Engine> = match loaded {
        crate::hints::LoadedStory::ZCode(bytes) => {
            // v6 Pict dimension table (Plan 1a, SQ-0186): resolve the story's
            // resource Blorb the same way sound resources are resolved below —
            // a self-blorb, a same-stem sidecar, or (Zork0's actual release
            // layout: `zork0-r393-s890714.z6` beside `Zork0.blb`, a resources-
            // only Blorb with no `Exec` of its own) a dir-scan stem-prefix match
            // — and header-sniff every Pict's size (no full decode). This MUST
            // run before `new_with_trace`: `picture_data` is called during boot,
            // which happens inside `new_with_trace` itself (Phase 0 lesson).
            // SQ-0719: `PictSource::resolve` also covers an Amiga `.adf` the
            // story was mounted out of, whose own `Pic.data` is its artwork.
            // SQ-0734: and the archive named in the per-game sidecar outranks
            // both, which is how a user picks the MCGA, EGA, CGA or Amiga
            // rendition of a game whose Blorb art is already perfectly fine.
            let mut picts = if cfg.images {
                // SQ-0876: and WHICH story on the medium, so a compilation
                // pairs each game with the archive in its own folder instead
                // of handing all six of the Masterpieces CD's graphical games
                // Arthur's plates.
                crate::graphics::PictSource::resolve_with_override(
                    &story_path,
                    picture_override,
                    disk_entry,
                )
            } else {
                crate::graphics::PictSource::new(None)
            };
            // SQ-0887: does this MACHINE show one palette at a time? An archive
            // cannot answer — Shogun's Amiga `Pic.data` and its DOS `.MG1` both
            // give every picture its own colours, and only the Amiga lets the
            // scene's table repaint the border — so the profile answers, here,
            // where the machine is already resolved. Before `all_pict_dims`, for
            // the same reason as the line below it.
            picts.set_screen_palette(
                cfg.interpreter_profile
                    .interpreter_number()
                    .and_then(zvm::interpreter::machine)
                    .is_some_and(|m| m.one_screen_palette),
            );
            // SQ-0816: the player may prefer the archive's own pixels to the
            // fused ones. Before `all_pict_dims`, so nothing is decoded under the
            // wrong answer.
            picts.set_fuse_dither(cfg.fuse_art_dither);
            let picture_dims = picts.all_pict_dims();
            // v6: the Blorb `Reso` standard window (e.g. Zork0 → 320×200) is the
            // game's native ART resolution. `new_with_trace` advertises 2× it —
            // the reference-authentic 640×400 unit screen (SQ-0479) — before boot
            // so windows + hardcoded art align. `None` (no Reso / non-v6) falls
            // back to 320×200 art → 640×400 screen inside.
            // SQ-0719/SQ-0736: a native Amiga `Pic.data` archive has no `Reso`
            // chunk to read — the format has no such concept — so the machine
            // answers instead of the container, and the existing scale rule
            // fires unchanged rather than being special-cased for `.adf`. IBM PC
            // supplies nothing here, so a Blorb (or a Blorb-less scopa) decides
            // exactly as before.
            // SQ-0734: and a named archive answers between the two — after the
            // Blorb (which is not in play at all when an override loaded) and
            // before the machine, because a 320-wide `.MG1` implies the ordinary
            // standard window on a machine, IBM PC, that declares none.
            // SQ-0806: a TWO-COLOUR rendition with no machine behind it cannot
            // give a story the arbitrary colours §8.3 lets it name, so the story
            // is told the interpreter has none — `honor_game_colours` off, which
            // is exactly what that flag already means (§8.3.2, see
            // `loop_tick::poll_zvm_default_colours`).
            //
            // A `.CG1` archive is a STENCIL. On Zork Zero's border: 46,336
            // opaque lit pixels, 17,152 opaque black, and 192,512 TRANSPARENT
            // — its lit state is paint, the face of the pillars, and its
            // transparency is drawn so the ground behind reads as a colour the
            // two-bit artwork never had to store. Zork Zero asks for a white
            // page anyway, because it issues `set_colour(fg=2, bg=9)` for every
            // video card alike (measured identical across `.cg1`, `.eg1` and
            // `.mg1`) and the story file cannot see which archive was loaded.
            // That page paints out both at once.
            //
            // Through the honour flag rather than the interpreter number, which
            // would look like the tidier fix and is not: header `$1E` steers far
            // more of a v6 game than colour, and advertising 1 (DECSystem-20)
            // costs Shogun its entire RIGHT border — measured, ~11,000 opaque
            // pixels gone on `.cg1` and `.eg1` alike.
            //
            // Pinned as one-run so a later settings write cannot bake "never
            // honour game colours" into the global config (SQ-0646's hazard, and
            // the same guard every other one-run source now gets — SQ-0807).
            //
            // SQ-0846: and NOT on a machine whose own screen already IS this
            // two-colour display — a Macintosh, whose interpreter chose its white
            // page and its mono `Pic.data` in one decision.
            //
            // SQ-0956: nor on a DOS press, which SQ-0928 turned into a machine
            // as well. Declining there cost Zork Zero its `color` command, and
            // the ground it fell back to was the host theme's — right on a dark
            // terminal by luck and wrong on a light one. The card states its own
            // screen instead, three lines below.
            //
            // SQ-0860: recorded on `AppState` too (`artwork_declines_colours`),
            // because the post-IFID `reload_style` re-derives this key from the
            // per-story FILES and would otherwise land back on the global base —
            // captured above, BEFORE this ran — undoing both the value and the pin
            // a few lines after they were set.
            if picts.declines_game_colours(cfg.machine_default_colours()) && cfg.honor_game_colours {
                artwork_declines_colours = true;
                cfg.honor_game_colours = false;
                cfg.one_run.pin(crate::config::keys::HONOR_GAME_COLOURS, false);
            }
            // SQ-0956: and where the launch DOES have a machine, the card it is
            // showing is part of it. A `.CG1` is a CGA card in the 640-wide mode,
            // which has two states — black under light grey — and the palette says
            // so: white 9 is EGA entry 7 there, `#AAAAAA`, which is what every lit
            // pixel of `machine-screenshots/dos-zorkzero-cga.png` measures, text
            // and artwork alike. `Palette::IbmCga` carries both halves of that:
            // the table, and the fact that the display has one bit, which
            // `zvm::screen::two_colour_card_request` is the reader of.
            //
            // Set HERE and not beside the palette above, because the archive is
            // what names the card and it was not resolved yet up there — but still
            // before the session constructor, which runs the story to its first
            // prompt and is where the game's own `set_colour` lands.
            if let Some((palette, pair)) = picts.two_colour_card_screen(&cfg) {
                // SQ-1393: the card's table, over the base resolved above. The
                // scheme's eight Z-machine ANSI slots keep the seed the BASE table
                // gave them — they were resolved before anything was mounted, and
                // that is exactly the state this replaces: the global used to be
                // moved here with the seed already taken from the earlier value.
                // What follows the card is everything read through
                // `ColorScheme::machine_palette`: the greys, the v6 pixel path,
                // the IBM bold rule, and the VM's own two-colour-card rule.
                machine_palette = palette;
                cs.machine_palette = palette;
                // …and the pair §8.3.3 reports is the card's, not the machine's:
                // black 2 rather than blue 6, with the ink unmoved at white 9.
                //
                // SQ-1154: unconditionally, now. `--colour theme|terminal` used to
                // decline the card here, one arm below the palette that had already
                // been installed — so the regime reached the reported pair and not
                // the table it is read back through. It is withheld one layer up
                // instead: those two arms are unlicensed, so
                // `two_colour_card_screen` answers `None` and neither line runs.
                // Whatever reaches this scope IS the card, palette and pair
                // together, which is the point of resolving them in one call.
                host_default_colours = Some(pair);
            }
            // SQ-0837/SQ-0838: then the archive the MEDIUM supplied, and only
            // then the machine. The archive comes first because Infocom's own
            // Macintosh interpreter chose its window and its picture file in one
            // decision ("for a small window use mono gfx, for a big window use
            // color gfx"), so a mono `Pic.data` mounted off a Mac volume states
            // the 480×300 std-Mac screen it was drawn for. It cannot disturb any
            // other medium: for an `.adf` the archive and the Amiga profile give
            // the same 320×200, and a story with no native archive falls through
            // to the machine exactly as before.
            // SQ-1022: the four links, the art scale, the interpreter number,
            // the colours and the cell, resolved in ONE place so no other caller
            // has to reproduce the order. `MachineBoot::resolve`'s own docs carry
            // the SQ-0837/SQ-0838 reasoning for why the archive precedes the
            // machine; it used to live here and is now where every caller sees it.
            // SQ-1011/SQ-1009: the typeface the RELEASE shipped on its own medium.
            // Resolved HERE — before the boot, and long before `reload_style` —
            // because the cell now follows the face rather than the other way
            // round: a proportional disk font states its own line height, and the
            // story has to be told that height at construction. Only this scope
            // can ask, since the font lives on the medium and the answer depends on
            // how the profile was decided (a machine asked for by hand has no
            // volume to read).
            // SQ-1037: and the machine's OWN system face, off a boot disk the player
            // keeps under `~/.lanthorn/`. Second rung of one cascade, not a second
            // lookup — the order lives in `native_font::resolve` and nowhere else.
            let user_disks = crate::system_fonts::UserDisks::new(&cfg.system_font_disk);
            let launch_faces = crate::native_font::resolve(&crate::native_font::FaceRequest {
                story_path: &story_path,
                entry: disk_entry,
                profile: cfg.interpreter_profile,
                source: cfg.interpreter_source,
                art_scale: picts.art_scale(),
                disks: Some(&user_disks),
            });
            let boot = crate::machine_boot::MachineBoot::resolve(
                cfg.interpreter_profile,
                &picts,
                named_art_std_window,
                // SQ-0719/SQ-0930 — the configured number wins, and a DOS medium
                // names the IBM PC rather than falling through to zvm's default.
                cfg.advertised_interpreter_number(),
                host_default_colours,
                // SQ-1154: and whether this launch presents its machine at all,
                // which governs the per-machine screen RULES as well as the values
                // above. Under `--colour theme|terminal` it does not, so the
                // Amiga's shared pens and the Macintosh's screen page stay off and
                // the host's own ground is painted un-snapped.
                cfg.machine_colours_licensed(),
                launch_faces,
                // SQ-1393: the machine's own colour table — the base resolved
                // before anything was mounted, refined a few rows up where the
                // archive turned out to be a two-colour card — and the `$1F`
                // override, both of which used to be process-wide statics set
                // before the constructor rather than facts of this boot.
                machine_palette,
                cfg.interpreter_version,
            );
            // SQ-0790: how DENSE that art is, which only a native archive knows.
            // A 320-wide rendition doubles onto the unit screen exactly as a
            // Blorb's does; an EGA/CGA one is 640 wide with half-width pixels and
            // arrives at (1, 2). `None` for every Blorb-sourced story, which is
            // the uniform rule untouched.
            launch_art_scale = boot.art_scale;
            // The cell, the face and the pen, as the one value the renderer takes.
            launch_text_face = Some(boot.text_face());

            // `--debug` (SQ-0449): trace from the first boot instruction so the
            // game's initialisation code is captured (a later `/debug` can't).
            // SQ-0719: the configured number still wins; absent one, the profile
            // names its machine, and IBM PC names nothing so zvm's own default
            // rule (Frotz's: 6 for v6, 1 otherwise) stays in force untouched.
            // SQ-0930: …except when a DOS MEDIUM named the IBM PC, where deferring
            // to that rule told the story it was a DECSystem-20 off the one disk
            // that says otherwise. See `Config::advertised_interpreter_number`.
            let mut s = match GameSession::new_for_machine(bytes, cfg.honor_game_colours, cfg.enable_sound, flags.debug, picture_dims, host_screen, Some(random_seed), &boot) {
                Ok(s) => s,
                Err(e) => {
                    use zvm::error::ZError;
                    let msg = match e {
                        ZError::UnsupportedVersion(v) => format!("unsupported Z-machine version {v}"),
                        ZError::NotAStoryFile => "file is not a valid Z-machine story file".to_string(),
                        ZError::Truncated => "story file is truncated".to_string(),
                        _ => format!("{e:?}"),
                    };
                    return Err(BootError(msg));
                }
            };
            // v6 Pict source (Plan 1b Task 2, SQ-0186): retained on the session
            // (not `AppState`) so `drain_turn` can rasterize `draw_picture`/
            // `erase_picture` events into `pictures_canvas` self-contained —
            // Plan 1a's dimension table above is a separate, boot-time-only use.
            s.set_pict_source(Some(picts));
            // v6 boot-picture flush (Plan 1b Task 5): a v6 game draws its opening
            // art during boot, inside `new_with_trace` above, before the Pict
            // source existed to rasterize it — drain that backlog once now so the
            // very first `screen()` (before the player's first turn) already
            // shows the boot graphics instead of a blank window.
            s.flush_boot_pictures();
            // Pinned virtual screen dimensions, if the user set either key.
            // `pre_boot_host_screen` above already resolves this pin (it's what
            // `story_screen_dims` reads first) and passed it into the constructor,
            // so a v4/v5 story that lays its status bar out at boot already saw the
            // pinned width. This re-write is now a safety net only — it still fires
            // when `host_screen` came back `None` (no terminal to query), and is a
            // harmless no-op re-write of the same value otherwise. An UNSET key is
            // left for the story pane's real measurement to keep following at every
            // later frame (`poll_zvm_resize`, ZMSD §8.4 — SQ-0532/A-F1).
            // v6 stories run at their NATIVE picture resolution (advertised before
            // boot in new_with_trace); the virtual screen is a v1–5 concern, so
            // leave the v6 native dims untouched here (SQ-0186).
            if s.machine.mem.version() != 6
                && (cfg.virtual_screen_rows.is_some() || cfg.virtual_screen_cols.is_some())
            {
                let rows = cfg.virtual_screen_rows.unwrap_or(s.machine.mem.read_byte(0x20) as u16);
                let cols = cfg.virtual_screen_cols.unwrap_or(s.machine.mem.read_byte(0x21) as u16);
                let cell = s.machine.v6_cell();
                zvm::screen::write_screen_dims(
                    &mut s.machine.mem,
                    rows.clamp(1, 255) as u8,
                    cols.clamp(1, 255) as u8,
                    cell,
                );
            }
            s.machine.undo_cap = cfg.undo_levels;
            story_zversion = Some(s.machine.mem.version());
            Box::new(s)
        }
        crate::hints::LoadedStory::Glulx(bytes) => {
            let pict_blorb = resolve_pict_blorb(&story_path, cfg.images);
            match GlulxSession::new_in(
                game_dir.clone(),
                bytes,
                cfg.virtual_screen_cols.unwrap_or(crate::config::FALLBACK_SCREEN_COLS) as u32,
                cfg.virtual_screen_rows.unwrap_or(crate::config::FALLBACK_SCREEN_ROWS) as u32,
                cfg.acceleration,
                cfg.images,
                cfg.enable_sound,
                borderless,
                char_px,
                pict_blorb,
                &vfs_sidecar,
                theme_colours,
                // `--debug` (SQ-0465): trace from the first boot instruction so the
                // game's initialisation code is captured (a later `/debug` can't).
                flags.debug,
                Some(random_seed),
            ) {
                Ok(s) => Box::new(s),
                Err(e) => return Err(BootError(format!("cannot load Glulx story: {e:?}"))),
            }
        }
        crate::hints::LoadedStory::Scott(bytes) => {
            // The four picture facts, resolved the one way both a launch and
            // an `@restart` resolve them (SQ-1485, `reset.rs`'s Scott arm is
            // the other caller).
            let pictures = crate::graphics::ScottPictureSources::resolve(
                &story_path,
                &bytes,
                &game_dir,
                resolve_pict_blorb(&story_path, cfg.images),
                game_picker.as_ref(),
                cfg.scott_picture_resolution_override,
            );
            match crate::scott_session::ScottSession::new_with_options(
                bytes,
                // `--debug` (SQ-0449/SQ-0464): trace from boot so the opening
                // occurrence pass (run inside the VM constructor) is captured.
                flags.debug,
                Some(random_seed),
                // ScottFree's `-y`/`-s`/`-t`/`-p` options, this story's own
                // per-game choice (SQ-1413).
                crate::scott_session::resolve_options(&game_dir),
                pictures,
            ) {
                Ok(s) => Box::new(s),
                Err(e) => return Err(BootError(format!("cannot load Scott Adams story: {e}"))),
            }
        }
    };
    // Strip the game's own inline read prompt only when the dedicated command
    // bar is on (SQ-0264); otherwise inline-prompt mode keeps the game's ">".
    session.set_strip_prompt(cfg.command_bar);

    // `--debug` (SQ-0449): tracing is already on from the first boot instruction
    // (the Z-machine arm used `GameSession::new_with_trace` above), so the boot
    // PCs are already in the cumulative set. Here we just seed prior runs' coverage
    // from the per-story sidecar so those lines colour immediately too.
    if flags.debug {
        let loaded = crate::pcset_store::read_pcs(&game_dir);
        if !loaded.is_empty() {
            session.seed_executed_pcs(&loaded);
        }
    }

    // Engine is up — the host's loading indicator can stop.
    hooks.engine_ready();

    // SQ-0319: announce the imported garglk config (after the spinner erased its
    // line, so the message isn't clobbered). Printed only when a sidecar applied.
    if let Some(line) = &garglk_line {
        hooks.console(line);
    }

    // SQ-0811: name the seed the story just booted on. A run that turns out
    // interesting is only replayable if the player can find out what it was
    // seeded with, and this is the last moment before the alternate screen takes
    // the terminal — so it stays in the scrollback afterwards, like the warnings
    // above it. Said on every launch, because the interesting run is never the
    // one you thought to ask about beforehand.
    hooks.console(&random_seed_line(random_seed, cfg.random_seed.is_some()));

    // ── 2. IFID + map dir + load/create mapper ────────────────────────────────

    // `ifid` was computed above (before the engine build) so the per-game honor
    // override could feed the engine; `game_dir` (per-story storage) was computed
    // and created before the engine build too. The IFID stays for title/hint/
    // display and the per-game style reload below.
    let arc_file = default_state_path(&game_dir);

    // Load mapper (and optionally restore the game save) from the archive.
    let mut startup_transcript: crate::state::LoadedTranscript = None;
    // Rewind/replay history carried from the archive when the game is auto-restored.
    let mut startup_history: Vec<std::sync::Arc<crate::history::TurnRecord>> = Vec::new();
    // Command history (Up/Down recall) carried from the archive, always loaded.
    let mut startup_command_history: Vec<String> = Vec::new();
    // Turn counter carried from the archive when the game is auto-restored, so a
    // later save records the cumulative count rather than only post-resume moves.
    let mut startup_turns: Option<u32> = None;
    // What the just-restored archive is missing because it predates the screen
    // (SQ-1401) or paint-log (SQ-1403) format bump — SQ-1410. Computed here
    // (while `ac` still exists) and applied to the transcript once `state`
    // exists, below.
    let mut startup_restore_degradation: Option<crate::archive::RestoreDegradation> = None;
    // When auto_load is false but a save exists and prompt_load_on_launch is true,
    // stash the save for the launch dialog instead of discarding it.
    let mut pending_resume_stash: crate::state::PendingResume = None;
    let mut mapper = if arc_file.exists() {
        match load_archive(&arc_file) {
            Ok(ac) => {
                // Restore the machine from the saved game state only when auto_load is enabled.
                if cfg.auto_load {
                    match session.restore_state(&ac.engine_save()) {
                        Ok(()) => {
                            if let Some(scr) = ac.screen.clone() {
                                if let Some(zs) = zvm_session_opt_mut(&mut *session) {
                                    crate::session::restore_screen(zs, scr);
                                }
                            }
                            // The v6 screen: display list where the archive has one
                            // (SQ-0588), else canvas PNGs. No-op for non-v6 archives.
                            crate::engine_helpers::apply_v6_pictures(&mut *session, &ac);
                            // Hand Glulx back the room it was saved in (SQ-0523);
                            // no-op for zvm.
                            crate::engine_helpers::seed_resumed_location(&mut *session, &ac.meta);
                            startup_restore_degradation = Some(crate::archive::RestoreDegradation::from_format_version(
                                ac.meta.format_version,
                                crate::engine_helpers::is_v6_session(&*session),
                            ));
                            startup_transcript = Some((ac.transcript, ac.transcript_kinds, ac.transcript_runs, ac.transcript_para, ac.transcript_images));
                            startup_history = ac.history;
                            // Restore the turn counter from the same archive (SQ-0429):
                            // the auto_load resume path mirrors the interactive restore,
                            // which sets state.turns = ac.meta.turns. Without this, a
                            // resumed game's later save records only post-resume moves.
                            startup_turns = Some(ac.meta.turns);
                        }
                        Err(e) => {
                            hooks.console(&format!("warning: could not restore game from archive: {}; starting fresh", restore_error_msg(e)));
                        }
                    }
                } else if cfg.prompt_load_on_launch && !ac.save.is_empty() {
                    pending_resume_stash = Some((ac.engine_save(), ac.transcript, ac.transcript_kinds, ac.screen));
                }
                if cfg.aux_storage != crate::config::AuxStorage::Global {
                    session.set_aux_data(ac.aux.clone());
                }
                startup_command_history = ac.command_history;
                // The map is part of the game's state: it loads only when the state is
                // auto-resumed here. When auto_load is off it either rides the launch-resume
                // dialog (adopted on accept, see apply_launch_resume) or stays blank.
                if cfg.auto_load { ac.mapper } else { Mapper::default() }
            }
            Err(e) => {
                hooks.console(&format!("warning: could not load archive {}: {}", arc_file.display(), e));
                Mapper::default()
            }
        }
    } else {
        Mapper::default()
    };

    // Startup: pre-load the per-game aux table from the global file when in
    // global mode.  In archive mode the table was populated above from the
    // loaded archive (if any).
    if cfg.aux_storage == crate::config::AuxStorage::Global {
        session.set_aux_data(crate::aux_store::read_global_aux(&game_dir));
    }

    // …and tell the engine where the Z-machine's own stream files live:
    // `<game_dir>/script.txt` (output stream 2, ZMSD §7.1.1) and
    // `<game_dir>/commands.txt` (output stream 4 and input stream 1, §7.1.2 and
    // §10.2), beside `default.aux` and for the same reason. Naming the directory
    // opens nothing — the files appear only if the story, or `/set-transcript`,
    // actually selects a stream.
    session.set_stream_files(&game_dir);

    // The per-story Glk file VFS sidecar was loaded into the VM before boot
    // (GlulxSession::new). A Glulx game may write a Glk file during boot (e.g.
    // CM's init cache); flush it now so it persists before the first turn and
    // survives an immediate quit (SQ-0290). For a Z-machine session vfs_dirty()
    // is always false, so this is a no-op there.
    if session.vfs_dirty() {
        let _ = crate::vfs_store::write_vfs(&game_dir, &session.vfs_bytes());
        session.clear_vfs_dirty();
    }

    // ── 3. Seed initial transcript + starting room ────────────────────────────

    let mut state = AppState::default();
    // Apply the look resolved from style.toml above (before the engine build).
    state.colors = cs;
    state.symbols = set;
    // Stash the garglk.ini overlay (already folded into `cs` above) so the
    // post-IFID reload_style below — and every later /reload — re-applies it.
    state.garglk_overlay = garglk_overlay;
    for w in style_w1.into_iter().chain(style_w2) {
        state.push_notice(&format!("[{}]", w));
    }
    let (keymap, keymap_warnings) = crate::keymap::KeyMap::resolve(&cfg.keymap);
    state.keymap = keymap;
    // Surface any keymap conflict warnings once in the transcript.
    for w in keymap_warnings {
        state.push_notice(&format!("[{}]", w));
    }
    let (hotkeys, hotkey_warnings) = crate::keymap::HotkeyLayout::resolve(&cfg.hotkeys);
    state.hotkeys = hotkeys;
    for w in hotkey_warnings {
        state.push_notice(&format!("[{}]", w));
    }
    state.show_room_numbers = cfg.show_room_numbers;
    state.show_status_bar = cfg.show_status_bar;
    state.game_picker = game_picker;
    state.game_picker_query_answered = game_picker_query_answered;
    state.term_default_colors = term_default_colors;
    state.query_sweep = query_sweep;
    state.pane_sizes = crate::state::PaneSizes {
        split_ratio: cfg.split_ratio,
        band_height: cfg.command_band.height,
        inv_dock_pct: cfg.inv_dock_pct,
        room_dock_pct: cfg.room_dock_pct,
    };
    // `[command_panel] auto_open` — open the command panel with the story, for
    // players who want it as their default input surface rather than a thing to
    // summon. SQ-1123: whether a panel opens with this story is the border
    // control's own state, so a per-game answer wins over the global
    // `[command_panel] auto_open` — absent key = inherit, as every sidecar key
    // does. SQ-1237 widened the per-game key to a three-state cycle (the
    // inventory panel has no global auto-open of its own, so the fallback for
    // an absent key is still just command-or-none).
    let initial_panel = crate::styles::read_per_game_panel(&game_dir).unwrap_or(
        if cfg.command_band.auto_open {
            crate::state::SidePanel::Command
        } else {
            crate::state::SidePanel::None
        },
    );
    // SQ-0318: remember the global honor base so reload_style can recompute the
    // per-game > garglk > global precedence (and `auto` can fall back here).
    state.honor_game_colours_base = honor_game_colours_base;
    // SQ-0945: and the global v6 pixel-lock default, so `set-v6-pixel-lock auto` can
    // put the live key back to it after clearing this game's sidecar override.
    state.v6_pixel_lock_base = v6_pixel_lock_base;
    state.guidance_base = guidance_base;
    state.return_probe_base = return_probe_base;
    state.v6_render_base = v6_render_base;
    // SQ-0855: and whether a flag put it there, which the base alone cannot say —
    // the post-IFID `reload_style` below re-reads both per-story sources from disk
    // and would otherwise let either of them overrule the flag.
    state.game_colours_cli = flags.game_colours;
    // SQ-0860: and whether the artwork declared the interpreter colourless, for the
    // same reason — the reload below re-reads the per-story files, and neither of
    // them knows what archive was loaded.
    state.artwork_declines_colours = artwork_declines_colours;
    // SQ-0936: and the density of the artwork it mounted, which the v6 render's
    // magnification ladder is derived from. An archive that declares no picture
    // space keeps the uniform `V6_ART_SCALE`, which is the field's default.
    if let Some(scale) = launch_art_scale {
        state.v6_art_scale = scale;
    }
    // SQ-1009: and the face that art scale is drawn at, with the cell it declares.
    // Set BEFORE the `reload_style` below, which recomputes the cell and would
    // otherwise put the machine table's back over the face's.
    if let Some(face) = launch_text_face {
        state.v6_text = face;
    }
    // SQ-0873: and the story's Version, which `reload_style` needs to decide
    // whether this launch gets its machine's period look. Same reason as the two
    // above — the reload re-derives from the config and the per-story files, and
    // neither of them knows what engine was built or what it opened.
    state.story_zversion = story_zversion;
    state.config = cfg;

    // `--transcript-file` (SQ-0410): open before anything below pushes to the
    // transcript, so the opening banner a few lines down lands in the file too.
    if let Some(path) = &flags.transcript_file {
        state.attach_transcript_sink(path);
    }

    // Debug trace (trace feature): start a fresh log for this run and arm the
    // engine's screen-trace buffer per config; no-op when no section is active.
    if state.config.trace.any() {
        crate::trace::truncate(&state.config.user_dir);
    }
    session.set_trace_screen(state.config.trace.screen);

    // Resolve the sound container + construct the audio backend (silent if the
    // feature is off, there is no device, or sound is disabled in config).
    // The load line prints here, before the alternate screen is entered, so it
    // stays in the normal terminal scrollback for verification after exit.
    // Through `graphics::resource_blorb`, not `blorb::resolve_resource_blorb`:
    // the `IFhd` game identifier describes the CONTAINER, not its `Pict` chunks,
    // and a Blorb built for another build numbers its sounds exactly as
    // build-specifically as it numbers its pictures. Refusing it for one and
    // trusting it for the other would also have said so on screen — a release
    // whose artwork was just refused went on to print "loaded resources from
    // Shogun.blb (sidecar) (0 sounds, 48 images)" one line later (SQ-0867).
    //
    // Inert on today's corpus, and deliberately so: no refused Blorb in it holds
    // a single `Snd `, and the one real sound-path mismatch — `Lurking.blb`,
    // release 221 / serial 870918 against a release 219 / serial 870912 story —
    // sits beside a LOOSE story and is exempt under the rule's second arm, which
    // is where a person's own filing is allowed to answer the question.
    // SQ-0907: sounds the story's own medium carries, for the two Infocom games that
    // use them off a release disk. Read once, here, because a sound has to start on
    // the turn the game asks for it.
    state.disk_sounds = crate::native_sound::from_medium(&story_path);
    if !state.disk_sounds.is_empty() {
        let mut effects: Vec<u16> = state.disk_sounds.keys().copied().collect();
        effects.sort_unstable();
        hooks.console(&format!(
            "{} sound effect{} on the medium ({})",
            effects.len(),
            if effects.len() == 1 { "" } else { "s" },
            effects.iter().map(u16::to_string).collect::<Vec<_>>().join(", "),
        ));
    }
    state.sound_blorb = match crate::graphics::resource_blorb(&story_path).found {
        Some((b, path)) => {
            let count = |usage: &[u8; 4]| b.resources().iter().filter(|r| &r.usage == usage).count();
            let (sounds, images) = (count(b"Snd "), count(b"Pict"));
            let own = path == story_path;
            hooks.console(&format!(
                "loaded resources from {}{} ({} sound{}, {} image{})",
                path.display(),
                if own { " (self)" } else { " (sidecar)" },
                sounds, if sounds == 1 { "" } else { "s" },
                images, if images == 1 { "" } else { "s" },
            ));
            Some(b)
        }
        None => None,
    };
    // `state.audio` stays `None` here and opens lazily on first actual use
    // (`AppState::play_turn_sounds` / `play_glulx_sound_ops`, or the
    // `/play-sound` diagnostic) — opening a real output device costs real
    // time (~240ms measured, SQ-1014's audit) and a story that never plays a
    // sound should never pay it just because `enable_sound` is on (SQ-1423).

    // Seed autocomplete with the story's parser vocabulary (room nouns are added live).
    state.dict_words = session.introspect().map(|i| i.vocabulary()).unwrap_or_default();

    // Open whichever panel this story starts with (SQ-1123, widened to a
    // three-state cycle by SQ-1237): the per-game override, or the global
    // `[command_panel] auto_open` fallback resolved into `initial_panel` above.
    // Instant (no slide) so the first frame is already the settled layout.
    match initial_panel {
        crate::state::SidePanel::Command => {
            let mut mapper_noop = mapper::mapper::Mapper::default();
            // Not through `Action::OpenCommandBand`: that action PERSISTS the
            // panel state per-game (SQ-1123), and a global `auto_open` must not
            // pin itself to whichever story you happened to launch. The state
            // change without the persistence is exactly what this helper is.
            crate::input::open_command_band(&mut state, &mut mapper_noop, true);
            state.band_dock.toggle_to(true, true);
        }
        crate::state::SidePanel::Inventory => {
            // Same non-persisting rule as the command panel above.
            crate::input::open_inventory_panel(&mut state, true);
            state.inv_dock.toggle_to(true, true);
        }
        crate::state::SidePanel::None => {}
    }

    // Push the game's opening banner and capture the title from it. Glulx returns
    // ordered elements (text + any startup/cover images); the Z-machine returns
    // empty here and falls back to the flat string path. Either way `banner` is the
    // banner text for title extraction (the elems' concatenated Text equals it).
    let banner_elems = session.take_transcript_elems();
    let banner: String = if banner_elems.is_empty() {
        session.take_transcript()
    } else {
        banner_elems
            .iter()
            .filter_map(|e| match e {
                crate::session::TranscriptElem::Text { text, .. } => Some(text.as_str()),
                crate::session::TranscriptElem::Image(_)
                | crate::session::TranscriptElem::ScreenClear => None,
            })
            .collect()
    };
    let banner_title = crate::session::title_from_banner(&banner);
    // SQ-0766: ask the story browser's own metadata resolver first — the `IFmd`
    // chunk, the fetched IFDB sidecar, then the bundled tables — so the pane
    // names the game the way the list does. The banner heuristic is the tier
    // below it, and the filename stem is the last resort it was meant to be.
    let meta_title =
        crate::picker::metadata_title_in(&story_path, &game_dir, &ifid, is_scott, &story_bytes);
    state.title =
        crate::session::resolve_title(None, meta_title.as_deref(), banner_title.as_deref(), &story_path);
    let story_filename = story_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    state.pane_title =
        crate::session::format_pane_title(&state.title, story_filename, disk_image.is_some());
    state.ifid = ifid.clone();
    state.game_dir = game_dir.clone();
    // Restore the per-game map-panel visibility (SQ-0304): if the user last hid
    // the map for this story, start with it hidden.
    if start_map_hidden {
        state.layout = crate::state::Layout::TranscriptFull;
    }
    // Now that game_dir is set, re-resolve through reload_style so the per-game
    // override (<game_dir>/style.toml) is merged over the global at startup — the
    // initial resolve above is global-only (game_dir wasn't set yet). On a per-game
    // parse error the global look already set above stands.
    let _ = crate::reload::reload_style(&mut state);
    if banner_elems.is_empty() {
        state.push_transcript(&banner);
    } else {
        crate::state::apply_transcript_elems(&mut state, &banner_elems);
    }

    // The opening room description is already on screen, so its words are already
    // completable — waiting for the first turn would leave Tab with only the flat
    // dictionary for exactly the move a player is most likely to want help with
    // (SQ-1116).
    crate::input::refresh_seen_words(&mut state, &*session);
    crate::input::refresh_scope_words(&mut state, &*session);

    // A config.toml that doesn't load — bad syntax or a value of the wrong type — is
    // ignored WHOLESALE: TOML is one document, so a single stray character costs every
    // setting in the file. Say so, with the error TOML reported, rather than letting
    // the user wonder why their config has no effect (SQ-0580, SQ-0645). Saving is
    // refused while it's broken, so nothing overwrites it.
    if let Some(err) = state.config.config_error.clone() {
        let msg = format!(
            "{} could not be loaded ({err}) — running on defaults, and settings will \
             not be saved until it is fixed",
            state.config.config_file.display(),
        );
        state.push_transcript_internal(&msg, crate::state::TranscriptKind::Warning);
    }

    // SQ-0734: the per-game `pictures` key named an archive that is missing or
    // will not decode. Surfaced the same way a broken config.toml is — a warning
    // line in the transcript, which stays put instead of expiring like a toast —
    // because the alternative is a player who thinks they are seeing the native
    // art they asked for and is quietly seeing the Blorb's instead.
    if let Some(msg) = picture_warning {
        state.push_transcript_internal(&msg, crate::state::TranscriptKind::Warning);
    }

    // SQ-0663: the theme just rebuilt by reload_style above may carry its own
    // non-fatal diagnostics (a `parent` re-root naming an unknown selector, or
    // a cycle — see `Theme::warnings`), which used to fall back to registry
    // defaults with no visible sign anything was wrong. Surface them the same
    // way the broken-config.toml case just above is surfaced: one transcript
    // Warning line per issue (collapsed to a summary beyond a few — see
    // `describe_theme_warnings`), so a typo like `parent = "acent"` in
    // style.toml no longer degrades silently.
    for line in crate::theme::resolve::describe_theme_warnings(state.colors.theme.warnings()) {
        state.push_transcript_internal(&line, crate::state::TranscriptKind::Warning);
    }

    // One-time notice: config.toml no longer carries style — those moved to style.toml.
    // `config_file` IS `config_path(&cli)` — `config::resolve` sets it from that
    // very call — so a host with no command line reads the same file.
    if let Ok(raw_cfg) = std::fs::read_to_string(&state.config.config_file) {
        if crate::config::config_has_style_sections(&raw_cfg) {
            state.push_transcript_internal(
                "config.toml [colors]/[symbols] are no longer used — styling lives in style.toml ([colors] there; map glyph presets are now [map] keys)",
                crate::state::TranscriptKind::Warning,
            );
        }
    }

    // Observe the starting room so it appears on the map immediately.
    //
    // Built by [`Engine::seed_turn`] and NOT by a `TurnResult { … }` literal: the
    // literal that used to stand here spelled `erase_lower: false` into itself, so
    // an `erase_window` the game issued during its own boot was never drained and
    // the first real turn took it instead — wiping the banner and the opening room
    // description one command late (SQ-1106). Taken UNCONDITIONALLY, before the
    // location test below, because a story whose starting room is undetectable still
    // has a boot to drain.
    let seed_result = session.seed_turn();
    if let Some(snap_number) = seed_result.location.as_ref().map(|snap| snap.number) {
        apply_turn(&mut mapper, "", &seed_result, &mut state.death_watch);
        flush_screen_trace(&state.config.user_dir, &mut *session, state.config.trace.screen);
        flush_v6_trace(&state.config.user_dir, &mut *session, state.config.trace.v6);
        if state.config.trace.any() {
            let ptr = format!(
                "[trace → {}: {}]",
                state.config.user_dir.join("trace.log").display(),
                state.config.trace.active_list(),
            );
            state.push_transcript_internal(&ptr, crate::state::TranscriptKind::Meta);
        }
        let rid = snap_number as mapper::graph::RoomId;
        state.select_room(Some(rid));
        // Recenter using a default pane size; will be corrected after first draw.
        state.recenter_on(
            mapper
                .graph
                .room(rid)
                .and_then(|r| r.pos)
                .unwrap_or((0, 0)),
            40,
            24,
        );
    }

    // [more] pager for the OPENING BANNER (SQ-0532 wave-5). The banner is one
    // batch of game output exactly like a turn's, and a v6 story box is small —
    // Zork Zero's window 0 holds 20 rows and its prologue wraps to 23, so the
    // view pinned to the newest rows and the illuminated drop-cap that opens the
    // game scrolled off before it was ever seen. Arm the pager the way
    // `finish_command_turn` arms it for a turn: the first frame measures the rows
    // the banner actually produced and engages ONLY if it overflowed the story
    // viewport, parking the view on the first screenful. Same rule as a turn
    // (SQ-0539): a boot that ends on a `read_char` — a splash "press any key", a
    // startup menu — pages too, and the paging keys are swallowed by the pager
    // until the view catches up rather than answering that read. Skipped entirely
    // for a resumed transcript (below): that scrollback was already read, and
    // paging it would park a returning player mid-history.
    // The baseline is the first row of the banner that carries PROSE, not row 0:
    // a story that opens with a few newlines (every Inform 7 Glulx one does) had
    // those blank rows counted as text the reader must not miss, which paged a
    // banner that fit and ate the first keystroke of the first command (SQ-1434).
    if startup_transcript.is_none()
        && crate::pager::should_arm(session.pending_input(), crate::pager::more_suppressed(&*session))
    {
        state.pager.arm(crate::pager::opening_baseline(&state));
    }

    // If an archived transcript was loaded on startup, replace the fresh one.
    if let Some((lines, kinds, runs, para, images)) = startup_transcript {
        state.transcript = lines;
        state.clear_anchor = None;
        state.transcript_kinds = kinds;
        state.transcript_runs = runs;
        state.transcript_para = para;
        state.reset_transcript_sidecars();
        // Re-attach inline images after the sidecar reset so an auto-resumed
        // transcript renders its embedded art (SQ-0518).
        state.transcript_images = images;
        // The word scrape above ran against the FRESH boot transcript this one
        // just replaced; the sidecar reset dropped it, so rebuild it from the
        // resumed scrollback (SQ-1135).
        crate::input::refresh_seen_words(&mut state, &*session);
    }
    // After the resumed transcript above, not before: this line must survive
    // as the last one on screen, not be overwritten by `state.transcript = lines`.
    if let Some(degradation) = startup_restore_degradation {
        crate::engine_helpers::push_restore_degradation_notice(&mut state, degradation);
    }
    if !startup_history.is_empty() {
        state.history = startup_history;
    }
    state.command_history = startup_command_history;
    if let Some(turns) = startup_turns {
        state.turns = turns;
    }

    // If a save was found but auto_load is off and prompt_load_on_launch is on,
    // open the launch dialog so the user can choose to resume or start fresh.
    if let Some(stash) = pending_resume_stash {
        state.pending_resume = Some(stash);
        state.overlays.launch_dialog = true;
        state.overlays.dialog_focus = 0;
    }

    // `--debug` (SQ-0449): persist the cumulative coverage on story-end, and
    // auto-open the debug inspector now (mirrors `/debug`'s open recipe). Tracing
    // was already enabled above; `set_debug_trace(true)` here is idempotent.
    state.persist_debug_trace = flags.debug;
    if flags.debug && session.debugger().is_some() {
        session.set_debug_trace(true);
        let dbg = session.debugger().expect("checked above");
        let mut panel = crate::debug_panel::DebugPanelState::new(dbg.pc());
        panel.apply_engine_layout(dbg);
        panel.refresh(dbg);
        state.debug = Some(panel);
        state.focus = crate::state::Focus::Map;
    }
    // The fork-and-probe seam (SQ-1121). Armed with the story's own bytes and the
    // boot facts that change how it runs, so the shadow a vetted suggestion is
    // tried in is the SAME game on the SAME machine — a shadow that differs in
    // any of them answers plausibly about a game the player is not playing. The
    // shadow itself is not booted here: most sessions never ask it anything.
    state.probe.arm(crate::probe::ShadowRecipe {
        story_bytes: std::sync::Arc::new(story_bytes.clone()),
        // The live game's own persistent data, read-only (SQ-1124). Without it a
        // shadow of Counterfeit Monkey re-runs the initialisation this launch
        // skipped, which is the whole of SQ-1121's "too slow to probe".
        store: game_dir.clone(),
        // Taken from the LIVE SESSION rather than from the sidecar on disk: on a
        // first launch the sidecar is empty and the session's is not, and it is
        // the session's that makes the shadow cheap.
        vfs_bytes: std::sync::Arc::new(session.vfs_bytes()),
        honor_game_colours: state.config.honor_game_colours,
        interpreter_number: state.config.interpreter_number,
        random_seed: Some(state.config.effective_random_seed()),
        acceleration: state.config.acceleration,
        screen: (
            state.config.virtual_screen_cols.unwrap_or(crate::config::FALLBACK_SCREEN_COLS) as u32,
            state.config.virtual_screen_rows.unwrap_or(crate::config::FALLBACK_SCREEN_ROWS) as u32,
        ),
    });

    Ok(BootedStory {
        session,
        mapper,
        state,
        game_dir,
        ifid,
        arc_file,
        story_bytes,
        story_path,
        data_base,
    })
}
