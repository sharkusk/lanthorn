//! `GlulxSession`: an [`Engine`] over a `gvm` Glulx machine + the app
//! [`AppGlk`] backend.
//!
//! The Z-machine's synchronous turn model fits `gvm` directly: a turn delivers
//! input to the pending Glk request, then **drives the gvm step loop** (like
//! `gvm-cli`'s `drive`) until the next `glk_select` request or Quit, draining the
//! [`AppGlk`] backend's output into the [`TurnResult`].
//!
//! Full introspection for Glulx is a later phase (SP4): [`introspect`] returns
//! `None`, so the tree-walking play-aids (inventory strip, here-column scope)
//! stay quiet. What IS answered: rooms via the heading heuristics below, and —
//! since SQ-1210 — the object-word set (`Engine::object_word_set`, backed by
//! `gvm::objects::ParseNames`), so the word reveal and the seen-words scrape
//! ask the story's own objects instead of its dictionary's flag bits. Glulx saves are tagged `"glulx"`; the 3b-i foreign-engine restore guard
//! prevents cross-loading a Z-machine save (and vice-versa).
//!
//! [`introspect`]: Engine::introspect
//! [`current_location`]: Engine::current_location

use std::any::Any;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use gvm::glk::GlkBackend;
use gvm::{GError, Machine, Memory, StepResult};

use crate::engine::{Engine, EngineError, EngineSave, KeyInput, LocationInfo, ScreenModel, StatusModel, WinNode};
use crate::glk_backend::AppGlk;
use crate::session::{clamp_runs, strip_read_prompt, trim_elems_to_len, FilenameReq, InputKind, PendingIo, TranscriptElem, TurnResult};
use zvm::location::LocationMethod;

/// The engine tag recorded in an `EngineSave` produced by the Glulx adapter.
pub const GLULX_ENGINE: &str = "glulx";
/// The save-format version within the `glulx` engine (gvm snapshot).
const GLULX_SAVE_FORMAT: u32 = 1;

// ── key → Glk keycode ──────────────────────────────────────────────────────────

/// Map a neutral [`KeyInput`] to a Glk character-input code (`keycode_*` from
/// `glk.h`, or a Latin-1/Unicode code point for a character).
///
/// Returns `None` for keys with no Glk meaning (Insert, F-keys past 12), so the
/// caller leaves the turn untouched — mirroring the Z-machine adapter's
/// "skip unmapped key" behavior.
pub fn key_to_glk(key: KeyInput) -> Option<u32> {
    use gvm::glk::keycode as kc;
    Some(match key {
        KeyInput::Char(c) => c as u32,
        KeyInput::Enter => kc::RETURN,
        KeyInput::Backspace | KeyInput::Delete => kc::DELETE,
        KeyInput::Tab => kc::TAB,
        KeyInput::Escape => kc::ESCAPE,
        KeyInput::Up => kc::UP,
        KeyInput::Down => kc::DOWN,
        KeyInput::Left => kc::LEFT,
        KeyInput::Right => kc::RIGHT,
        KeyInput::Home => kc::HOME,
        KeyInput::End => kc::END,
        KeyInput::PageUp => kc::PAGE_UP,
        KeyInput::PageDown => kc::PAGE_DOWN,
        // keycode_Func1 is the highest-valued F-key code; FuncN = Func1 - (N-1).
        KeyInput::Func(n) if (1..=12).contains(&n) => kc::FUNC1 - (n as u32 - 1),
        KeyInput::Func(_) | KeyInput::Insert => return None,
    })
}

// ── the session ────────────────────────────────────────────────────────────────

/// A running Glulx game session.
pub struct GlulxSession {
    pub(crate) machine: Machine,
    /// Which kind of input the VM is currently waiting for.
    pending: InputKind,
    /// Whether the game has ended.
    quit: bool,
    /// A game-initiated save/restore awaiting the host's file I/O, bubbled to the
    /// run loop via the next `TurnResult`. Set when a turn's drive stops on an
    /// `@save`/`@restore`; cleared by `resume_save`/`resume_restore`.
    pending_io: Option<PendingIo>,
    /// A game-initiated create_by_prompt awaiting a host filename, bubbled to the
    /// run loop. Set when a turn's drive stops on one; cleared by resume_filename.
    pending_filename: Option<FilenameReq>,
    /// A story-pane size reported while a game `@save`/`@restore`/`create_by_prompt`
    /// was suspended on the host's dialog, held until the dialog resolves. Applying
    /// it there and then would have to DRIVE the VM to deliver the Arrange, and a
    /// drive auto-fails the suspended op (see [`drive_settled`]) — a terminal
    /// resize would silently fail the save the player is still naming. The host's
    /// resize poller records the size as delivered and never retries it, so the
    /// session has to carry it. (SQ-0656)
    deferred_resize: Option<(u32, u32)>,
    /// The last screen snapshot (the backend's tree is only reachable mutably, so
    /// `screen()` returns this cache, refreshed after each turn).
    screen_cache: ScreenModel,
    /// The last `/dump-windows` diagnostic snapshot of the live Glk window tree,
    /// refreshed alongside `screen_cache` (the tree is only reachable mutably, so
    /// `window_dump()` returns this cache). (SQ-0329) The debug inspector's Glk
    /// section reuses this snapshot.
    pub(crate) window_dump_cache: Vec<String>,
    /// The debug inspector's lazily-built Glulx disassembly + discovery cache
    /// (SQ-0465). `None` until the inspector's first `debugger()` read builds it —
    /// discovery over a multi-MB I7 image is not free, so it never runs unless the
    /// inspector is actually opened. See [`crate::glulx_debug`].
    pub(crate) disasm_cache: std::cell::RefCell<Option<gvm::disasm::DisasmCache>>,
    /// Where this story keeps the words its parser accepts for an object —
    /// [`gvm::objects::ParseNames`], derived on first ask (SQ-1210). `None`
    /// inside the cell is a real answer, not a failure to look: a story whose
    /// RAM holds no Inform object list this reader can verify (glulxercise, a
    /// non-Inform compiler) refuses at detection, and every consumer falls back
    /// exactly as before the seam existed. A `OnceCell` because the answer
    /// describes the compiler's layout — the `$70` list's linkage is emitted by
    /// `Inform6/src/tables.c` and never rewritten at play — and because the
    /// detection is a scan of all of RAM, which must not run per turn. The
    /// same story survives `restore_state`/`restore_game_save` (a foreign
    /// engine's save is refused before the swap), so the layout does too.
    parse_names: std::cell::OnceCell<Option<gvm::objects::ParseNames>>,
    /// The [`parse_names`](Self::parse_names) walk folded into the one set the
    /// bulk callers query — "does ANY object answer to this word" (SQ-1176,
    /// SQ-1210). Same shape and soundness argument as
    /// `GameSession::object_word_set`: a cache of LIVE data, because the
    /// `name` arrays sit in RAM and a game can rewrite them, dropped wherever
    /// the VM runs — every drive on this session funnels through
    /// [`drive_turn`](Self::drive_turn), [`resize`](Self::resize),
    /// [`apply_deferred_resize`](Self::apply_deferred_resize),
    /// [`settle_after_event`](Self::settle_after_event) or the two restore
    /// paths, and each of those takes this cell.
    object_word_set: std::cell::RefCell<Option<std::sync::Arc<grammar_model::ObjectWordSet>>>,
    /// Auxiliary persistent data (Glulx aux persistence is a later phase).
    aux: BTreeMap<String, Vec<u8>>,
    aux_dirty: bool,
    /// The current room, derived from the last Inform `Subheader` heading and
    /// held sticky across heading-less turns (examine/talk/failed-move).
    last_room: Option<LocationInfo>,
    /// Learns which RAM word holds the game's `location` global, so same-named
    /// rooms get distinct ids (SQ-0526). See [`crate::glulx_roomlock`].
    room_lock: crate::glulx_roomlock::RoomLock,
    /// When false, the game's own trailing `>` read prompt is kept in the
    /// transcript instead of being stripped. Default true. See
    /// [`Engine::set_strip_prompt`].
    strip_prompt: bool,
    /// The per-story persistent store for the game's OWN fixed-name saves
    /// (`create_by_name`), and whether this session may write to it. Empty = no
    /// store (game-auto saves auto-fail). See [`drive_auto`] and [`GameStore`].
    store: GameStore,
}

/// Where a drive services the game's own fixed-name (`create_by_name`) saves,
/// and whether this session may WRITE there.
///
/// The directory and the permission are one subject and travel as one value
/// (CLAUDE.md's refactoring policy): a caller that supplied the path and forgot
/// the flag would get a session that silently writes into a store it was only
/// ever meant to read, and nothing downstream could tell.
///
/// Three shapes, and the third is what SQ-1124 added. A launched game gets a
/// [`writable`](GameStore::writable) store; a test gets [`none`](GameStore::none)
/// and the game's own saves auto-fail; a **shadow** ([`crate::probe`]) gets
/// [`read_only`](GameStore::read_only) — it may READ the live game's init cache,
/// which is the difference between Counterfeit Monkey's shadow booting in a
/// fraction of a second and re-running its whole initialisation, and it may write
/// nothing at all.
#[derive(Clone, Debug, Default)]
pub struct GameStore {
    dir: std::path::PathBuf,
    writable: bool,
}

impl GameStore {
    /// A store the game may read and write — a launched session.
    pub fn writable(dir: std::path::PathBuf) -> GameStore {
        GameStore { dir, writable: true }
    }

    /// A store the game may READ and never write — a shadow.
    pub fn read_only(dir: std::path::PathBuf) -> GameStore {
        GameStore { dir, writable: false }
    }

    /// No store at all: the game's own fixed-name saves auto-fail.
    pub fn none() -> GameStore {
        GameStore::default()
    }

    /// The directory, or an empty path when there is no store.
    fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// True when there is no store configured.
    fn absent(&self) -> bool {
        self.dir.as_os_str().is_empty()
    }

    /// True when this session may write into the store.
    fn may_write(&self) -> bool {
        self.writable && !self.absent()
    }
}

/// Wall-clock budget for a single drive (one turn's worth of execution). A
/// well-behaved game reaches an input request in milliseconds; if it runs this
/// long it is assumed to be in a runaway loop (e.g. layout code that cannot
/// converge on a given screen geometry) and the turn is aborted as a recoverable
/// fault so the app survives instead of hard-hanging. Generous, because this is a
/// last-resort backstop — `gvm` divides every proportional split in virtual
/// pixels and floors each child independently (SQ-1220), so the rounding a
/// layout loop feeds on (unequal halves of an odd split) does not arise.
/// Set via env `LANTHORN_TURN_BUDGET_MS` for testing.
fn turn_budget() -> Duration {
    std::env::var("LANTHORN_TURN_BUDGET_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(10))
}

/// Why a [`drive`] loop stopped.
enum DriveStop {
    /// The VM is waiting for input of this kind.
    Input(InputKind),
    /// The VM quit (or the runaway watchdog aborted the turn).
    Quit,
    /// A `glk_select` is waiting only on a non-input event (timer/mouse/hyperlink;
    /// Glk §4.4). No typed input is requested: the host's timer clock delivers
    /// ticks and a click routes to `deliver_mouse`/`deliver_hyperlink`.
    Event,
    /// The game executed `@save`: the host must write a save file, then
    /// [`GlulxSession::resume_save`].
    Save,
    /// The game executed `@restore`: the host must pick a save file, then
    /// [`GlulxSession::resume_restore`].
    Restore,
    /// The game executed `create_by_prompt`: the host must supply a filename, then
    /// [`GlulxSession::resume_filename`].
    Filename { usage: u32, fmode: u32 },
}

/// Step the machine until it pauses for input, quits, or requests an in-game
/// save/restore. Aborts a runaway turn via the wall-clock watchdog.
fn drive(machine: &mut Machine) -> DriveStop {
    let budget = turn_budget();
    let start = Instant::now();
    let mut steps: u64 = 0;
    loop {
        match machine.step() {
            StepResult::Continue => {
                steps += 1;
                // Checking the clock every step is too costly; sample periodically.
                if steps.is_multiple_of(1_000_000) && start.elapsed() > budget {
                    machine.abort_with_fault(format!(
                        "turn aborted after {:?} / {steps} steps with no input request \
                         (runaway game loop); the app stays interactive",
                        start.elapsed()
                    ));
                    return DriveStop::Quit;
                }
            }
            StepResult::Quit => return DriveStop::Quit,
            StepResult::NeedLine { .. } => return DriveStop::Input(InputKind::Line),
            StepResult::NeedChar { .. } => return DriveStop::Input(InputKind::Char),
            StepResult::NeedEvent { .. } => return DriveStop::Event,
            StepResult::SaveRequest => return DriveStop::Save,
            StepResult::RestoreRequest => return DriveStop::Restore,
            StepResult::NeedFilename { usage, fmode } => {
                // SavedGame usage: @save/@restore are host-intercepted (they become
                // Save/Restore requests served by lanthorn's own Save State dialogs),
                // so a save-file prompt here is redundant AND would split the turn
                // before @save. Auto-name it and keep driving to the opcode; the
                // synthesized VFS file is unused (the snapshot is host-side). Only the
                // real consumers (Transcript / InputRecord / Data) surface a request.
                if usage & 0x0f == 0x01 {
                    machine.supply_filename(Some(format!("__prompt_{}__", usage & 0x0f)));
                } else {
                    return DriveStop::Filename { usage, fmode };
                }
            }
        }
    }
}

/// Seed the machine's host-managed SavedGame existence index from every
/// `<store>/*.qzl` on disk (raw basename minus the `.qzl` suffix, matching
/// how [`drive_auto`] writes and how the index is keyed), so a `create_by_name`
/// game probing `glk_fileref_does_file_exist` before `@restore` sees its save
/// across launches (SQ-0301). No-op when there is no store, or it is unreadable.
/// Over-seeding player-save `.qzl` names is inert — a game only ever probes names
/// it created. Runs for a READ-ONLY store too: a shadow must see the cache it is
/// about to restore, or it never asks for it.
fn seed_saved_games(machine: &mut Machine, store: &GameStore) {
    if store.absent() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(store.dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("qzl") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0) as u32;
        machine.seed_saved_game_file(name.to_string(), size);
    }
}

/// Drive to a player-facing stop, transparently servicing the game's OWN
/// (`create_by_name`) `@save`/`@restore` against `store` — no host UI. A
/// game-managed `@save` writes `<store>/<name>.qzl`; a game-managed `@restore`
/// reads it if present, else fails cleanly (so a first run runs the game's init).
/// Only the player's SAVE/RESTORE verb (`create_by_prompt`), or any save when
/// there is no store, bubbles up as `DriveStop::Save`/`Restore`. A read-only
/// store reads as usual and answers every write with a clean failure.
fn drive_auto(machine: &mut Machine, store: &GameStore) -> DriveStop {
    loop {
        let stop = drive(machine);
        let restore = match stop {
            DriveStop::Save => false,
            DriveStop::Restore => true,
            other => return other,
        };
        let req = machine.pending_saveload_request().unwrap_or_default();
        // The player's verb (by_prompt), an unknown target, or no store: let the
        // caller decide (host UI in a turn, auto-fail in a non-interactive drive).
        if req.by_prompt || req.name.is_empty() || store.absent() {
            return stop;
        }
        let path = store.dir().join(format!("{}.qzl", req.name));
        if restore {
            match std::fs::read(&path) {
                Ok(bytes) if machine.complete_restore_quetzal(&bytes) => {}
                _ => machine.complete_restore_failure(),
            }
        } else if store.may_write() {
            let ok = std::fs::create_dir_all(store.dir()).is_ok()
                && std::fs::write(&path, machine.save_quetzal()).is_ok();
            machine.complete_save(ok);
        } else {
            // A READ-ONLY store (a shadow). The game is told its own save failed,
            // which it already handles — a first run of any story gets exactly
            // that answer — and nothing reaches the disk. Reads above are still
            // served, which is the whole point: the cache the live session wrote
            // is what makes the shadow's boot cheap.
            machine.complete_save(false);
        }
        // Keep driving: the op may chain into another game-managed save/restore.
    }
}

/// Drive to an input request or quit, returning `(pending_kind, quit)`. A
/// player `@save`/`@restore` fired during a non-interactive drive (startup,
/// resize, sound-notify) is auto-failed — those paths have no UI to prompt the
/// player, and leaving the VM suspended would wedge the next turn. The game's
/// OWN fixed-name saves are serviced silently by [`drive_auto`] first.
fn drive_settled(machine: &mut Machine, store: &GameStore) -> (InputKind, bool) {
    loop {
        match drive_auto(machine, store) {
            DriveStop::Input(k) => return (k, false),
            DriveStop::Event => return (InputKind::Event, false),
            DriveStop::Quit => return (InputKind::Line, true),
            DriveStop::Save => machine.complete_save(false),
            DriveStop::Restore => machine.complete_restore_failure(),
            DriveStop::Filename { .. } => machine.supply_filename(None),
        }
    }
}

impl GlulxSession {
    /// Build a session from a raw Glulx image, reporting a `cols × rows` display.
    ///
    /// Steps to the first input request / quit (driving the opening text into the
    /// backend); the text is NOT drained here, so `take_transcript` returns the
    /// banner.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        image: Vec<u8>,
        cols: u32,
        rows: u32,
        acceleration: bool,
        graphics_enabled: bool,
        sound_enabled: bool,
        char_px: (u32, u32),
        pict_blorb: Option<blorb::Blorb>,
        vfs_bytes: &[u8],
    ) -> Result<GlulxSession, GError> {
        // No persistent store: the game's own fixed-name @save/@restore auto-fail
        // (empty game_dir), matching pre-persistence behavior. No theme colours
        // (style_measure of an unhinted colour reports unknown). Used by tests.
        Self::new_in(
            std::path::PathBuf::new(), image, cols, rows, acceleration,
            graphics_enabled, sound_enabled, false, char_px, pict_blorb, vfs_bytes,
            [[(None, None); 11]; 2], false, None,
        )
    }

    /// Like [`GlulxSession::new`] but with a `game_dir` persistent store: the
    /// game's OWN fixed-name saves (`create_by_name` — CM's init cache, undo,
    /// autotesting) are serviced silently against `<game_dir>/<name>.qzl` during
    /// every drive (including boot), with no host UI. Only the player's SAVE/
    /// RESTORE verb (`create_by_prompt`) bubbles up for the app's saves dialog.
    ///
    /// `debug` enables the debug inspector's execution tracing from the very first
    /// boot instruction (the `--debug` flag); leave it `false` for a normal launch.
    ///
    /// `random_seed` is the value the story's `random` opcode starts from
    /// (SQ-0811). It is applied before the boot drive below, because a Glulx
    /// game's initialisation may already draw from the generator — and for a
    /// roguelike that is precisely where the dungeon is dealt. `None` — every
    /// caller but the launcher — leaves gvm's own fixed default, so a test's
    /// sequence stays the reproducible one it has always been.
    #[allow(clippy::too_many_arguments)]
    pub fn new_in(
        game_dir: std::path::PathBuf,
        image: Vec<u8>,
        cols: u32,
        rows: u32,
        acceleration: bool,
        graphics_enabled: bool,
        sound_enabled: bool,
        borderless: bool,
        char_px: (u32, u32),
        pict_blorb: Option<blorb::Blorb>,
        vfs_bytes: &[u8],
        theme: crate::glk_backend::GlkStylePairs,
        debug: bool,
        random_seed: Option<u32>,
    ) -> Result<GlulxSession, GError> {
        let store = if game_dir.as_os_str().is_empty() {
            GameStore::none()
        } else {
            GameStore::writable(game_dir)
        };
        Self::new_with_store(
            store, image, cols, rows, acceleration, graphics_enabled, sound_enabled,
            borderless, char_px, pict_blorb, vfs_bytes, theme, debug, random_seed,
        )
    }

    /// A silent copy of a launched game, for [`crate::probe`] — the same
    /// constructor with a **read-only** persistent store.
    ///
    /// This is the whole of SQ-1124's boot fix, and it is one word. A shadow
    /// booted with no store re-runs the story's initialisation from cold, which
    /// on Counterfeit Monkey is millions of opcodes even accelerated; booted
    /// against the live game's own store it takes the same `@restore`-the-init-
    /// cache path the live session took at launch (the CHANGELOG's "5.4s →
    /// 0.76s from the second launch"), and reaches a prompt in a fraction of the
    /// time. The store is read-only, so the shadow can never write the cache it
    /// reads, nor anything else — see [`GameStore`].
    #[allow(clippy::too_many_arguments)]
    pub fn new_shadow(
        game_dir: std::path::PathBuf,
        image: Vec<u8>,
        cols: u32,
        rows: u32,
        acceleration: bool,
        vfs_bytes: &[u8],
        random_seed: Option<u32>,
    ) -> Result<GlulxSession, GError> {
        Self::new_with_store(
            GameStore::read_only(game_dir),
            image,
            cols,
            rows,
            acceleration,
            false, // graphics
            false, // sound
            false, // borderless
            (8, 16),
            None, // no picture Blorb
            vfs_bytes,
            [[(None, None); 11]; 2],
            false, // no execution trace
            random_seed,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_store(
        store: GameStore,
        image: Vec<u8>,
        cols: u32,
        rows: u32,
        acceleration: bool,
        graphics_enabled: bool,
        sound_enabled: bool,
        borderless: bool,
        char_px: (u32, u32),
        pict_blorb: Option<blorb::Blorb>,
        vfs_bytes: &[u8],
        theme: crate::glk_backend::GlkStylePairs,
        debug: bool,
        random_seed: Option<u32>,
    ) -> Result<GlulxSession, GError> {
        let mem = Memory::new(image)?;
        let picts = crate::graphics::PictSource::new(pict_blorb);
        let mut backend = Box::new(AppGlk::with_graphics(cols, rows, char_px, picts));
        // Theme colours must be in place BEFORE boot: a game may probe
        // glk_style_measure for the host's rendered colours during its startup
        // (SQ-0315; Kerkerkruip probes its style_User2 slot there, SQ-0803).
        backend.set_theme_colours(theme);
        let mut machine = Machine::with_glk(mem, backend);
        machine.set_acceleration(acceleration);
        machine.set_graphics(graphics_enabled);
        machine.set_sound(sound_enabled);
        if let Some(seed) = random_seed {
            machine.set_rng_seed(seed);
        }
        // Per-game borderless-windows mode: applies from the first relayout at
        // boot, so the game's windows abut with no reserved gutter (SQ-0341).
        machine.set_borderless(borderless);
        // Load the per-story Glk file VFS sidecar BEFORE booting: a Glulx game
        // may read a cache during boot (e.g. CM skips its long init) or write one
        // (leaving vfs_dirty set), so the sidecar must be in place first (SQ-0290).
        machine.load_vfs(vfs_bytes);
        // Reseed host-managed SavedGame slots from disk BEFORE booting: `.qzl`
        // saves live in host files decoupled from the VFS and the machine's
        // existence index is session-transient, so a create_by_name game probing
        // glk_fileref_does_file_exist during init would otherwise never see its
        // own on-disk save across launches (SQ-0301).
        seed_saved_games(&mut machine, &store);
        // `--debug` (SQ-0465): enable execution tracing BEFORE the boot drive so
        // the game's initialisation code is captured in the coverage set — a
        // later `/debug` toggle cannot see the boot PCs. Off by default, so a
        // normal launch keeps the single-branch hot loop with zero trace work.
        machine.trace_exec = debug;
        let (pending, quit) = drive_settled(&mut machine, &store);
        let mut session = GlulxSession {
            machine,
            pending,
            quit,
            pending_io: None,
            pending_filename: None,
            deferred_resize: None,
            screen_cache: blank_screen(),
            window_dump_cache: Vec::new(),
            disasm_cache: std::cell::RefCell::new(None),
            parse_names: std::cell::OnceCell::new(),
            object_word_set: std::cell::RefCell::new(None),
            aux: BTreeMap::new(),
            aux_dirty: false,
            last_room: None,
            room_lock: crate::glulx_roomlock::RoomLock::new(0, 0),
            strip_prompt: true,
            store,
        };
        session.refresh_screen();
        session.room_lock = match session.remembered_room_global() {
            Some(addr) => crate::glulx_roomlock::RoomLock::locked_at(
                session.machine.mem().ramstart(),
                session.scan_words(),
                addr,
            ),
            None => crate::glulx_roomlock::RoomLock::new(
                session.machine.mem().ramstart(),
                session.scan_words(),
            ),
        };
        // Resolve the opening room's id the way a live TURN does, not with a bare
        // name hash. When this story's `location` global was learned in an earlier
        // run the lock is already restored above, so a turn keys rooms by the room's
        // own address while the boot keyed them by name — two ids for the room you
        // are standing in, so the first action you took spawned a duplicate of the
        // opening room. Same defect `seed_last_room` fixes for a resume (SQ-0526);
        // the boot path had its own copy. `room_for` still falls back to the name
        // hash when nothing is locked, which is every game that never learns.
        let ram = session.scan_ram();
        let awaiting_line_input = session.pending == InputKind::Line;
        session.last_room = session
            .appglk()
            .take_room_heading(awaiting_line_input)
            .map(|n| session.room_for(&n, &ram));
        Ok(session)
    }

    /// Report a new display size to the backend (the host story-pane size).
    pub fn set_screen_size(&mut self, cols: u32, rows: u32) {
        self.appglk().set_screen_size(cols, rows);
    }

    /// Push the current theme's rendered default colours (a pair per Glk style
    /// class, buffer and grid) to the backend for `glk_style_measure` (SQ-0315,
    /// SQ-0803). Called on boot and kept fresh by the run loop so a live style
    /// reload is reflected.
    pub fn set_theme_colours(&mut self, theme: crate::glk_backend::GlkStylePairs) {
        self.appglk().set_theme_colours(theme);
    }

    /// Enable/disable Glk sound on the running VM (the Sound gestalt + schannel
    /// dispatch), so a runtime sound toggle reaches games that re-check
    /// `gestalt_Sound` before playing.
    pub fn set_sound(&mut self, on: bool) {
        self.machine.set_sound(on);
    }

    /// React to a change in the host story-pane size: report it to the backend,
    /// relayout the window tree to the new geometry (rescaling graphics canvases),
    /// and — if the game is waiting on input — deliver a Glk Arrange event so it
    /// redraws (e.g. a graphics window repaints its image at the new size). Drives
    /// the game's redraw to its next input request and refreshes the screen cache.
    /// A no-op once the game has quit.
    pub fn resize(&mut self, cols: u32, rows: u32) {
        if self.quit {
            return;
        }
        // Never drive the VM while a game `@save`/`@restore`/`create_by_prompt` is
        // parked on the host's dialog: `drive_settled` auto-fails a suspended op
        // (there is no UI to defer to from a non-interactive drive), so a terminal
        // resize would record the player's in-flight save as FAILED while its name
        // prompt is still open — and the later confirm would write the archive over
        // a game that already saw a failure. Queue the size for the resume instead.
        // (SQ-0656)
        if self.game_io_pending() {
            self.deferred_resize = Some((cols, rows));
            return;
        }
        self.set_screen_size(cols, rows);
        self.machine.rearrange();
        self.object_word_set.take(); // the drive runs game code (SQ-1176 duty)
        let (pending, quit) = drive_settled(&mut self.machine, &self.store);
        self.pending = pending;
        self.quit = quit;
        self.refresh_screen();
    }

    /// Is a game-initiated `@save`/`@restore` or `create_by_prompt` suspended,
    /// waiting for the host's dialog to resolve? While this holds, the VM must not
    /// be driven by anything but the matching `resume_*` — every other drive path
    /// funnels through [`drive_settled`], which answers a suspended op with failure
    /// rather than leaving the VM blocked. Reads the machine rather than the
    /// session's own `pending_io`, which `finish_turn` takes on its way out.
    /// (SQ-0656)
    fn game_io_pending(&self) -> bool {
        self.machine.is_saveload_pending() || self.pending_filename.is_some()
    }

    /// Apply a resize that arrived while a dialog held the VM (see
    /// [`Self::resize`]). Called from each `resume_*` once the op has been
    /// answered: a chained request keeps the size queued for the next resume, and
    /// a quit drops it. (SQ-0656)
    fn apply_deferred_resize(&mut self) {
        let Some((cols, rows)) = self.deferred_resize else { return };
        if self.quit {
            self.deferred_resize = None;
            return;
        }
        if self.game_io_pending() {
            return;
        }
        self.deferred_resize = None;
        self.set_screen_size(cols, rows);
        self.machine.rearrange();
        self.object_word_set.take(); // the drive runs game code (SQ-1176 duty)
        let (pending, quit) = drive_settled(&mut self.machine, &self.store);
        self.pending = pending;
        self.quit = quit;
    }

    /// Drive to the next input request after a host-side event was delivered
    /// (timer tick, sound/volume notify, mouse, hyperlink) — unless a game
    /// save/restore/filename request is suspended on the host's dialog, in which
    /// case the event stays QUEUED in the Glk model (that is what
    /// `Machine::deliver_*` does when the VM is not parked at a `glk_select`) and
    /// the game picks it up at its next select, after the dialog resolves.
    /// Driving here instead would auto-fail the player's in-flight save. (SQ-0656)
    fn settle_after_event(&mut self) {
        if self.game_io_pending() {
            return;
        }
        self.object_word_set.take(); // the drive runs game code (SQ-1176 duty)
        let (pending, quit) = drive_settled(&mut self.machine, &self.store);
        self.pending = pending;
        self.quit = quit;
    }

    /// Toggle the per-game borderless-windows mode live and relayout so the
    /// change shows immediately: gvm re-lays out its windows (with or without the
    /// reserved gutter cells) and the game redraws to its next input request.
    /// A no-op once the game has quit. (SQ-0341)
    pub fn set_borderless(&mut self, on: bool) {
        if self.quit {
            return;
        }
        self.machine.set_borderless(on);
        self.machine.rearrange();
        // Same rule as `resize`: no non-interactive drive while a dialog holds a
        // suspended `@save`/`@restore` (SQ-0656). The mode is already set on the
        // machine and the tree relaid out; the game hears about it at its next
        // select. Nothing to queue.
        self.settle_after_event();
        self.refresh_screen();
    }

    /// Drive one turn's worth of execution, updating `pending`/`quit`/`pending_io`.
    /// On an in-game `@save`/`@restore` the drive stops with `pending_io` set (and
    /// `pending`/`quit` left unchanged, since the game is mid-turn); the run loop
    /// performs the file I/O and calls `resume_save`/`resume_restore`.
    fn drive_turn(&mut self) {
        // The VM is about to run, so RAM may change under the cached
        // object-word set — a game CAN rewrite an object's `name` array
        // mid-play, and a stale set would keep answering for the old words.
        // Same per-turn invalidation the Z-machine's `drain_turn` performs
        // (SQ-1176); the other drive paths on this session (`resize`,
        // `apply_deferred_resize`, `settle_after_event`, the two restores) each
        // take the cell too.
        self.object_word_set.take();
        match drive_auto(&mut self.machine, &self.store) {
            DriveStop::Input(k) => {
                self.pending = k;
                self.quit = false;
                self.pending_io = None;
                self.pending_filename = None;
            }
            DriveStop::Event => {
                // Waiting on a timer/mouse/hyperlink only: no typed input pending.
                // The run loop's Glk timer clock delivers ticks; a click routes to
                // deliver_mouse/deliver_hyperlink.
                self.pending = InputKind::Event;
                self.quit = false;
                self.pending_io = None;
                self.pending_filename = None;
            }
            DriveStop::Quit => {
                self.quit = true;
                self.pending_io = None;
                self.pending_filename = None;
            }
            DriveStop::Save => self.pending_io = Some(PendingIo::Save),
            DriveStop::Restore => self.pending_io = Some(PendingIo::Restore),
            DriveStop::Filename { usage, fmode } => {
                self.pending_filename = Some(FilenameReq { usage, fmode })
            }
        }
    }

    /// The reader for the words this story's parser accepts for an object,
    /// derived on first use — see [`parse_names`](Self::parse_names) the field.
    /// `None` is the documented refusal: no verified Inform object list in this
    /// image, and every consumer keeps its pre-SQ-1210 fallback.
    pub fn parse_names(&self) -> Option<&gvm::objects::ParseNames> {
        self.parse_names
            .get_or_init(|| gvm::objects::ParseNames::detect(self.machine.mem()).ok())
            .as_ref()
    }

    fn appglk(&mut self) -> &mut AppGlk {
        self.machine
            .backend_mut()
            .as_any_mut()
            .downcast_mut::<AppGlk>()
            .expect("GlulxSession always drives an AppGlk backend")
    }

    fn refresh_screen(&mut self) {
        // `screen_model` now derives the pane background/foreground from the
        // PRIMARY buffer window's own Normal-style colour snapshot (per-window),
        // so a game that styles its text for a light interpreter (e.g.
        // CounterfeitMonkey's black-on-white intro) paints a matching pane while
        // each other window keeps its own colour on its node. (SQ-0196/SQ-0328)
        // Re-push the live window tree first: a window's per-window colours can
        // change after it opens (Kerkerkruip sets its panel backgrounds post-open),
        // and the tree is otherwise only refreshed on a structural relayout — so
        // without this the pane would render with stale/absent backgrounds until a
        // manual resize forced a relayout (SQ-0332).
        self.machine.sync_window_tree();
        self.sync_input_window();
        let model = self.appglk().screen_model();
        self.screen_cache = model;
        self.window_dump_cache = self.appglk().window_dump_lines();
    }

    /// Point the backend's primary buffer at the window awaiting line input, so
    /// the inline prompt + transcript follow the player's actual prompt window
    /// (narco opens a decorative pane first, then does everything in its second
    /// buffer). A no-op for the usual single-buffer game. (SQ-0337)
    fn sync_input_window(&mut self) {
        let win = self.machine.line_request_window();
        self.appglk().set_input_window(win);
    }

    /// Drain the primary window output + refresh the screen, building the turn
    /// result. Shared by `submit`/`submit_key`/`resume_*`.
    fn finish_turn(&mut self) -> TurnResult {
        self.machine.flush();
        // Point the primary at the window now awaiting line input BEFORE draining
        // it: narco moves its line request between two buffers (right while
        // "dreaming", left when awake), and the drain/prompt must follow. (SQ-0337)
        self.sync_input_window();
        // Drain the primary window's ordered elements (text runs + inline
        // images). Derive the flat `raw`/`raw_runs` from the Text elements for
        // the existing consumers (banner-strip, location, tests): the
        // concatenation of element text equals what the old text-only drain
        // produced.
        let mut elems = self.appglk().take_transcript_elems();
        let mut raw = String::new();
        let mut raw_runs = Vec::new();
        for e in &elems {
            if let TranscriptElem::Text { text, runs } = e {
                raw.push_str(text);
                raw_runs.extend(runs.iter().copied());
            }
        }
        // Strip the game's trailing read prompt (e.g. a final "\n>") so the app's
        // own bottom input bar is the only ">" shown -- mirroring the Z-machine
        // path. clamp_runs keeps the style chunks aligned with the shortened text,
        // and trim_elems_to_len applies the same shortening to the element list so
        // the ordered elems stay consistent with the flat `transcript`.
        let transcript = if self.strip_prompt { strip_read_prompt(&raw).to_owned() } else { raw };
        let kept = transcript.chars().count();
        let transcript_runs = clamp_runs(raw_runs, kept);
        trim_elems_to_len(&mut elems, kept);
        self.refresh_screen();
        // SQ-0526: fold this turn into the room-lock learner, then resolve the
        // room. `movement` is what we can say for sure about the room changing —
        // and a repeated heading says NOTHING, because it is either a `look` or a
        // move into a different room with the same name (the maze).
        // A heading is only a room when the turn hands the player the command
        // prompt; one printed on a "press any key" page is a banner (SQ-0732),
        // and so is one on a page that reads a line without prompting for a
        // command — cragne Manor's content warning (SQ-0733).
        let awaiting_line_input = self.pending == InputKind::Line;
        let heading = self.appglk().take_room_heading(awaiting_line_input);
        let movement = match (&heading, self.last_room.as_ref().map(|r| &r.name)) {
            (None, _) => crate::glulx_roomlock::Movement::Unchanged,
            (Some(h), Some(prev)) if h == prev => crate::glulx_roomlock::Movement::Ambiguous,
            (Some(_), _) => crate::glulx_roomlock::Movement::Changed,
        };
        let ram = self.scan_ram();
        let name = heading.clone().or_else(|| self.last_room.as_ref().map(|r| r.name.clone()));
        let was_locked = self.room_lock.locked();
        self.room_lock.observe(ram.clone(), name.clone(), movement);
        if let (None, Some(addr)) = (was_locked, self.room_lock.locked()) {
            self.remember_room_global(addr);
        }
        // Re-resolve the room every turn, not only when a heading is printed: a
        // maze step prints the SAME heading from a different room, so the id has
        // to come from the global even when the name did not change.
        if let Some(n) = name {
            self.last_room = Some(self.room_for(&n, &ram));
        }
        let location = self.last_room.clone();
        let location_method = location.as_ref().map(|_| LocationMethod::RoomHeading);
        let diagnostics = std::mem::take(&mut self.machine.diagnostics);
        let fault = self.machine.take_fault_trace().map(|t| t.to_lines());
        let glulx_sound_ops = self.appglk().take_sound_ops();
        TurnResult {
            transcript,
            transcript_runs,
            location,
            quit: self.quit,
            // A `glk_window_clear` on the primary buffer (e.g. an Inform 7 menu
            // redraw) pins the reprint to a fresh screen instead of stacking a
            // fresh copy of the menu on every keypress. (SQ-0403)
            erase_lower: Engine::drain_screen_clear(self),
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops,
            diagnostics,
            fault,
            location_method,
            pending_io: self.pending_io.take(),
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: elems,
            prose_retired: None,
        }
    }

    /// Words of RAM the room-lock learner scans, from `ramstart`.
    ///
    /// Inform lays its globals out at the start of RAM — Adventure's `location`
    /// sits at `ramstart+0x28` — so a bounded window finds it while keeping both
    /// the per-turn diff and the retained learning snapshots small. An I7 game's
    /// RAM can run to megabytes; scanning all of it every turn, and holding a
    /// dozen copies, would cost far more than the feature is worth. A game whose
    /// global lies beyond the window simply never locks and keeps the name-derived
    /// ids it has today.
    fn scan_words(&self) -> usize {
        const WINDOW_BYTES: u32 = 64 * 1024;
        let mem = self.machine.mem();
        let start = mem.ramstart();
        let end = mem.endmem().min(start.saturating_add(WINDOW_BYTES));
        (end.saturating_sub(start) / 4) as usize
    }

    /// A snapshot of the scanned RAM window.
    fn scan_ram(&self) -> Vec<u32> {
        let base = self.machine.mem().ramstart();
        (0..self.scan_words())
            .map(|i| self.machine.mem().read32(base + (i as u32) * 4).unwrap_or(0))
            .collect()
    }

    /// The room id for this turn: the locked `location` global when it is known,
    /// else the hash of the room name — which is what every Glulx game used before
    /// the lock existed, and what a game that never locks keeps using.
    fn room_for(&self, name: &str, ram: &[u32]) -> LocationInfo {
        match self.room_lock.room_id(ram) {
            Some(addr) => zvm::ObjectSnapshot {
                number: crate::roomid::glulx_room_id(addr),
                parent: 0,
                name: name.to_string(),
            },
            None => heading_to_room(name),
        }
    }

    /// The re-key table produced when the lock resolves: rooms mapped under a
    /// name-derived id while learning, paired with their real ids. Empty on every
    /// other turn. The caller applies it to the map so the pre-lock rooms are the
    /// same nodes as the post-lock ones instead of duplicates.
    pub fn take_room_remap(&mut self) -> Vec<(String, u32)> {
        self.room_lock.take_remap()
    }

    /// Where the learned `location` address is remembered for this story.
    fn room_global_path(&self) -> Option<std::path::PathBuf> {
        (!self.store.absent()).then(|| self.store.dir().join("room-global"))
    }

    /// The address learned in an earlier run of this story, if any.
    ///
    /// Persisting it is not an optimisation, it is what keeps a RESUME correct:
    /// the learner needs unambiguous room changes to converge, and a save parked
    /// somewhere ambiguous — the maze, where every heading repeats — could never
    /// re-learn. Falling back to name-derived ids there would key the resumed
    /// rooms differently from the ones already in the restored map and duplicate
    /// every one of them. Object addresses are fixed for a given story file, so a
    /// remembered address stays valid; it is verified every turn regardless, and
    /// dropped if the game ever contradicts it.
    fn remembered_room_global(&self) -> Option<u32> {
        let raw = std::fs::read_to_string(self.room_global_path()?).ok()?;
        raw.trim().parse::<u32>().ok().filter(|&a| a != 0)
    }

    /// Remember a freshly learned address for later runs. Best-effort: a story
    /// with no writable directory simply re-learns next time.
    fn remember_room_global(&self, addr: u32) {
        if !self.store.may_write() {
            return;
        }
        if let Some(p) = self.room_global_path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(p, addr.to_string());
        }
    }

    /// The learned `location` address, for persisting across runs.
    pub fn locked_room_global(&self) -> Option<u32> {
        self.room_lock.locked()
    }

    /// Start already locked to a previously learned address (see
    /// [`Self::locked_room_global`]). Object addresses are fixed for a given story
    /// file, so a remembered address is valid on every later run — and the lock is
    /// still verified each turn, so a wrong one is dropped rather than trusted.
    pub fn relock_room_global(&mut self, addr: u32) {
        self.room_lock = crate::glulx_roomlock::RoomLock::locked_at(
            self.machine.mem().ramstart(),
            self.scan_words(),
            addr,
        );
    }

    /// Seed the cached location after a host Save State resume (SQ-0523).
    ///
    /// Glulx has no object tree to interrogate, so the current room is recovered
    /// from the room HEADING the game prints (`Subheader` style) and cached in
    /// `last_room` — host-side state that lives outside the VM snapshot. A resume
    /// therefore restored the VM correctly but left this cache holding the room
    /// the FRESH BOOT printed, so the map selected and centred on the game's
    /// opening room until the next turn produced a heading and corrected it: save
    /// deep in Adventure's maze, resume, and the map jumped to "At End Of Road"
    /// until you typed `look`.
    ///
    /// `name` is the room name the archive recorded (`Meta::location`), and the id
    /// is rebuilt with the same [`heading_to_room`] the live path uses, so the
    /// seeded room is identical to the one the next heading would produce. The
    /// Z-machine needs no equivalent: its `current_location` reads the restored
    /// object tree.
    pub fn seed_last_room(&mut self, name: &str) {
        // Resolve the id exactly as a live turn does (SQ-0526): once the location
        // global is known, rooms are keyed by the room's own address, and the
        // restored VM already holds the right value. Building a NAME-derived id
        // here instead produced a second, disconnected node for the room the
        // player was actually standing in — the map jumped to the real one on the
        // next turn and left the impostor behind.
        let ram = self.scan_ram();
        self.last_room = Some(self.room_for(name, &ram));
    }

    /// The Glk timer interval the game has requested (via
    /// `glk_request_timer_events`), or `None` when timer events are off. The host
    /// reads this to arm its own clock and calls [`Self::deliver_timer`] once per
    /// interval.
    pub fn timer_interval(&self) -> Option<std::time::Duration> {
        self.machine
            .glk_timer_interval()
            .map(|ms| std::time::Duration::from_millis(ms as u64))
    }

    /// A timer interval elapsed: deliver a Glk `Evtype_Timer` event to the game
    /// and drive it to its next input request, returning the resulting turn. A
    /// no-op turn once the game has quit.
    pub fn deliver_timer(&mut self) -> TurnResult {
        if !self.quit {
            self.machine.deliver_timer();
            self.settle_after_event();
        }
        self.finish_turn()
    }

    /// A sound finished: deliver a Glk `Evtype_SoundNotify` to the game and drive
    /// it to its next input request, returning the resulting turn (which carries
    /// any sound ops the game buffered while handling the notify — sound
    /// sequencing). A no-op turn once the game has quit.
    pub fn sound_notify(&mut self, sound: u32, notify: u32) -> TurnResult {
        if !self.quit {
            self.machine.deliver_sound_notify(sound, notify);
            self.settle_after_event();
        }
        self.finish_turn()
    }

    /// A gradual volume change completed (Sound2 `set_volume_ext`): deliver a Glk
    /// `Evtype_VolumeNotify` to the game and drive it to its next input request.
    /// Mirrors [`Self::sound_notify`]. A no-op turn once the game has quit.
    pub fn volume_notify(&mut self, notify: u32) -> TurnResult {
        if !self.quit {
            self.machine.deliver_volume_notify(notify);
            self.settle_after_event();
        }
        self.finish_turn()
    }

    /// The ids of every window with an active Glk mouse request — the windows a
    /// terminal click may be diverted into. Empty when no window is watching for
    /// clicks.
    ///
    /// Ids only, not rects (SQ-1203): gvm's own layout rect reserves a 1-cell
    /// border gutter per bordered split whether or not the theme actually draws a
    /// rule there (`upper_window_border` defaults to `BorderStyle::None`,
    /// SQ-0821), so it disagrees with what the renderer painted by every
    /// collapsed gutter between the pane origin and the window. The hit-test
    /// (`glk_mouse_target`) uses the render's own recorded rects
    /// (`StoryPaneMetrics::win_rects`) instead — "ask the drawing where it put
    /// the text", not gvm's layout.
    pub fn mouse_windows(&mut self) -> Vec<u32> {
        let ids: Vec<u32> = self.appglk().layout().iter().map(|&(id, ..)| id).collect();
        ids.into_iter().filter(|&id| self.machine.mouse_requested(id)).collect()
    }

    /// The ids of every window with an active Glk hyperlink request — the
    /// windows a click on a linked transcript cell may be diverted into. Empty
    /// when no window is watching for hyperlink clicks. Ids only, for the same
    /// reason as [`Self::mouse_windows`] (SQ-1203).
    pub fn hyperlink_windows(&mut self) -> Vec<u32> {
        let ids: Vec<u32> = self.appglk().layout().iter().map(|&(id, ..)| id).collect();
        ids.into_iter().filter(|&id| self.machine.hyperlink_requested(id)).collect()
    }

    /// The `(width, height)` of one text-grid cell in pixels — used to convert a
    /// click's window-relative cells into pixels for a graphics window.
    pub fn char_pixels(&mut self) -> (u32, u32) {
        self.appglk().char_pixels()
    }

    /// Take what the app's input line should hold for the newest line-input request,
    /// if one has appeared since the last call (Glk spec §4.2 `initlen`). One-shot
    /// per request; `Some("")` means "start empty", which is how the app learns the
    /// previous line is gone. advent.blb's toolbar drives both cases — a verb button
    /// primes `"Examine "`, and a verb clicked over an already-typed noun runs the
    /// whole command itself and asks again empty.
    pub fn take_line_seed(&mut self) -> Option<String> {
        self.machine.take_line_seed()
    }

    /// Keep the pending line request's buffer holding exactly what the player has
    /// typed so far (Glk §4.2). A game that cancels line input mid-edit reads that
    /// buffer back as the partial input — advent.blb's toolbar does it on every
    /// button press — so the app must not let it go stale. A no-op when the game is
    /// not awaiting a line.
    pub fn sync_line_input(&mut self, text: &str) {
        self.machine.set_line_input_text(text);
    }

    /// A terminal click landed inside a mouse-watching window: deliver a Glk
    /// `Evtype_MouseInput` event at window-relative `(x, y)` and drive the game to
    /// its next input request. A no-op turn once the game has quit. `x`/`y` are
    /// char col/row for a grid window, pixels for a graphics window.
    pub fn deliver_mouse(&mut self, win: u32, x: u32, y: u32) -> TurnResult {
        let (x, y) = self.clamp_into_window(win, x, y);
        if !self.quit {
            self.machine.deliver_mouse(win, x, y);
            self.settle_after_event();
        }
        self.finish_turn()
    }

    /// Pull a window-relative click back inside the window the GAME thinks it
    /// has. The DRAWN rect a click is hit-tested against is wider than gvm's own
    /// by every gap the renderer hands to the trailing child: the separator the
    /// theme declined to draw (SQ-1203) and, since SQ-1220, the padding cell a
    /// proportional split could not divide — two cells for City of Secrets' help
    /// menu, whose grid is on the far side of a 15 % split. Both gaps are ours,
    /// the game was told a smaller window, and Glk §8.6 promises a mouse event's
    /// coordinates lie inside it — so a click on a cell it has never heard of
    /// arrives at the nearest one it has rather than off the end of its menu.
    ///
    /// Text windows only. A graphics window's coordinates are PIXELS against a
    /// canvas the render scales to the drawn rect, so they are in range by
    /// construction and there is nothing here to clamp them against.
    fn clamp_into_window(&mut self, win: u32, x: u32, y: u32) -> (u32, u32) {
        let reported = self
            .appglk()
            .layout()
            .iter()
            .find(|&&(id, ty, ..)| id == win && ty != gvm::glk::WinType::Graphics)
            .map(|&(_, _, r, _)| (r.width, r.height));
        match reported {
            Some((w, h)) => (x.min(w.saturating_sub(1)), y.min(h.saturating_sub(1))),
            None => (x, y),
        }
    }

    /// A click landed on a linked transcript cell inside a hyperlink-watching
    /// window: deliver a Glk `Evtype_Hyperlink` event carrying `link` (the link
    /// value from the cell→link map) and drive the game to its next input
    /// request. A no-op turn once the game has quit. One-shot — the gvm
    /// `deliver_hyperlink` consumes the request, so the game must re-arm.
    pub fn deliver_hyperlink(&mut self, win: u32, link: u32) -> TurnResult {
        if !self.quit {
            self.machine.deliver_hyperlink(win, link);
            self.settle_after_event();
        }
        self.finish_turn()
    }

    /// The bare, standard Glulx-Quetzal bytes for the game's own in-game
    /// `@save` (VM state only — no `GReg`/`Glk ` chunks; resumed via a call
    /// stub). Distinct from [`Engine::save_state`], which is the full host
    /// snapshot behind Save State (`.lanthorn`).
    pub fn save_quetzal(&self) -> Vec<u8> {
        self.machine.save_quetzal()
    }
}

/// Decide whether a terminal click at absolute `(col, row)` should be diverted
/// to the game as a Glk mouse-input event, and if so compute its coordinates.
///
/// Returns `(win, val1, val2)` only when no overlay is open, `win` is currently
/// mouse-watching (`requested`, as reported by [`GlulxSession::mouse_windows`]),
/// and the click lands inside that window's rect as the renderer actually DREW
/// it (`win_rects`, from `StoryPaneMetrics::win_rects` — absolute screen
/// coordinates, one entry per Glk-identified leaf this frame). `val1`/`val2` are
/// then window-relative col/row for a grid window, or pixels for a graphics
/// window.
///
/// The hit-test is against the DRAWN rect, not gvm's own layout rect, and that
/// is the whole of SQ-1203: gvm reserves a 1-cell border gutter per bordered
/// split whether or not the theme draws a rule there (`upper_window_border`
/// defaults to `BorderStyle::None`, SQ-0821), so the two rects skew by every
/// collapsed gutter between the pane origin and the window. A click on a menu's
/// drawn top row landed one row above every gvm rect and was silently dropped.
/// `story = (x, y, w, h)` (the story-pane rect) is kept only as a fast
/// overlay-adjacent bounds check — every drawn rect already lies inside it.
///
/// `sub_px` is the click's offset WITHIN its cell, which a terminal only knows
/// under pixel mouse reporting (SQ-0563). With it, a graphics window hears the
/// exact pixel clicked — the only way to reach a button that occupies part of a
/// cell, like advent.blb's W/E compass buttons. Without it (`None`), the cell's
/// centre is reported: the player aimed at the middle of what the cell shows, and
/// a top-left corner could land one button up or left on a packed toolbar.
pub fn glk_mouse_target(
    overlay_open: bool,
    col: u16,
    row: u16,
    story: (u16, u16, u16, u16),
    requested: &[u32],
    win_rects: &[(u32, crate::engine::WinKind, ratatui::layout::Rect)],
    char_px: (u32, u32),
    sub_px: Option<(u16, u16)>,
) -> Option<(u32, u32, u32)> {
    if overlay_open {
        return None;
    }
    let (sx0, sy0, sw, sh) = story;
    if col < sx0 || col >= sx0 + sw || row < sy0 || row >= sy0 + sh {
        return None;
    }
    let pt = ratatui::layout::Position { x: col, y: row };
    let &(win, kind, rect) = win_rects
        .iter()
        .find(|&&(id, _, r)| requested.contains(&id) && r.contains(pt))?;
    let rel_x = (col - rect.x) as u32;
    let rel_y = (row - rect.y) as u32;
    let (vx, vy) = if kind == crate::engine::WinKind::Graphics {
        // The kitty render scales the canvas to exactly the window's cell rect
        // (`render_kitty_virtual`), so cells and canvas pixels stay in proportion
        // and a cell offset converts straight to a canvas pixel. Clamped inside
        // the cell in case the terminal's cell size and the one the game was told
        // ever disagree.
        let (ox, oy) = match sub_px {
            Some((x, y)) => (
                (x as u32).min(char_px.0.saturating_sub(1)),
                (y as u32).min(char_px.1.saturating_sub(1)),
            ),
            None => (char_px.0 / 2, char_px.1 / 2),
        };
        (rel_x * char_px.0 + ox, rel_y * char_px.1 + oy)
    } else {
        (rel_x, rel_y)
    };
    Some((win, vx, vy))
}

/// Decide which hyperlink-watching window owns a click on a linked transcript
/// cell at absolute `(col, row)`, if any.
///
/// Returns the window id only when no overlay is open, the window is currently
/// hyperlink-watching (`requested`, as reported by
/// [`GlulxSession::hyperlink_windows`]), and the click lands inside that
/// window's DRAWN rect (`win_rects`, same source and reasoning as
/// [`glk_mouse_target`] — SQ-1203). A hyperlink event carries the link value
/// from the cell→link map rather than coordinates, so only the window id is
/// returned.
pub fn glk_hyperlink_window(
    overlay_open: bool,
    col: u16,
    row: u16,
    story: (u16, u16, u16, u16),
    requested: &[u32],
    win_rects: &[(u32, crate::engine::WinKind, ratatui::layout::Rect)],
) -> Option<u32> {
    if overlay_open {
        return None;
    }
    let (sx0, sy0, sw, sh) = story;
    if col < sx0 || col >= sx0 + sw || row < sy0 || row >= sy0 + sh {
        return None;
    }
    let pt = ratatui::layout::Position { x: col, y: row };
    win_rects
        .iter()
        .find(|&&(id, _, r)| requested.contains(&id) && r.contains(pt))
        .map(|&(id, _, _)| id)
}

/// Build a name-based room snapshot from an Inform room heading. Glulx has no
/// readable object tree, so identity is the synthetic id of the normalized name.
fn heading_to_room(name: &str) -> LocationInfo {
    zvm::ObjectSnapshot {
        number: crate::roomid::synthetic_room_id(name),
        parent: 0,
        name: name.to_string(),
    }
}

/// The empty initial screen snapshot.
fn blank_screen() -> ScreenModel {
    ScreenModel {
        root: WinNode::Blank,
        status: StatusModel::HostManaged,
        bg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),
        fg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),
        content_size: (0, 0),
    }
}

impl Engine for GlulxSession {
    fn submit(&mut self, command: &str) -> TurnResult {
        // A new command turn re-starts per-turn execution coverage (the `|` gutter
        // + last-turn set); the cumulative `ever_executed` is preserved. Mirrors
        // the Z-machine engine's per-turn clear chokepoint.
        if self.machine.trace_exec {
            self.machine.executed_pcs.clear();
        }
        if !self.quit {
            self.machine.supply_line(command);
            self.drive_turn();
        }
        self.finish_turn()
    }

    fn submit_key(&mut self, key: KeyInput) -> Option<TurnResult> {
        let code = key_to_glk(key)?;
        if self.machine.trace_exec {
            self.machine.executed_pcs.clear();
        }
        if !self.quit {
            self.machine.supply_char(code);
            self.drive_turn();
        }
        Some(self.finish_turn())
    }

    fn take_transcript(&mut self) -> String {
        self.machine.flush();
        let raw = self.appglk().take_transcript().0;
        if self.strip_prompt { strip_read_prompt(&raw).to_owned() } else { raw }
    }

    fn drain_screen_clear(&mut self) -> bool {
        self.appglk().take_primary_cleared()
    }

    fn take_transcript_elems(&mut self) -> Vec<TranscriptElem> {
        // Ordered banner/startup drain: text runs + any inline images the game
        // drew before the first turn (title/cover art). Mirrors `finish_turn`'s
        // trailing-read-prompt handling so the returned elements stay consistent
        // with the flat `take_transcript()` string: the concatenation of the
        // returned `Text` equals `strip_read_prompt(raw)` (or `raw` unchanged
        // when `strip_prompt` is false) — same gating as `take_transcript`.
        self.machine.flush();
        let mut elems = self.appglk().take_transcript_elems();
        let mut raw = String::new();
        for e in &elems {
            if let TranscriptElem::Text { text, .. } = e {
                raw.push_str(text);
            }
        }
        let kept = if self.strip_prompt { strip_read_prompt(&raw).chars().count() } else { raw.chars().count() };
        trim_elems_to_len(&mut elems, kept);
        elems
    }

    fn set_strip_prompt(&mut self, on: bool) {
        self.strip_prompt = on;
    }

    fn pending_input(&self) -> InputKind {
        self.pending
    }

    fn resume_save(&mut self, wrote_ok: bool) -> TurnResult {
        // The host wrote (or failed to write) the save file; deliver the result
        // to the suspended @save and run to the next input request.
        self.machine.complete_save(wrote_ok);
        self.pending_io = None;
        self.drive_turn();
        self.apply_deferred_resize();
        self.finish_turn()
    }

    fn resume_restore(&mut self, data: Option<&[u8]>) -> TurnResult {
        // `Some(bytes)` = the player picked a save; `None` = cancelled. Corrupt
        // bytes fall back to failure so the game sees a clean failure result.
        // Uses `complete_restore_quetzal` (stub-based, live-state-preserving),
        // matching the bare standard `.qzl` that `@save` now writes — NOT
        // `complete_restore_success`, which expects the full host-snapshot
        // format (Save State only).
        match data {
            Some(bytes) if self.machine.complete_restore_quetzal(bytes) => {}
            _ => self.machine.complete_restore_failure(),
        }
        self.pending_io = None;
        self.drive_turn();
        self.apply_deferred_resize();
        self.finish_turn()
    }

    fn pending_filename(&self) -> Option<FilenameReq> {
        self.pending_filename
    }

    fn file_names(&self) -> Vec<String> {
        self.machine.file_names()
    }

    fn resume_filename(&mut self, name: Option<String>) -> TurnResult {
        // `Some` = the player chose/entered a name; `None` = cancelled (NULL fileref).
        self.machine.supply_filename(name);
        self.pending_filename = None;
        self.drive_turn();
        self.apply_deferred_resize();
        self.finish_turn()
    }

    fn has_quit(&self) -> bool {
        self.quit
    }

    fn screen(&self) -> ScreenModel {
        self.screen_cache.clone()
    }

    fn window_dump(&self) -> Vec<String> {
        self.window_dump_cache.clone()
    }

    fn save_state(&self) -> EngineSave {
        EngineSave::new(GLULX_ENGINE, GLULX_SAVE_FORMAT, self.machine.save_state())
    }

    fn restore_state(&mut self, save: &EngineSave) -> Result<(), EngineError> {
        if !save.is_engine(GLULX_ENGINE) {
            return Err(EngineError::EngineMismatch {
                expected: GLULX_ENGINE.to_string(),
                found: save.engine.clone(),
            });
        }
        // Put the session back at a clean input prompt BEFORE the swap when a game
        // `@save`/`@restore`/`create_by_prompt` is suspended on a dialog. The saves
        // manager reaches here from exactly that state: the game asked to RESTORE,
        // the player answered the dialog with a host Save State instead.
        //
        // This is not tidiness. A gvm snapshot's PC sits just PAST the `glk_select`
        // it was taken at (unlike a Quetzal save, whose PC the Z-machine rewinds to
        // the `read` itself), so the restored state only makes sense paired with a
        // LIVE suspension for the host to resolve. Restored into a session with no
        // suspension, the game resumes past the select and re-reads the `event_t`
        // the snapshot left in RAM — replaying the snapshot's last command as a
        // whole extra turn, moving the player or taking an object twice over.
        // Answering the doomed op costs nothing (the state it belongs to is about to
        // be discarded) and leaves the VM parked at a select. (SQ-0656)
        //
        // Deliberately ahead of the blob check, unlike `restore_game_save`: should
        // the snapshot turn out to be unreadable, the run loop answers the request
        // it thinks is still open with `resume_restore(None)`, which is now a no-op
        // that re-arms the prompt — so a rejected blob still leaves a playable
        // session at the game's own "Failed." rather than a suspended one.
        if self.game_io_pending() {
            if let Some(req) = self.machine.pending_saveload_request() {
                if req.restore {
                    self.machine.complete_restore_failure();
                } else {
                    self.machine.complete_save(false);
                }
            }
            if self.pending_filename.take().is_some() {
                self.machine.supply_filename(None);
            }
            let _ = drive_settled(&mut self.machine, &self.store);
            // The abandoned verb's tail ("Failed.") describes a run that is being
            // replaced and would land above the archive's own restored scrollback.
            let _ = self.take_transcript_elems();
        }
        self.machine
            .restore_state(&save.bytes)
            .map_err(|e| EngineError::BadSave(format!("{e:?}")))?;
        // The restore swapped RAM wholesale, and this path does not drive a
        // turn first — drop the cached object-word set as `drive_turn` does, or
        // it keeps answering for the session we just left (SQ-1176). The
        // `parse_names` layout survives: same story, same compiler tables.
        self.object_word_set.take();
        // Nothing the previous run was waiting on survives the swap.
        // `Machine::restore_state` drops the VM-side suspensions (SQ-0656); these
        // are the host-side halves of the same records, and leaving them set would
        // have the run loop re-open a dialog for a call that no longer exists.
        self.pending_io = None;
        self.pending_filename = None;
        // Re-derive the input mode and the quit flag from the state just installed,
        // uniformly with `GameSession::restore_state` (session.rs): a snapshot taken
        // at a "press any key" prompt may be restored while this session sits at a
        // line prompt, and `pending` is what the app renders its input bar from.
        // The machine is parked at its select (guaranteed by the block above), so
        // this re-reports the restored suspension without executing anything.
        let (pending, quit) = drive_settled(&mut self.machine, &self.store);
        self.pending = pending;
        self.quit = quit;
        // A size queued while the dialog was open would otherwise be stranded: the
        // host's resize poller has already recorded it as delivered.
        self.apply_deferred_resize();
        self.refresh_screen();
        Ok(())
    }

    fn restore_game_save(&mut self, bytes: &[u8]) -> Result<(), EngineError> {
        // A HOST restore of in-game `@save` bytes: the saves-manager Load, the
        // `/restore-state` command, or a bare `.glksave`/`.qzl` carried in from
        // another interpreter. Uniform in-game-save semantics say every engine's
        // `@save` archive must load here, not just through the game's own
        // `@restore` (SQ-0556) — and the bytes are the same standard
        // Glulx-Quetzal either way, so the semantics are too:
        // `complete_restore_quetzal` reverts RAM/stack/heap to the save point,
        // pops `@save`'s call stub and stores the `-1` "just restored" sentinel,
        // and — per Glulx spec §1.8.5 — leaves the LIVE Glk model alone rather
        // than reinstating a serialized one. The bare format has no `Glk ` chunk
        // to reinstate a stale window tree from, which is exactly why it is the
        // right shape to seal for `@save`.
        //
        // Runs before anything is abandoned: a foreign or corrupt blob fails on
        // the `IFhd` identity check with the session still intact.
        if !self.machine.complete_restore_quetzal(bytes) {
            return Err(EngineError::BadSave(
                "not a Glulx save for this story (or the file is corrupt)".into(),
            ));
        }
        // The one thing the game's own `@restore` never faces: this arrives
        // while the session is parked inside a `glk_select` belonging to the run
        // we just left. §1.8.5 keeps that suspension out of the save file, so
        // the host has to retire it — otherwise `step` re-reports it forever
        // instead of resuming at the restored PC. The game re-requests its own
        // line event on the way back to the prompt.
        self.machine.abandon_pending_input();
        // RAM was reverted to the save point without a turn being driven — same
        // duty as in `restore_state` above (SQ-1176).
        self.object_word_set.take();
        // Run the save-verb tail out to the next prompt, so the session is
        // re-armed at a clean input request rather than parked mid-verb
        // (mirrors the Z-machine's `restore_game_save`).
        let (pending, quit) = drive_settled(&mut self.machine, &self.store);
        self.pending = pending;
        self.quit = quit;
        self.pending_io = None;
        self.pending_filename = None;
        // That tail's output is redundant with the host's own "[Game restored]"
        // notice, and an `@save` archive is about to reinstate its own
        // transcript over the top of it — drain and discard.
        let _ = self.take_transcript_elems();
        self.apply_deferred_resize(); // as in `restore_state`
        self.refresh_screen();
        Ok(())
    }

    fn is_saveload_pending(&self) -> bool {
        self.machine.is_saveload_pending()
    }

    fn aux_data(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.aux
    }

    fn set_aux_data(&mut self, data: BTreeMap<String, Vec<u8>>) {
        self.aux = data;
    }

    fn aux_dirty(&self) -> bool {
        self.aux_dirty
    }

    fn clear_aux_dirty(&mut self) {
        self.aux_dirty = false;
    }

    fn vfs_bytes(&self) -> Vec<u8> {
        self.machine.vfs_bytes()
    }

    fn load_vfs(&mut self, bytes: &[u8]) {
        self.machine.load_vfs(bytes);
    }

    fn vfs_dirty(&self) -> bool {
        self.machine.vfs_dirty()
    }

    fn clear_vfs_dirty(&mut self) {
        self.machine.clear_vfs_dirty();
    }

    fn current_location(&self) -> Option<LocationInfo> {
        self.last_room.clone()
    }

    fn set_trace_screen(&mut self, on: bool) {
        self.machine.trace_screen = on;
    }

    fn take_screen_trace(&mut self) -> Vec<String> {
        std::mem::take(&mut self.machine.screen_trace)
    }

    fn set_debug_trace(&mut self, on: bool) {
        self.machine.trace_exec = on;
        // Only the per-turn set is cleared when tracing stops; the cumulative
        // `ever_executed` (permanent colour + persisted coverage) is preserved.
        if !on {
            self.machine.executed_pcs.clear();
        }
    }

    fn seed_executed_pcs(&mut self, pcs: &std::collections::HashSet<u32>) {
        self.machine.seed_ever_executed(pcs);
    }

    /// The story's grammar and dictionary, as the engine-neutral snapshot.
    ///
    /// `None` when the Glulx grammar tables cannot be located — there is
    /// nothing to offer from a table we could not read. Both dictionary record
    /// shapes read, byte-valued and Unicode (SQ-1231).
    fn story_vocabulary(&self) -> Option<crate::vocab::StoryVocabulary> {
        let mem = self.machine.mem();
        let grammar = gvm::grammar::Grammar::load(mem).ok()?;
        let words = grammar
            .words()
            .map(|w| (w.to_string(), grammar.roles(w).unwrap_or_default()))
            .collect();
        let preps = grammar.prepositions().iter().cloned().collect();
        // Glulx truncates by plain characters, so `DICT_WORD_SIZE` is a
        // character count and the snapshot's own `knows` is exact.
        let key_len = grammar.tables().dict_word_size as usize;
        Some(crate::vocab::StoryVocabulary::new(grammar.verbs().to_vec(), words, preps, key_len))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn debugger(&self) -> Option<&dyn crate::engine::Debugger> {
        Some(self)
    }

    // introspect() uses the trait default (None): the tree questions — contents,
    // room objects, children, the player — need handles this adapter cannot
    // correlate with its synthetic heading-keyed rooms, and answering them with
    // empty lists would turn "could not ask" into a false "asked, nothing there"
    // for every consumer that tells those apart (see `Engine::object_word_set`'s
    // doc). The one object question Glulx CAN answer gets its own seam below.

    /// "Does ANY object answer to this word", from the story's own Inform
    /// object list — `gvm::objects::ParseNames`, the same walk the Z-machine
    /// side does through `zvm::objects` (SQ-1210). Fail-safe by construction:
    /// detection refuses (`None`) unless a verified `$70` list with readable
    /// `name` arrays exists, and every consumer then keeps its documented
    /// dictionary fallback.
    fn object_word_set(&self) -> Option<std::sync::Arc<grammar_model::ObjectWordSet>> {
        // Whether the story keeps a readable object list is a compile-time
        // layout fact (`parse_names` is a `OnceCell`), so a `None` needs no
        // cache.
        let names = self.parse_names()?;
        if let Some(set) = self.object_word_set.borrow().as_ref() {
            return Some(std::sync::Arc::clone(set));
        }
        // One walk of the object list per TURN, not per token: every drive path
        // drops the entry (see the field's doc), because the `name` arrays live
        // in RAM and a game can rewrite them.
        let set = std::sync::Arc::new(grammar_model::ObjectWordSet::build(
            &names.all(self.machine.mem()),
        ));
        *self.object_word_set.borrow_mut() = Some(std::sync::Arc::clone(&set));
        Some(set)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;
    use crate::engine::WinNode;
    use gvm::glk::keycode;

    // ── A tiny Glulx instruction encoder (mirrors gvm-cli's test encoder) ──────

    #[derive(Clone, Copy)]
    enum E {
        Imm(u32),
        LocLoad(u8),
        LocStore(u8),
        MemLoad(u32),
        Push,
        Discard,
    }
    fn emode(e: E) -> u8 {
        match e {
            E::Imm(_) => 3,
            E::LocLoad(_) | E::LocStore(_) => 9,
            E::MemLoad(_) => 7,
            E::Push => 8,
            E::Discard => 0,
        }
    }
    fn edata(e: E) -> Vec<u8> {
        match e {
            E::Imm(v) | E::MemLoad(v) => v.to_be_bytes().to_vec(),
            E::LocLoad(o) | E::LocStore(o) => vec![o],
            E::Push | E::Discard => vec![],
        }
    }
    fn enc(op: u32, args: &[E]) -> Vec<u8> {
        let mut out = Vec::new();
        if op <= 0x7f {
            out.push(op as u8);
        } else {
            out.extend_from_slice(&((op | 0x8000) as u16).to_be_bytes());
        }
        let mut modes = vec![0u8; args.len().div_ceil(2)];
        for (i, &a) in args.iter().enumerate() {
            let m = emode(a);
            if i % 2 == 0 {
                modes[i / 2] |= m;
            } else {
                modes[i / 2] |= m << 4;
            }
        }
        out.extend_from_slice(&modes);
        for &a in args {
            out.extend(edata(a));
        }
        out
    }

    // RAM layout (RAMSTART 0x400, ENDMEM 0x500): the event struct, the
    // line-input length word (event.val1), and the line buffer.
    const EVENT: u32 = 0x400;
    const VAL1: u32 = 0x408;
    const LINEBUF: u32 = 0x480;

    /// Wrap `body` as a start function with `nlocals` 4-byte locals into a
    /// runnable image (RAMSTART 0x400, ENDMEM 0x500 → RAM [0x400, 0x500) holds
    /// the event struct + line buffer; code lives in ROM [0x24, 0x400)).
    fn image_for(body: Vec<u8>, nlocals: u8) -> Vec<u8> {
        let mut func = vec![0xC1u8, 0x04, nlocals, 0x00, 0x00]; // type C1; nlocals 4-byte
        func.extend(body);
        let (ramstart, endmem) = (0x400u32, 0x500u32);
        let mut img = vec![0u8; ramstart as usize];
        img[0..4].copy_from_slice(b"Glul");
        img[0x04..0x08].copy_from_slice(&0x0003_0102u32.to_be_bytes());
        img[0x08..0x0C].copy_from_slice(&ramstart.to_be_bytes());
        img[0x0C..0x10].copy_from_slice(&ramstart.to_be_bytes());
        img[0x10..0x14].copy_from_slice(&endmem.to_be_bytes());
        img[0x14..0x18].copy_from_slice(&0x1000u32.to_be_bytes());
        img[0x18..0x1C].copy_from_slice(&0x24u32.to_be_bytes());
        img[0x24..0x24 + func.len()].copy_from_slice(&func);
        img
    }

    /// Open a TextBuffer (id → local0) and make it current.
    fn open_buffer_prelude() -> Vec<u8> {
        use E::*;
        let mut b = enc(0x149, &[Imm(2), Imm(0)]); // setiosys glk
        for v in [Imm(0), Imm(3), Imm(0), Imm(0), Imm(0)] {
            b.extend(enc(0x40, &[v, Push])); // rock, wintype=3, size, method, split
        }
        b.extend(enc(0x130, &[Imm(0x23), Imm(5), LocStore(0)])); // window_open → local0
        b.extend(enc(0x40, &[LocLoad(0), Push]));
        b.extend(enc(0x130, &[Imm(0x2f), Imm(1), Discard])); // set_window(local0)
        b
    }

    fn streamchar(c: u8) -> Vec<u8> {
        vec![0x70, 0x01, c] // streamchar (opcode 0x70; mode C8 small const)
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn vfs_delegation_load_and_bytes_roundtrip_through_the_machine() {
        // A trivial quit program yields a valid (settled) session; the VFS
        // methods operate on the machine, not on running game code. `machine.glk`
        // is pub(crate) to gvm, so a session-level test cannot originate a file
        // write from the app crate — the dirty-on-write path is covered at the
        // gvm layer (`vfs_dirty_tracks_mutations`, `machine_vfs_roundtrip`). Here
        // we assert the app-side delegation: `load_vfs` populates the machine VFS
        // and `vfs_bytes` re-encodes it, using gvm's public sidecar codec.
        let mut sess =
            GlulxSession::new(image_for(enc(0x120, &[]), 1), 80, 24, true, false, false, (1, 1), None, &[])
                .expect("new");
        assert!(!sess.vfs_dirty(), "a fresh session's VFS is not dirty");
        assert!(
            gvm::glk::decode_files(&sess.vfs_bytes()).is_empty(),
            "a fresh session has no persisted files"
        );

        let mut files = std::collections::BTreeMap::new();
        files.insert("scores".to_string(), b"42".to_vec());
        sess.load_vfs(&gvm::glk::encode_files(&files));
        // Loading a sidecar is not a game mutation.
        assert!(!sess.vfs_dirty(), "loading the sidecar does not dirty the VFS");

        let out = gvm::glk::decode_files(&sess.vfs_bytes());
        assert_eq!(
            out.get("scores").map(Vec::as_slice),
            Some(b"42".as_slice()),
            "the loaded file survives a vfs_bytes round-trip through the session"
        );
    }

    #[test]
    fn new_loads_vfs_sidecar_before_boot() {
        // SQ-0290: a Glulx game may read/write a Glk file during BOOT (e.g. CM's
        // init cache). GlulxSession::new must load the sidecar into the VM's VFS
        // BEFORE driving the boot, so the running game sees it and any boot-time
        // write persists. Pin that ordering guarantee: a non-empty sidecar passed
        // to new() is present in the session's VFS immediately after construction.
        let mut files = std::collections::BTreeMap::new();
        files.insert("cache".to_string(), b"init-data".to_vec());
        let sidecar = gvm::glk::encode_files(&files);

        let sess = GlulxSession::new(
            simple_line_image(), 80, 24, true, false, false, (1, 1), None, &sidecar,
        )
        .expect("new");

        let out = gvm::glk::decode_files(&sess.vfs_bytes());
        assert_eq!(
            out.get("cache").map(Vec::as_slice),
            Some(b"init-data".as_slice()),
            "the sidecar VFS entry must be loaded into the VM before boot"
        );
    }

    #[test]
    fn new_in_seeds_on_disk_saved_game_slot_before_boot() {
        use E::*;
        // SQ-0301: a create_by_name SavedGame slot's save lives in a host
        // <game_dir>/<name>.qzl file, decoupled from the VFS. The machine's
        // existence index is session-transient, so new_in must reseed it from
        // disk BEFORE booting — else a game probing glk_fileref_does_file_exist
        // during init never sees its own on-disk save across launches. Probe:
        // create_by_name("foo", SavedGame) then streamnum(does_file_exist) — the
        // banner shows "1" only if the on-disk foo.qzl was seeded.
        const NAMEADDR: u32 = 0x420; // free RAM; NAMEADDR+3 is already NUL

        let mut body = open_buffer_prelude(); // buffer window -> loc0
        for (i, &c) in b"foo".iter().enumerate() {
            body.extend(enc(0x4E, &[Imm(NAMEADDR), Imm(i as u32), Imm(c as u32)])); // astoreb
        }
        // fref = glk_fileref_create_by_name(usage=1 SavedGame, NAMEADDR, rock=0) -> loc4.
        body.extend(enc(0x40, &[Imm(0), Push])); // rock
        body.extend(enc(0x40, &[Imm(NAMEADDR), Push])); // nameptr
        body.extend(enc(0x40, &[Imm(1), Push])); // usage (arg0, topmost)
        body.extend(enc(0x130, &[Imm(0x61), Imm(3), LocStore(4)]));
        // exists = glk_fileref_does_file_exist(fref) -> loc8.
        body.extend(enc(0x40, &[LocLoad(4), Push]));
        body.extend(enc(0x130, &[Imm(0x67), Imm(1), LocStore(8)]));
        body.extend(enc(0x71, &[LocLoad(8)])); // streamnum -> prints "1"/"0" to the window
        body.extend(line_prompt());
        body.extend(enc(0x120, &[])); // quit

        // Tag distinct from config.rs's `bm-seed-…`: same binary, same pid when the
        // tests run in threads, and both wipe the dir before use.
        let dir = std::env::temp_dir().join(format!("bm-glulx-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp game_dir");
        std::fs::write(dir.join("foo.qzl"), b"pretend-save-bytes").expect("write foo.qzl");

        let mut sess = GlulxSession::new_in(
            dir.clone(), image_for(body, 3), 80, 24, true, false, false, false, (1, 1), None, &[],
            [[(None, None); 11]; 2], false, None,
        )
        .expect("new_in");
        assert_eq!(sess.pending_input(), InputKind::Line);
        assert_eq!(
            sess.take_transcript(),
            "1",
            "an on-disk .qzl slot is seeded before boot, so does_file_exist reports true across launches",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_to_glk_maps_special_keys_and_chars() {
        assert_eq!(key_to_glk(KeyInput::Char('z')), Some(b'z' as u32));
        assert_eq!(key_to_glk(KeyInput::Enter), Some(keycode::RETURN));
        assert_eq!(key_to_glk(KeyInput::Backspace), Some(keycode::DELETE));
        assert_eq!(key_to_glk(KeyInput::Delete), Some(keycode::DELETE));
        assert_eq!(key_to_glk(KeyInput::Tab), Some(keycode::TAB));
        assert_eq!(key_to_glk(KeyInput::Escape), Some(keycode::ESCAPE));
        assert_eq!(key_to_glk(KeyInput::Up), Some(keycode::UP));
        assert_eq!(key_to_glk(KeyInput::Down), Some(keycode::DOWN));
        assert_eq!(key_to_glk(KeyInput::Left), Some(keycode::LEFT));
        assert_eq!(key_to_glk(KeyInput::Right), Some(keycode::RIGHT));
        assert_eq!(key_to_glk(KeyInput::Home), Some(keycode::HOME));
        assert_eq!(key_to_glk(KeyInput::End), Some(keycode::END));
        assert_eq!(key_to_glk(KeyInput::PageUp), Some(keycode::PAGE_UP));
        assert_eq!(key_to_glk(KeyInput::PageDown), Some(keycode::PAGE_DOWN));
        assert_eq!(key_to_glk(KeyInput::Func(1)), Some(keycode::FUNC1));
        assert_eq!(key_to_glk(KeyInput::Func(12)), Some(keycode::FUNC12));
        // Keys with no Glk meaning are skipped.
        assert_eq!(key_to_glk(KeyInput::Insert), None);
        assert_eq!(key_to_glk(KeyInput::Func(13)), None);
    }

    #[test]
    fn submit_echoes_line_with_runs_and_screen_is_two_window_tree() {
        use E::*;
        // Build the program inline (clearer than the helper).
        let mut body = open_buffer_prelude();
        for v in [Imm(0), Imm(4), Imm(1), Imm(0x12), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0x23), Imm(5), LocStore(4)])); // grid → local1
        body.extend(enc(0x40, &[LocLoad(0), Push]));
        body.extend(enc(0x130, &[Imm(0x2f), Imm(1), Discard])); // set_window(buffer)
        body.extend(streamchar(b'O'));
        body.extend(streamchar(b'K'));
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        // Header-styled 'Y'.
        body.extend(enc(0x40, &[Imm(3), Push]));
        body.extend(enc(0x130, &[Imm(0x86), Imm(1), Discard])); // set_style(Header)
        body.extend(streamchar(b'Y'));
        body.extend(enc(0x40, &[Imm(0), Push]));
        body.extend(enc(0x130, &[Imm(0x86), Imm(1), Discard])); // set_style(Normal)
        // Echo the typed line: put_buffer(0x180, len=mem[0x108]).
        body.extend(enc(0x40, &[MemLoad(VAL1), Push])); // len
        body.extend(enc(0x40, &[Imm(LINEBUF), Push])); // addr
        body.extend(enc(0x130, &[Imm(0x84), Imm(2), Discard])); // glk_put_buffer
        body.extend(enc(0x120, &[])); // quit

        let mut sess = GlulxSession::new(image_for(body, 2), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Line);
        // Banner drained.
        assert_eq!(sess.take_transcript(), "OK");
        // screen() is a grid-over-buffer pair.
        let model = sess.screen();
        match &model.root {
            WinNode::Pair { vertical, first, second, .. } => {
                assert!(*vertical);
                assert!(matches!(**first, WinNode::Grid(_)));
                assert!(matches!(**second, WinNode::Buffer(_)));
            }
            other => panic!("expected a grid-over-buffer Pair, got {other:?}"),
        }
        assert!(model.grid().is_some());

        // Submit a line → "Y" (bold) then the echoed "hi"; the program quits.
        let r = sess.submit("hi");
        assert_eq!(r.transcript, "Yhi");
        assert!(r.quit);
        assert!(
            r.transcript_runs.iter().any(|&(_, bits, _, _, _, _, _, _)| bits == 0x02),
            "the Header-styled 'Y' contributes a bold run: {:?}",
            r.transcript_runs
        );
    }

    /// A line-input prompt: request_line_event(LINEBUF, 20) then glk_select.
    fn line_prompt() -> Vec<u8> {
        use E::*;
        let mut b = Vec::new();
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            b.extend(enc(0x40, &[v, Push]));
        }
        b.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        b.extend(enc(0x40, &[Imm(EVENT), Push]));
        b.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        b
    }

    #[test]
    fn ingame_save_restore_round_trips_through_the_engine() {
        use E::*;
        // RAM scratch for the @save/@restore result descriptors (mode 7 = store to
        // a 4-byte main-memory address; `MemLoad` reuses that mode for stores).
        const SAVE_RES: u32 = 0x410;
        const RESTORE_RES: u32 = 0x414;
        let mut body = open_buffer_prelude();
        body.extend(line_prompt()); // turn 1 prompt
        body.extend(enc(0x123, &[Imm(0), MemLoad(SAVE_RES)])); // @save -> mem[SAVE_RES]
        body.extend(line_prompt()); // turn 2 prompt (resume point after a restore)
        body.extend(enc(0x124, &[Imm(0), MemLoad(RESTORE_RES)])); // @restore -> mem[RESTORE_RES]
        body.extend(enc(0x120, &[])); // quit

        let mut sess = GlulxSession::new(image_for(body, 1), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Line, "opens at the turn-1 prompt");

        // Turn 1: the command drives into @save, which bubbles a Save request.
        let r1 = sess.submit("save");
        assert_eq!(r1.pending_io, Some(PendingIo::Save), "@save bubbles a Save request");
        assert!(!r1.quit);

        // The run loop captures the snapshot bytes, writes them, then reports success.
        let blob = sess.save_state().bytes;
        let r2 = sess.resume_save(true);
        assert_eq!(r2.pending_io, None, "@save completes and runs on");
        assert!(!r2.quit);
        assert_eq!(sess.pending_input(), InputKind::Line, "reaches the turn-2 prompt");

        // Turn 2: the command drives into @restore, which bubbles a Restore request.
        let r3 = sess.submit("restore");
        assert_eq!(r3.pending_io, Some(PendingIo::Restore), "@restore bubbles a Restore request");

        // Feeding the saved bytes back restores to the save point (the turn-2
        // prompt) rather than falling through to the trailing quit.
        let r4 = sess.resume_restore(Some(&blob));
        assert!(!r4.quit, "a successful restore returns to the save point, not the quit");
        assert_eq!(r4.pending_io, None);
        assert_eq!(sess.pending_input(), InputKind::Line, "restored back to the turn-2 prompt");
    }

    /// SQ-0283 Task 6: the Glulx in-game save/restore round-trip through the
    /// bare standard `save_quetzal()` bytes (what the archive writer seals for an
    /// in-game trigger), NOT the full `save_state()` host snapshot. Also checks that the
    /// live Glk window survives the restore (unlike `restore_state`, which
    /// would replace it from a `Glk ` chunk, `restore_quetzal` never touches
    /// `self.glk` — §1.8.5).
    #[test]
    fn ingame_save_restore_round_trips_via_save_quetzal_and_keeps_the_live_window() {
        use E::*;
        const SAVE_RES: u32 = 0x410;
        const RESTORE_RES: u32 = 0x414;
        let mut body = open_buffer_prelude();
        body.extend(line_prompt()); // turn 1 prompt
        body.extend(enc(0x123, &[Imm(0), MemLoad(SAVE_RES)])); // @save -> mem[SAVE_RES]
        body.extend(line_prompt()); // turn 2 prompt (resume point after a restore)
        body.extend(enc(0x124, &[Imm(0), MemLoad(RESTORE_RES)])); // @restore -> mem[RESTORE_RES]
        body.extend(enc(0x120, &[])); // quit

        let mut sess = GlulxSession::new(image_for(body, 1), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Line, "opens at the turn-1 prompt");

        // A window is open (from open_buffer_prelude) before the save.
        assert!(!matches!(sess.screen().root, WinNode::Blank), "a window is open before @save");

        // Turn 1: the command drives into @save, which bubbles a Save request.
        let r1 = sess.submit("save");
        assert_eq!(r1.pending_io, Some(PendingIo::Save), "@save bubbles a Save request");
        assert!(!r1.quit);

        // The host seals these bytes into the archive's game.glksave entry.
        let blob = sess.save_quetzal();
        assert!(!blob.is_empty(), "save_quetzal produces bytes");

        let r2 = sess.resume_save(true);
        assert_eq!(r2.pending_io, None, "@save completes and runs on");
        assert!(!r2.quit);
        assert_eq!(sess.pending_input(), InputKind::Line, "reaches the turn-2 prompt");

        // Turn 2: the command drives into @restore, which bubbles a Restore request.
        let r3 = sess.submit("restore");
        assert_eq!(r3.pending_io, Some(PendingIo::Restore), "@restore bubbles a Restore request");

        // Feeding the save_quetzal bytes back (via resume_restore ->
        // complete_restore_quetzal) restores VM state to the save point (the
        // turn-2 prompt) rather than falling through to the trailing quit.
        let r4 = sess.resume_restore(Some(&blob));
        assert!(!r4.quit, "a successful restore returns to the save point, not the quit");
        assert_eq!(r4.pending_io, None);
        assert_eq!(sess.pending_input(), InputKind::Line, "restored back to the turn-2 prompt");

        // The live Glk window survives the restore (restore_quetzal never
        // touches self.glk) -- the screen tree still has an open window, and
        // the session can keep driving turns through it.
        assert!(!matches!(sess.screen().root, WinNode::Blank), "the live window survives restore_quetzal");
    }

    /// SQ-0531: Glulx under the unified writer. `@save` now writes the same
    /// `.lanthorn` archive a host Save State writes, but the bytes it seals
    /// inside must still be the BARE standard Glulx-Quetzal (`save_quetzal`) —
    /// not the host snapshot, whose `GReg`/`Glk ` chunks would replace the live
    /// display model on restore. The whole archive round-trips back through the
    /// game's own `@restore`.
    #[test]
    fn glulx_unified_writer_seals_bare_quetzal_for_ingame_and_the_snapshot_for_hoststate() {
        use crate::archive::SaveTrigger;
        use crate::persist_files::game_save_bytes;
        use E::*;
        const SAVE_RES: u32 = 0x410;
        const RESTORE_RES: u32 = 0x414;
        let mut body = open_buffer_prelude();
        body.extend(line_prompt());
        body.extend(enc(0x123, &[Imm(0), MemLoad(SAVE_RES)])); // @save
        body.extend(line_prompt());
        body.extend(enc(0x124, &[Imm(0), MemLoad(RESTORE_RES)])); // @restore
        body.extend(enc(0x120, &[])); // quit

        let mut sess =
            GlulxSession::new(image_for(body, 1), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert_eq!(sess.submit("save").pending_io, Some(PendingIo::Save));

        // The two triggers pick DIFFERENT bytes for Glulx — that is the whole
        // reason the writer consults the trigger rather than always snapshotting.
        let ingame = game_save_bytes(&sess, SaveTrigger::Ingame);
        let host = game_save_bytes(&sess, SaveTrigger::HostState);
        assert_eq!(ingame.bytes, sess.save_quetzal(), "@save seals the bare standard Glulx-Quetzal");
        assert_eq!(host.bytes, sess.save_state().bytes, "Save State seals the host snapshot");
        assert_ne!(ingame.bytes, host.bytes, "the two Glulx save shapes are genuinely different");
        assert_eq!(ingame.engine, host.engine, "both carry the same engine tag");

        // Write the @save archive exactly as `handle_save_as` does, then answer
        // the game's own @restore with what comes back out of it.
        let dir = std::env::temp_dir().join(format!("bm-sq0531-glulx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        crate::persist_files::save_named(
            &dir, "GLULX-TEST-0531", "slot", SaveTrigger::Ingame, &mapper::mapper::Mapper::default(),
            &ingame, None, &[], None, None, sess.aux_data(), 3, None, None, &crate::archive::SessionRecord::empty(),
        )
        .expect("save_named writes the Glulx archive");
        let path = dir.join("slot.lanthorn");

        // `game.glksave` — the Glulx interchange extension — holds those bytes verbatim.
        assert_eq!(
            crate::archive::read_quetzal_from_file(&path).expect("inner bytes"),
            ingame.bytes,
            "the archive's game.glksave is byte-identical to save_quetzal()"
        );
        assert_eq!(
            crate::archive::read_archive_meta(&path).unwrap().trigger,
            SaveTrigger::Ingame,
        );

        assert_eq!(sess.resume_save(true).pending_io, None);
        assert_eq!(sess.submit("restore").pending_io, Some(PendingIo::Restore));
        let bytes = crate::archive::read_quetzal_from_file(&path).expect("inner bytes");
        let r = sess.resume_restore(Some(&bytes));
        assert!(!r.quit, "restoring the archive returns to the save point, not the trailing quit");
        assert_eq!(sess.pending_input(), InputKind::Line, "restored back to the turn-2 prompt");
        assert!(!matches!(sess.screen().root, WinNode::Blank), "the live window survives");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A test story that saves mid-play and then keeps printing landmarks, so a
    /// restore is provable by what it prints next rather than by peeking at RAM:
    ///
    /// ```text
    /// prompt 1  ->  @save  ->  "A"  ->  prompt 2  ->  "B"  ->  prompt 3  ->  "C" -> quit
    /// ```
    ///
    /// A restore of the `@save` bytes resumes just past `@save`, so the run
    /// reprints "A" and parks at prompt 2 — and the NEXT command prints "B"
    /// again instead of "C". Nothing else can produce that sequence.
    fn save_then_landmarks_story() -> Vec<u8> {
        use E::*;
        const SAVE_RES: u32 = 0x410;
        let mut body = open_buffer_prelude();
        body.extend(line_prompt()); // prompt 1
        body.extend(enc(0x123, &[Imm(0), MemLoad(SAVE_RES)])); // @save
        body.extend(streamchar(b'A'));
        body.extend(line_prompt()); // prompt 2 — the save point's resume prompt
        body.extend(streamchar(b'B'));
        body.extend(line_prompt()); // prompt 3
        body.extend(streamchar(b'C'));
        body.extend(enc(0x120, &[])); // quit
        image_for(body, 1)
    }

    /// SQ-0556 — the quest itself. `@save` must behave the same on every engine,
    /// and "the same" includes being loadable from the SAVES MANAGER, not only
    /// through the game's own `@restore`. Glulx used to refuse outright
    /// (`BadSave("Glulx has no game-save (.qzl) format")`), which made it the one
    /// engine whose in-game save was a second-class citizen.
    ///
    /// The host restore takes the identical bytes the archive seals and gives
    /// them the identical semantics the game's own `@restore` gets — standard
    /// Glulx-Quetzal via `restore_quetzal`, live Glk model preserved (§1.8.5).
    #[test]
    fn glulx_ingame_save_archive_restores_through_the_host_path() {
        use crate::archive::SaveTrigger;
        use crate::persist_files::game_save_bytes;

        let mut sess = GlulxSession::new(save_then_landmarks_story(), 80, 24, true, false, false, (1, 1), None, &[])
            .expect("new");
        let _ = sess.take_transcript(); // drain the banner
        assert_eq!(sess.pending_input(), InputKind::Line, "opens at prompt 1");

        // Turn 1 drives into @save; the writer seals these bytes.
        assert_eq!(sess.submit("save").pending_io, Some(PendingIo::Save));
        let ingame = game_save_bytes(&sess, SaveTrigger::Ingame);
        assert_eq!(sess.resume_save(true).pending_io, None);

        // Play PAST the save point so a restore has something to undo.
        let r = sess.submit("onwards");
        assert!(r.transcript.contains('B'), "the run moved past the save point: {:?}", r.transcript);
        assert_eq!(sess.pending_input(), InputKind::Line, "parked at prompt 3");

        // The host restore that used to be a hard error.
        sess.restore_game_save(&ingame.bytes).expect("a Glulx @save archive loads through the host path");
        assert!(!sess.has_quit(), "the restore did not fall through to the trailing quit");
        assert_eq!(sess.pending_input(), InputKind::Line, "re-armed at an input prompt, not parked mid-verb");

        // Proof of WHERE we landed: the next command prints "B" (prompt 2 -> 3)
        // rather than "C" (prompt 3 -> quit). A restore that failed silently, or
        // that left the pre-restore `glk_select` suspension in place and
        // swallowed this command, cannot produce this.
        let after = sess.submit("again");
        assert!(after.transcript.contains('B'), "resumed from the save point: {:?}", after.transcript);
        assert!(!after.transcript.contains('C'), "did NOT continue from where we were: {:?}", after.transcript);
        assert!(!after.quit, "still playable after a host restore");
    }

    /// SQ-0556's real hazard, pinned. The bytes an `@save` seals must never be
    /// able to reinstate a STALE window model over a live one: Glulx §1.8.5 puts
    /// Glk library state (windows, streams, output stream, window contents,
    /// cursor positions) deliberately outside a save, and a host restore lands in
    /// a session whose windows the player is looking at right now.
    ///
    /// This guards the shape as well as the behaviour, because the tempting fix
    /// for this quest was to seal the richer `save_state()` (with its `GReg` +
    /// `Glk ` chunks) into the `@save` archive so it would be `restore_state`-able.
    /// That would have put a serialized window tree in the file — and this test
    /// fails the moment one appears there OR gets applied.
    #[test]
    fn glulx_host_restore_of_an_ingame_save_never_reinstates_a_stale_window_model() {
        use crate::archive::SaveTrigger;
        use crate::persist_files::game_save_bytes;
        use E::*;
        const SAVE_RES: u32 = 0x410;

        // prompt 1 -> @save -> prompt 2 -> split a status grid in -> prompt 3.
        let mut body = open_buffer_prelude();
        body.extend(line_prompt());
        body.extend(enc(0x123, &[Imm(0), MemLoad(SAVE_RES)])); // @save
        body.extend(line_prompt());
        // glk_window_open(split=local0, method=Above|Fixed, size=2, wintype=Grid, rock=0)
        for v in [Imm(0), Imm(4), Imm(2), Imm(0x12), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0x23), Imm(5), Discard]));
        body.extend(line_prompt());
        body.extend(enc(0x120, &[])); // quit

        let mut sess = GlulxSession::new(image_for(body, 1), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        let _ = sess.take_transcript();
        assert_eq!(sess.submit("save").pending_io, Some(PendingIo::Save));
        let ingame = game_save_bytes(&sess, SaveTrigger::Ingame);
        assert_eq!(sess.resume_save(true).pending_io, None);
        assert!(!matches!(sess.screen().root, WinNode::Pair { .. }), "one window at save time");

        // The shape half: the sealed bytes carry no serialized display state at
        // all, so there is nothing for a restore to reinstate.
        let has_chunk = |b: &[u8], id: &[u8; 4]| b.windows(4).any(|w| w == id);
        assert!(has_chunk(&ingame.bytes, b"IFhd"), "sealed bytes are Glulx-Quetzal");
        assert!(!has_chunk(&ingame.bytes, b"Glk "), "an @save archive must NOT carry a serialized window tree");
        assert!(!has_chunk(&ingame.bytes, b"GReg"), "an @save archive must NOT carry serialized registers");

        // The behaviour half: the game splits a second window in AFTER the save,
        // and the host restore must leave it standing.
        sess.submit("split");
        assert!(matches!(sess.screen().root, WinNode::Pair { .. }), "the game split a status window in after the save");

        sess.restore_game_save(&ingame.bytes).expect("host restore of the @save archive");
        assert!(
            matches!(sess.screen().root, WinNode::Pair { .. }),
            "the LIVE window model survives the restore; a stale one-window tree was not applied"
        );
        assert_eq!(sess.pending_input(), InputKind::Line, "still at a prompt with the live windows");
    }

    /// SQ-0556: a foreign/corrupt blob must still be refused — and refused
    /// CLEANLY, leaving the session playable rather than half-restored. The
    /// identity check runs before anything is abandoned for exactly this.
    #[test]
    fn glulx_host_restore_refuses_a_foreign_game_save_and_leaves_the_session_playable() {
        let mut sess = GlulxSession::new(save_then_landmarks_story(), 80, 24, true, false, false, (1, 1), None, &[])
            .expect("new");
        let _ = sess.take_transcript();

        assert!(sess.restore_game_save(b"not a Glulx save at all").is_err(), "garbage is refused");
        // Another story's save: same container, different IFhd.
        let other = {
            use E::*;
            let mut b = open_buffer_prelude();
            b.extend(line_prompt());
            b.extend(enc(0x123, &[Imm(0), MemLoad(0x410)]));
            b.extend(enc(0x120, &[]));
            let mut o = GlulxSession::new(image_for(b, 2), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
            assert_eq!(o.submit("save").pending_io, Some(PendingIo::Save));
            o.save_quetzal()
        };
        assert!(sess.restore_game_save(&other).is_err(), "another story's save is refused");

        // The refusals cost the session nothing: it still plays from where it was.
        assert_eq!(sess.pending_input(), InputKind::Line, "still parked at its own prompt");
        assert_eq!(sess.submit("save").pending_io, Some(PendingIo::Save), "still drives into its own @save");
    }

    #[test]
    fn game_managed_save_is_serviced_silently_to_game_dir_without_bubbling() {
        use E::*;
        // A create_by_name (game's OWN, not player-prompted) SavedGame @save must
        // be written silently to <game_dir>/<name>.qzl during the drive — no
        // Save request bubbled to the host UI. This is CM's init-cache path.
        const SAVE_RES: u32 = 0x410;
        const NAMEADDR: u32 = 0x420; // free RAM; the byte after is already NUL
        let dir = std::env::temp_dir().join(format!("bm-gameauto-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mut body = open_buffer_prelude();
        // Write the C-string "c" at NAMEADDR (astoreb; mem[NAMEADDR+1] is 0 already).
        body.extend(enc(0x4E, &[Imm(NAMEADDR), Imm(0), Imm(b'c' as u32)]));
        // fref = glk_fileref_create_by_name(usage=1 SavedGame, NAMEADDR, rock=0) -> local0.
        body.extend(enc(0x40, &[Imm(0), Push])); // rock
        body.extend(enc(0x40, &[Imm(NAMEADDR), Push])); // nameptr
        body.extend(enc(0x40, &[Imm(1), Push])); // usage (arg0, topmost)
        body.extend(enc(0x130, &[Imm(0x61), Imm(3), LocStore(0)]));
        // str = glk_stream_open_file(fref, fmode=1 Write, rock=0) -> local1.
        body.extend(enc(0x40, &[Imm(0), Push])); // rock
        body.extend(enc(0x40, &[Imm(1), Push])); // fmode Write
        body.extend(enc(0x40, &[LocLoad(0), Push])); // fref (arg0)
        body.extend(enc(0x130, &[Imm(0x42), Imm(3), LocStore(4)]));
        // @save str -> mem[SAVE_RES]: game-managed, serviced silently at boot.
        body.extend(enc(0x123, &[LocLoad(4), MemLoad(SAVE_RES)]));
        body.extend(line_prompt());
        body.extend(enc(0x120, &[])); // quit

        let sess = GlulxSession::new_in(
            dir.clone(), image_for(body, 2), 80, 24, true, false, false, false, (1, 1), None, &[],
            [[(None, None); 11]; 2], false, None,
        )
        .expect("new");
        // No Save request reached the host: the boot drive ran straight to the prompt.
        assert_eq!(sess.pending_input(), InputKind::Line, "the game-managed @save did not bubble");
        // The fixed-name slot was written to <dir>/c.qzl.
        let bytes = std::fs::read(dir.join("c.qzl")).expect("game-managed @save wrote c.qzl silently");
        assert!(!bytes.is_empty(), "the saved slot carries the save bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ingame_create_by_prompt_bubbles_filename_request_and_resume_continues() {
        use E::*;
        const FREF_RES: u32 = 0x410;
        let mut body = open_buffer_prelude();
        body.extend(line_prompt()); // turn 1 prompt
        // glk_fileref_create_by_prompt(usage=0, fmode=1 Write, rock=0) -> mem[FREF_RES].
        // @glk pops args with arg[0] topmost, so push rock, then fmode, then usage.
        body.extend(enc(0x40, &[Imm(0), Push])); // rock
        body.extend(enc(0x40, &[Imm(1), Push])); // fmode = Write
        body.extend(enc(0x40, &[Imm(0), Push])); // usage (topmost = arg[0])
        body.extend(enc(0x130, &[Imm(0x62), Imm(3), MemLoad(FREF_RES)]));
        body.extend(line_prompt()); // resume point after supply_filename
        body.extend(enc(0x120, &[])); // quit
        let mut sess = GlulxSession::new(image_for(body, 1), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Line, "opens at the turn-1 prompt");

        // The command drives into create_by_prompt, which bubbles a filename request.
        let r1 = sess.submit("script");
        assert_eq!(sess.pending_filename(), Some(FilenameReq { usage: 0, fmode: 1 }));
        assert!(r1.pending_io.is_none(), "a filename request is not a save/restore");
        assert!(!r1.quit);

        // The host supplies a name; execution resumes to the next prompt (proving the
        // @glk stored a value and did not fault or wedge).
        let r2 = sess.resume_filename(Some("transcript".to_string()));
        assert_eq!(sess.pending_filename(), None, "request cleared after supply");
        assert!(!r2.quit);
        assert_eq!(sess.pending_input(), InputKind::Line, "resumes at the turn-2 prompt");
    }

    #[test]
    fn create_by_prompt_savedgame_auto_resolves_into_save_without_a_filename_modal() {
        use E::*;
        const FREF: u32 = 0x410;
        const SAVE_RES: u32 = 0x414;
        let mut body = open_buffer_prelude();
        body.extend(line_prompt()); // turn 1 prompt
        // create_by_prompt(usage=SavedGame(1), fmode=Write(1), rock=0) -> mem[FREF].
        body.extend(enc(0x40, &[Imm(0), Push])); // rock
        body.extend(enc(0x40, &[Imm(1), Push])); // fmode = Write
        body.extend(enc(0x40, &[Imm(1), Push])); // usage = SavedGame (topmost)
        body.extend(enc(0x130, &[Imm(0x62), Imm(3), MemLoad(FREF)]));
        body.extend(enc(0x123, &[Imm(0), MemLoad(SAVE_RES)])); // @save (host-intercepted)
        body.extend(line_prompt());
        body.extend(enc(0x120, &[])); // quit
        let mut sess = GlulxSession::new(image_for(body, 1), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        let r1 = sess.submit("save");
        // SavedGame create_by_prompt must NOT surface a filename request; it auto-resolves
        // in-session so the turn reaches @save and bubbles a Save request in ONE turn.
        assert_eq!(sess.pending_filename(), None, "SavedGame create_by_prompt does not prompt");
        assert_eq!(r1.pending_io, Some(PendingIo::Save), "the same turn reaches @save");
    }

    #[test]
    fn ingame_restore_cancel_completes_as_failure() {
        use E::*;
        const RESTORE_RES: u32 = 0x414;
        let mut body = open_buffer_prelude();
        body.extend(line_prompt());
        body.extend(enc(0x124, &[Imm(0), MemLoad(RESTORE_RES)])); // @restore
        body.extend(enc(0x120, &[])); // quit
        let mut sess = GlulxSession::new(image_for(body, 1), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");

        let r1 = sess.submit("restore");
        assert_eq!(r1.pending_io, Some(PendingIo::Restore));
        // Cancel (no bytes): @restore fails, execution falls through to quit.
        let r2 = sess.resume_restore(None);
        assert!(r2.quit, "a cancelled restore fails and runs to the trailing quit");
    }

    /// Program: open a buffer, request a char, echo it as a char, quit.
    fn char_echo_image() -> Vec<u8> {
        use E::*;
        let mut body = open_buffer_prelude();
        body.extend(enc(0x40, &[LocLoad(0), Push]));
        body.extend(enc(0x130, &[Imm(0xd2), Imm(1), Discard])); // request_char_event
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        body.extend(enc(0x40, &[MemLoad(VAL1), Push])); // the key code (val1)
        body.extend(enc(0x130, &[Imm(0x80), Imm(1), Discard])); // glk_put_char(key)
        body.extend(enc(0x120, &[])); // quit
        image_for(body, 1)
    }

    #[test]
    fn submit_key_delivers_char_and_skips_unmapped() {
        let mut sess = GlulxSession::new(char_echo_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Char);
        // An unmapped key (Insert) leaves the VM untouched.
        assert!(sess.submit_key(KeyInput::Insert).is_none());
        assert_eq!(sess.pending_input(), InputKind::Char, "VM untouched by unmapped key");
        // A printable key is delivered, echoed, and drives to quit.
        let r = sess.submit_key(KeyInput::Char('Z')).expect("mapped key produces a turn");
        assert_eq!(r.transcript, "Z");
        assert!(r.quit);
    }

    /// Program: open a buffer, arm a 50ms timer, then glk_select with NO line/char
    /// request — a legal blocking wait on the timer (Glk §4.4). Once the timer
    /// resolves the select, quit.
    fn timer_wait_image() -> Vec<u8> {
        use E::*;
        let mut body = open_buffer_prelude();
        body.extend(enc(0x40, &[Imm(50), Push]));
        body.extend(enc(0x130, &[Imm(0xd6), Imm(1), Discard])); // request_timer_events(50)
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        body.extend(enc(0x120, &[])); // quit
        image_for(body, 1)
    }

    #[test]
    fn timer_only_select_suspends_as_event_and_deliver_timer_advances() {
        let mut sess = GlulxSession::new(timer_wait_image(), 80, 24, true, false, false, (1, 1), None, &[])
            .expect("new");
        // The timer-only select suspends as Event (not Line/Char) and did NOT
        // spin to quit — the SQ-0299 fix.
        assert_eq!(sess.pending_input(), InputKind::Event, "timer-only select suspends as Event");
        assert!(!sess.has_quit(), "the game blocks on the timer; it did not run to quit");
        assert_eq!(
            sess.timer_interval(),
            Some(std::time::Duration::from_millis(50)),
            "the timer clock is armed"
        );
        // The host clock delivers a tick, resolving the select; the game resumes.
        let r = sess.deliver_timer();
        assert!(r.quit, "the delivered timer tick advanced the game past the select to quit");
    }

    /// Program: open a buffer, request a line, quit on input.
    fn simple_line_image() -> Vec<u8> {
        use E::*;
        let mut body = open_buffer_prelude();
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        body.extend(enc(0x120, &[])); // quit
        image_for(body, 1)
    }

    /// Program: open a buffer, split off a proportional (50%) graphics window
    /// below it, fill a rect to create its canvas, request a line, then select
    /// twice so the game stays alive across an Arrange (re-suspending on the
    /// persisted line request). local0 = buffer, local1 = graphics window.
    fn graphics_split_line_image() -> Vec<u8> {
        use E::*;
        let mut body = open_buffer_prelude();
        // window_open(split=buffer, method=Below|Proportional=0x23, size=50,
        // wintype_Graphics=5, rock=0) → local1. Push order is args reversed.
        for v in [Imm(0), Imm(5), Imm(50), Imm(0x23), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0x23), Imm(5), LocStore(4)])); // window_open → local1 (byte offset 4)
        // fill_rect(win=graphics, color=0x00FFFFFF, left=0, top=0, w=4, h=4) to
        // materialize the canvas so relayout has something to resize.
        for v in [Imm(4), Imm(4), Imm(0), Imm(0), Imm(0x00FF_FFFF), LocLoad(4)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0xEA), Imm(6), Discard])); // glk_window_fill_rect
        // request_line_event(win=buffer, LINEBUF, maxlen=20, initlen=0).
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        // Three selects: #1 drains the Redraw queued by opening the graphics
        // window; #2 is where new() suspends on the line request; #3 is where the
        // game re-suspends after the resize()-delivered Arrange (proving survival).
        for _ in 0..3 {
            body.extend(enc(0x40, &[Imm(EVENT), Push]));
            body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        }
        body.extend(enc(0x120, &[])); // quit
        image_for(body, 2)
    }

    /// Find the first Graphics leaf's canvas pixel size in a screen model.
    fn graphics_canvas_dims(node: &WinNode) -> Option<(u32, u32)> {
        match node {
            WinNode::Graphics(g) => Some((g.canvas.width(), g.canvas.height())),
            WinNode::Pair { first, second, .. } => {
                graphics_canvas_dims(first).or_else(|| graphics_canvas_dims(second))
            }
            _ => None,
        }
    }

    #[test]
    fn resize_rescales_graphics_canvas_and_game_survives_arrange() {
        // char_px = (2, 2): a 50%-height graphics window under an 80x24 screen.
        // The default-bordered split reserves one separator row (SQ-0325 T1), so
        // gvm snaps the 24 rows to 23 and divides the 22-row content into equal
        // 11-row halves → the graphics window is 80 cols x 11 rows → 160 x 22 px.
        // Shrinking the pane to 40 cols halves its width to 80 px (rows
        // unchanged); the game re-suspends on its line request (not quit).
        let mut sess =
            GlulxSession::new(graphics_split_line_image(), 80, 24, true, true, false, (2, 2), None, &[])
                .expect("new");
        assert_eq!(sess.pending_input(), InputKind::Line);
        let (w0, h0) = graphics_canvas_dims(&sess.screen().root).expect("a graphics window");
        assert_eq!((w0, h0), (160, 22), "initial canvas from the 80-col virtual screen");

        sess.resize(40, 24);
        assert_eq!(sess.pending_input(), InputKind::Line, "game re-suspended on its line request");
        assert!(!sess.has_quit(), "an Arrange must not end the game");
        let (w1, h1) = graphics_canvas_dims(&sess.screen().root).expect("a graphics window");
        assert_eq!((w1, h1), (80, 22), "canvas tracks the narrower pane after resize");
    }

    #[test]
    fn resize_after_quit_is_a_noop() {
        // A quit session must ignore resize (no drive, no panic).
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, false, (1, 1), None, &[])
            .expect("new");
        let r = sess.submit("go"); // drives to quit
        assert!(r.quit);
        sess.resize(40, 12); // must be a harmless no-op
        assert!(sess.has_quit());
    }

    #[test]
    fn finish_turn_drains_buffered_sound_ops() {
        use crate::session::SchannelOp;
        use gvm::glk::GlkBackend;
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, true, (1, 1), None, &[])
            .expect("new");
        {
            let g = sess.appglk();
            let c = g.schannel_create(0);
            g.schannel_play(c, 7, 1, 0);
        }
        let result = sess.submit(""); // finish_turn drains the buffered op
        assert!(
            result.glulx_sound_ops.iter().any(|op| matches!(op, SchannelOp::Play { snd: 7, .. })),
            "buffered play reached the TurnResult: {:?}",
            result.glulx_sound_ops
        );
    }

    #[test]
    fn trailing_read_prompt_is_stripped_from_banner_and_turns() {
        use E::*;
        // Print "Hi\n> ", request a line (banner), then after input print
        // "done\n> " and request again (a turn). Both the banner and the turn
        // transcript must drop the trailing prompt so the app's bottom input bar
        // is the only ">" the player sees (matches the Z-machine path).
        let mut body = open_buffer_prelude();
        for c in b"Hi\n> " {
            body.extend(streamchar(*c));
        }
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select (banner)
        for c in b"done\n> " {
            body.extend(streamchar(*c));
        }
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select (turn)
        body.extend(enc(0x120, &[])); // quit
        let image = image_for(body, 1);

        let mut sess = GlulxSession::new(image, 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert_eq!(sess.take_transcript(), "Hi", "banner drops the trailing prompt");
        let r = sess.submit("x");
        assert_eq!(r.transcript, "done", "turn output drops the trailing prompt");
    }

    #[test]
    fn trailing_read_prompt_kept_when_strip_prompt_is_false() {
        use E::*;
        // strip_prompt gates whether the trailing "> " read prompt is removed
        // (SQ-0264: inline-prompt mode keeps it). With strip_prompt = false both
        // the banner and turn transcripts must retain the game's own ">".
        let mut body = open_buffer_prelude();
        for c in b"Hi\n> " {
            body.extend(streamchar(*c));
        }
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select (banner)
        for c in b"done\n> " {
            body.extend(streamchar(*c));
        }
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select (turn)
        body.extend(enc(0x120, &[])); // quit
        let image = image_for(body, 1);

        let mut sess = GlulxSession::new(image, 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        sess.strip_prompt = false;
        assert_eq!(sess.take_transcript(), "Hi\n> ", "banner keeps the trailing prompt");
        let r = sess.submit("x");
        assert_eq!(r.transcript, "done\n> ", "turn output keeps the trailing prompt");
    }

    #[test]
    fn banner_take_transcript_elems_keeps_startup_image_and_strips_prompt() {
        // A game that draws a startup/cover image before the first turn: the
        // banner "Hi\n> " lands in the primary buffer, then an inline image is
        // drawn (seeded directly, as a resolvable Pict needs a Blorb the harness
        // lacks). `take_transcript_elems` must return the image in order AND strip
        // the trailing read prompt from the Text element — mirroring the
        // string-only `take_transcript` (which returns "Hi").
        use E::*;
        let mut body = open_buffer_prelude();
        for c in b"Hi\n> " {
            body.extend(streamchar(*c));
        }
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select (banner)
        let image = image_for(body, 1);

        let mut sess = GlulxSession::new(image, 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        let dummy = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(3, 3)),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        };
        sess.appglk().test_push_primary_image(dummy);

        let elems = sess.take_transcript_elems();
        assert_eq!(elems.len(), 2, "one Text element + the startup image");
        match &elems[0] {
            TranscriptElem::Text { text, .. } => {
                assert_eq!(text, "Hi", "trailing read prompt stripped from the Text element")
            }
            _ => panic!("elems[0] must be Text"),
        }
        assert!(
            matches!(&elems[1], TranscriptElem::Image(_)),
            "the startup image survives in order",
        );
    }

    #[test]
    fn save_state_is_tagged_and_round_trips_with_guard() {
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        let save = sess.save_state();
        assert_eq!(save.engine, GLULX_ENGINE);
        assert!(!save.bytes.is_empty(), "gvm snapshot is non-empty");
        // Same-engine restore succeeds.
        sess.restore_state(&save).expect("same-engine restore");
        // A foreign-engine save is refused.
        let foreign = EngineSave::new("zmachine", 1, save.bytes.clone());
        match sess.restore_state(&foreign) {
            Err(EngineError::EngineMismatch { expected, found }) => {
                assert_eq!(expected, GLULX_ENGINE);
                assert_eq!(found, "zmachine");
            }
            other => panic!("foreign restore must be refused, got {other:?}"),
        }
    }

    #[test]
    fn introspect_is_none() {
        let sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert!(sess.introspect().is_none(), "Glulx introspection is SP4");
    }

    #[test]
    fn heading_to_room_uses_synthetic_id() {
        let r = super::heading_to_room("Studio Apartment");
        assert_eq!(r.name, "Studio Apartment");
        assert_eq!(r.parent, 0);
        assert_eq!(r.number, crate::roomid::synthetic_room_id("Studio Apartment"));
        // Same name → same id (identity is name-based).
        assert_eq!(super::heading_to_room("Studio Apartment").number, r.number);
    }

    #[test]
    fn glulx_state_round_trips_through_lanthorn_archive() {
        use std::collections::BTreeMap;
        // A Glulx engine save survives a .lanthorn archive round-trip: write its
        // EngineSave (no screen.json), reload, and restore into a FRESH session
        // through Engine::restore_state — state is preserved, no panic.
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        let _ = sess.take_transcript(); // drain the banner
        let es = sess.save_state();
        assert_eq!(es.engine, GLULX_ENGINE);

        let mut path = std::env::temp_dir();
        path.push(format!("lanthorn-glulx-arch-{}.lanthorn", std::process::id()));
        let mapper = mapper::mapper::Mapper::default();
        crate::archive::save_archive(&path, &mapper, &es, None, &BTreeMap::new(),
            &[], &[], &[], &[], &[], &[]).expect("save archive");

        let ac = crate::archive::load_archive(&path).expect("load archive");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.engine, GLULX_ENGINE, "archive records the glulx tag");
        assert!(ac.screen.is_none(), "Glulx archive carries no screen.json");
        assert_eq!(ac.save, es.bytes, "archived bytes are the Glulx save");

        let mut fresh = GlulxSession::new(simple_line_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        let _ = fresh.take_transcript();
        fresh.restore_state(&ac.engine_save()).expect("Glulx restore from archive");
        assert_eq!(fresh.pending_input(), InputKind::Line, "restored input state");
        assert_eq!(fresh.save_state().bytes, es.bytes, "restored Glulx state matches");
    }

    #[test]
    fn glulx_restore_refuses_zmachine_archive() {
        // The foreign-engine guard fires gracefully (no panic) when a zmachine
        // save is offered to a Glulx session.
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        let foreign = EngineSave::new("zmachine", 1, vec![1, 2, 3]);
        assert!(matches!(
            sess.restore_state(&foreign),
            Err(EngineError::EngineMismatch { .. })
        ));
    }

    /// End-to-end smoke: a hand-built .ulx plays through GlulxSession and renders
    /// through the generic story-pane renderer with the map pane bolted alongside.
    #[test]
    fn glulx_plays_and_renders_end_to_end() {
        use crate::render::screen::render_story_pane;
        use crate::state::AppState;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        // Program: open a buffer, print "Hello", request a line, echo it, quit.
        let image = {
            use E::*;
            let mut body = open_buffer_prelude();
            for c in b"Hello" {
                body.extend(streamchar(*c));
            }
            for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
                body.extend(enc(0x40, &[v, Push]));
            }
            body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
            body.extend(enc(0x40, &[Imm(EVENT), Push]));
            body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
            body.extend(enc(0x40, &[MemLoad(VAL1), Push])); // len
            body.extend(enc(0x40, &[Imm(LINEBUF), Push])); // addr
            body.extend(enc(0x130, &[Imm(0x84), Imm(2), Discard])); // glk_put_buffer
            body.extend(enc(0x120, &[])); // quit
            image_for(body, 1)
        };

        let mut sess = GlulxSession::new(image, 78, 20, true, false, false, (1, 1), None, &[]).expect("new");

        // Mirror the app loop: drain the banner into the transcript, take a turn.
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        let banner = sess.take_transcript();
        assert_eq!(banner, "Hello");
        state.push_transcript(&banner);

        let r = sess.submit("there");
        assert_eq!(r.transcript, "there");
        state.push_transcript(&r.transcript);

        // Render the Glulx screen into a story pane (no panic, content present).
        let area = Rect::new(0, 0, 78, 20);
        let mut buf = Buffer::empty(area);
        let _ = render_story_pane(&sess.screen(), false, sess.introspect(), &state, area, &mut buf);

        let dump: String = (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(dump.contains("Hello"), "banner rendered in the story pane");
        assert!(dump.contains("there"), "echoed line rendered in the story pane");
    }

    // ── Mouse input (glk_request_mouse_event) ─────────────────────────────────

    /// Program: open a buffer, split a 1-row grid above, arm a mouse request AND
    /// a char request on the grid, then glk_select (suspends on the char). The
    /// grid is window 2 (buffer=1, pair=3).
    fn grid_mouse_watch_image() -> Vec<u8> {
        use E::*;
        let mut body = open_buffer_prelude(); // buffer → local0 (window 1)
        for v in [Imm(0), Imm(4), Imm(1), Imm(0x12), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push])); // rock, wintype=grid, size=1, method=above|fixed, split
        }
        body.extend(enc(0x130, &[Imm(0x23), Imm(5), LocStore(4)])); // window_open → local1 (grid = window 2)
        body.extend(enc(0x40, &[LocLoad(4), Push]));
        body.extend(enc(0x130, &[Imm(0xd4), Imm(1), Discard])); // request_mouse_event(grid)
        body.extend(enc(0x40, &[LocLoad(4), Push]));
        body.extend(enc(0x130, &[Imm(0xd2), Imm(1), Discard])); // request_char_event(grid) → select suspends
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        body.extend(enc(0x120, &[])); // quit
        image_for(body, 2)
    }

    #[test]
    fn mouse_windows_lists_only_watching_windows_and_char_pixels_exposed() {
        let mut sess =
            GlulxSession::new(grid_mouse_watch_image(), 80, 24, true, false, false, (9, 19), None, &[]).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Char, "suspends on the grid char request");

        // Only the grid (window 2) watches; the buffer (window 1) does not. Ids
        // only (SQ-1203) — the DRAWN rect for hit-testing comes from the
        // render's own `win_rects`, not from gvm's layout.
        let windows = sess.mouse_windows();
        assert_eq!(windows, vec![2], "only the requesting window's id is listed");

        assert_eq!(sess.char_pixels(), (9, 19), "cell pixel size exposed for graphics scaling");
    }

    #[test]
    fn deliver_mouse_resumes_the_game_and_is_one_shot() {
        let mut sess =
            GlulxSession::new(grid_mouse_watch_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert!(!sess.mouse_windows().is_empty(), "armed before the click");

        // The click resumes the suspended select; the game runs to its trailing quit.
        let r = sess.deliver_mouse(2, 5, 0);
        assert!(r.quit, "the game consumed the click and ran to quit");
        assert!(sess.mouse_windows().is_empty(), "mouse request consumed (one-shot)");
    }

    /// SQ-1220: the renderer draws a text window out over every gap it hands the
    /// trailing child — the separator the theme never draws (SQ-1203) and the
    /// padding cell a proportional split keeps — so a click hit-tested against
    /// that DRAWN rect can carry a coordinate past the window the game was told
    /// it has. Glk §8.6 says a mouse event's coordinates are inside the window.
    #[test]
    fn mouse_coordinates_are_clamped_into_the_window_the_game_was_told() {
        let mut sess =
            GlulxSession::new(grid_mouse_watch_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        // Window 2 is the grid: one row, the pane's full 80 columns.
        assert_eq!(sess.clamp_into_window(2, 5, 0), (5, 0), "a click inside the window is untouched");
        assert_eq!(
            sess.clamp_into_window(2, 79, 0),
            (79, 0),
            "the window's own last column is not moved"
        );
        assert_eq!(
            sess.clamp_into_window(2, 81, 3),
            (79, 0),
            "a click on a gutter cell past the edge lands on the last cell the game knows"
        );
        // An unknown window is passed through — nothing to clamp against.
        assert_eq!(sess.clamp_into_window(999, 95, 7), (95, 7), "no layout entry, no clamp");
    }

    // ── glk_mouse_target coordinate mapping ───────────────────────────────────
    //
    // `win_rects` carries the DRAWN rect in ABSOLUTE screen coordinates (SQ-1203
    // — what `render_node` actually painted, not gvm's own layout rect, which
    // reserves a border gutter the theme may draw thinner or not at all). Every
    // case below therefore states the window's rect in screen coordinates, not
    // pane-relative ones; `requested` is the separate id list `mouse_windows`
    // reports.

    use crate::engine::WinKind;

    #[test]
    fn glk_mouse_target_grid_is_identity_minus_origin() {
        // Story pane at (3, 2); a grid window drawn filling the top row, so its
        // absolute rect is (3, 2, 80, 1).
        let requested = [2u32];
        let win_rects = [(2u32, WinKind::Grid, ratatui::layout::Rect::new(3, 2, 80, 1))];
        // Click at absolute (10, 2) → grid-relative (10-3, 2-2) = (7, 0).
        let got = super::glk_mouse_target(false, 10, 2, (3, 2, 80, 24), &requested, &win_rects, (9, 19), None);
        assert_eq!(got, Some((2, 7, 0)), "grid reports window-relative col/row, from the DRAWN origin");
    }

    #[test]
    fn glk_mouse_target_graphics_scales_by_char_pixels() {
        // A graphics window drawn at absolute rect (4, 1, 20, 10); story pane at
        // origin (0, 0), so pane-relative and absolute coincide here.
        let requested = [5u32];
        let win_rects = [(5u32, WinKind::Graphics, ratatui::layout::Rect::new(4, 1, 20, 10))];
        // Click at (6, 3) → window-relative (2, 2) → CELL-CENTRE pixels
        // (2×9+4, 2×19+9) = (22, 47) (SQ-0520: centre, not top-left, so packed
        // toolbar buttons hit true).
        let got = super::glk_mouse_target(false, 6, 3, (0, 0, 80, 24), &requested, &win_rects, (9, 19), None);
        assert_eq!(got, Some((5, 22, 47)), "graphics reports cell-centre pixels");
    }

    /// SQ-0563: with pixel mouse reporting the click's offset inside its cell is
    /// known, so a graphics window hears the exact pixel — the only way to name a
    /// band that lies between two cell centres.
    #[test]
    fn glk_mouse_target_graphics_uses_the_sub_cell_offset_when_known() {
        let requested = [5u32];
        let win_rects = [(5u32, WinKind::Graphics, ratatui::layout::Rect::new(0, 0, 20, 2))];
        // advent.blb's toolbar geometry: 8×18 cells. Cell row 0, 14px down — the
        // W/E compass band, which cell-centre reporting (y=9 or 27) cannot reach.
        let got = super::glk_mouse_target(false, 2, 0, (0, 0, 80, 24), &requested, &win_rects, (8, 18), Some((3, 14)));
        assert_eq!(got, Some((5, 2 * 8 + 3, 14)), "the exact pixel clicked");

        // Second cell row: the offset is relative to the cell, so it adds on top.
        let got = super::glk_mouse_target(false, 2, 1, (0, 0, 80, 24), &requested, &win_rects, (8, 18), Some((0, 4)));
        assert_eq!(got, Some((5, 16, 18 + 4)), "row offset plus in-cell offset");
    }

    /// An offset can never escape its cell, even if the terminal's idea of the cell
    /// size disagrees with the one the game was told.
    #[test]
    fn glk_mouse_target_clamps_a_sub_cell_offset_inside_the_cell() {
        let requested = [5u32];
        let win_rects = [(5u32, WinKind::Graphics, ratatui::layout::Rect::new(0, 0, 20, 2))];
        let got = super::glk_mouse_target(false, 1, 0, (0, 0, 80, 24), &requested, &win_rects, (8, 18), Some((99, 99)));
        assert_eq!(got, Some((5, 8 + 7, 17)), "clamped to the cell's last pixel");
    }

    #[test]
    fn glk_mouse_target_declines_outside_and_under_an_overlay() {
        let requested = [2u32];
        let win_rects = [(2u32, WinKind::Grid, ratatui::layout::Rect::new(0, 0, 80, 1))];
        // Click below the grid (row 5) but inside the pane → misses every window.
        assert_eq!(
            super::glk_mouse_target(false, 10, 5, (0, 0, 80, 24), &requested, &win_rects, (1, 1), None),
            None,
            "a click outside every watching window falls through",
        );
        // Click outside the story pane entirely.
        assert_eq!(
            super::glk_mouse_target(false, 90, 0, (0, 0, 80, 24), &requested, &win_rects, (1, 1), None),
            None,
            "a click outside the story pane falls through",
        );
        // Same in-window click, but an overlay is open → declined.
        assert_eq!(
            super::glk_mouse_target(true, 10, 0, (0, 0, 80, 24), &requested, &win_rects, (1, 1), None),
            None,
            "an open overlay keeps the click",
        );
    }

    /// A window drawn on screen but NOT in the requested set (not currently
    /// mouse-watching) declines the click, even though its rect contains it
    /// (SQ-1203: the id set and the drawn rects are independent lists).
    #[test]
    fn glk_mouse_target_declines_a_drawn_but_unrequested_window() {
        let requested: [u32; 0] = [];
        let win_rects = [(2u32, WinKind::Grid, ratatui::layout::Rect::new(0, 0, 80, 1))];
        assert_eq!(
            super::glk_mouse_target(false, 10, 0, (0, 0, 80, 24), &requested, &win_rects, (1, 1), None),
            None,
            "drawn but not watching → declined",
        );
    }

    /// This is SQ-1203 itself, reproduced without a real game: gvm's layout rect
    /// reserves a border gutter the theme never draws (SQ-0821's no-rule
    /// default), so a window whose DRAWN origin is one cell EARLIER than gvm's
    /// reported origin must be hit-tested against the drawn one — a click on the
    /// drawn top-left corner, which the old gvm-rect hit-test missed entirely.
    #[test]
    fn glk_mouse_target_uses_the_drawn_rect_not_a_gutter_inflated_one() {
        let requested = [5u32];
        // gvm's own layout would have reported this window one cell down/right
        // (a reserved but undrawn border gutter); the DRAWN rect is what actually
        // reached the screen.
        let win_rects = [(5u32, WinKind::Grid, ratatui::layout::Rect::new(9, 24, 70, 5))];
        // A click on the drawn top-left corner (9, 24) must hit window-relative
        // (0, 0) — under the old gvm-rect math (origin (10, 25)) this cell fell
        // outside every window and the click was dropped.
        let got = super::glk_mouse_target(false, 9, 24, (0, 0, 80, 30), &requested, &win_rects, (1, 1), None);
        assert_eq!(got, Some((5, 0, 0)), "hit-tested against the DRAWN origin, not a gutter-inflated one");
    }

    // ── Hyperlink input (glk_request_hyperlink_event) ─────────────────────────

    /// Program: open a buffer, split a 1-row grid above, arm a hyperlink request
    /// AND a char request on the grid, then glk_select (suspends on the char).
    /// The grid is window 2 (buffer=1, pair=3). Mirrors `grid_mouse_watch_image`
    /// with `request_hyperlink_event` (0x102) in place of `request_mouse_event`.
    fn grid_hyperlink_watch_image() -> Vec<u8> {
        use E::*;
        let mut body = open_buffer_prelude(); // buffer → local0 (window 1)
        for v in [Imm(0), Imm(4), Imm(1), Imm(0x12), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push])); // rock, wintype=grid, size=1, method=above|fixed, split
        }
        body.extend(enc(0x130, &[Imm(0x23), Imm(5), LocStore(4)])); // window_open → local1 (grid = window 2)
        body.extend(enc(0x40, &[LocLoad(4), Push]));
        body.extend(enc(0x130, &[Imm(0x102), Imm(1), Discard])); // request_hyperlink_event(grid)
        body.extend(enc(0x40, &[LocLoad(4), Push]));
        body.extend(enc(0x130, &[Imm(0xd2), Imm(1), Discard])); // request_char_event(grid) → select suspends
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        body.extend(enc(0x120, &[])); // quit
        image_for(body, 2)
    }

    #[test]
    fn hyperlink_windows_lists_only_watching_windows() {
        let mut sess =
            GlulxSession::new(grid_hyperlink_watch_image(), 80, 24, true, false, false, (9, 19), None, &[]).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Char, "suspends on the grid char request");

        // Only the grid (window 2) watches; the buffer (window 1) does not. Ids
        // only (SQ-1203) — see `mouse_windows_lists_only_watching_windows_and_char_pixels_exposed`.
        let windows = sess.hyperlink_windows();
        assert_eq!(windows, vec![2], "only the requesting window's id is listed");
    }

    #[test]
    fn deliver_hyperlink_resumes_the_game_and_is_one_shot() {
        let mut sess =
            GlulxSession::new(grid_hyperlink_watch_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert!(!sess.hyperlink_windows().is_empty(), "armed before the click");

        // The click resumes the suspended select; the game runs to its trailing quit.
        let r = sess.deliver_hyperlink(2, 42);
        assert!(r.quit, "the game consumed the link and ran to quit");
        assert!(sess.hyperlink_windows().is_empty(), "hyperlink request consumed (one-shot)");
    }

    // ── glk_hyperlink_window hit test ─────────────────────────────────────────
    // Same DRAWN-rect / absolute-coordinate contract as `glk_mouse_target` above.

    #[test]
    fn glk_hyperlink_window_returns_the_owning_window_id() {
        // Story pane at (3, 2); a grid window drawn filling the top row, so its
        // absolute rect is (3, 2, 80, 1).
        let requested = [2u32];
        let win_rects = [(2u32, WinKind::Grid, ratatui::layout::Rect::new(3, 2, 80, 1))];
        // Click at absolute (10, 2) → inside the grid.
        let got = super::glk_hyperlink_window(false, 10, 2, (3, 2, 80, 24), &requested, &win_rects);
        assert_eq!(got, Some(2), "returns the window whose DRAWN rect contains the cell");
    }

    #[test]
    fn glk_hyperlink_window_declines_outside_overlay_and_empty() {
        let requested = [2u32];
        let win_rects = [(2u32, WinKind::Grid, ratatui::layout::Rect::new(0, 0, 80, 1))];
        // Click below the grid (row 5) but inside the pane → misses every window.
        assert_eq!(
            super::glk_hyperlink_window(false, 10, 5, (0, 0, 80, 24), &requested, &win_rects),
            None,
            "a click outside every watching window falls through",
        );
        // Click outside the story pane entirely.
        assert_eq!(
            super::glk_hyperlink_window(false, 90, 0, (0, 0, 80, 24), &requested, &win_rects),
            None,
            "a click outside the story pane falls through",
        );
        // Same in-window click, but an overlay is open → declined.
        assert_eq!(
            super::glk_hyperlink_window(true, 10, 0, (0, 0, 80, 24), &requested, &win_rects),
            None,
            "an open overlay keeps the click",
        );
        // No hyperlink-watching windows at all.
        assert_eq!(
            super::glk_hyperlink_window(false, 10, 0, (0, 0, 80, 24), &[], &[]),
            None,
            "no watching windows → nothing to divert to",
        );
    }

    /// A window drawn on screen but NOT in the requested set declines the click
    /// (SQ-1203: same independence as the mouse path).
    #[test]
    fn glk_hyperlink_window_declines_a_drawn_but_unrequested_window() {
        let requested: [u32; 0] = [];
        let win_rects = [(2u32, WinKind::Grid, ratatui::layout::Rect::new(0, 0, 80, 1))];
        assert_eq!(
            super::glk_hyperlink_window(false, 10, 0, (0, 0, 80, 24), &requested, &win_rects),
            None,
            "drawn but not watching → declined",
        );
    }
}
