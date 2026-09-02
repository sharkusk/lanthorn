//! Startup / boot sequence: parse args, load config, resolve and load the story,
//! build the engine, load the mapper/archive, seed the initial UI state, and set
//! up the terminal. Extracted verbatim from `main.rs` (SQ-0306) as `main()`'s
//! linear setup phase (originally "steps 1-4"). Split for SQ-0435 into
//! [`resolve_launch`] (the one-time arg/config resolution, run once by `main`)
//! and [`boot_story`] (the per-story build, run for each chosen story), so a
//! directory launch can replay the build across the picker→play loop. `main`
//! calls `resolve_launch`, then per story `boot_story` and the event loop over
//! the returned [`BootResult`]; helper fns they rely on stay in `main.rs`
//! (referenced via `crate::`) because they are shared with the loop or exercised
//! by `main.rs` tests.

use std::io::{stdout, Stdout};

use crossterm::event::{EnableBracketedPaste, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{enable_raw_mode, EnterAlternateScreen};
use mapper::mapper::Mapper;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

use clap::Parser;

use app::archive::load_archive;
use app::config::{config_path, resolve, Cli, Config, OnOff};
use app::engine::Engine;
use app::glulx_session::GlulxSession;
use app::hints;
use app::ifid::compute_ifid;
use app::session::{apply_turn, GameSession};
use app::state::AppState;
use app::storage::{DiskBuild, default_state_path, game_dir as story_game_dir, story_key_for};

use crate::engine_helpers::{restore_error_msg, zvm_session_opt_mut};
use crate::{
    install_panic_hook, loading_line, picker_ui, random_seed_line, resolve_pict_blorb,
    restore_terminal, saves_dir,
};

/// Everything [`boot`] produces that `main()`'s event loop then owns: the boxed
/// engine, the mapper, the UI state, the terminal handle, and the per-story
/// paths/identity the loop threads into save/restore/reset calls.
pub(crate) struct BootResult {
    pub session: Box<dyn Engine>,
    pub mapper: Mapper,
    pub state: AppState,
    /// Wrapped in [`app::terminal_dump::CountingWriter`] so `/dump-terminal` can
    /// report how many bytes a frame costs (SQ-0994). One `fetch_add` per write
    /// and one per flush; it never looks at a byte.
    pub terminal: Terminal<CrosstermBackend<app::terminal_dump::CountingWriter<std::io::BufWriter<Stdout>>>>,
    pub game_dir: std::path::PathBuf,
    pub ifid: String,
    pub arc_file: std::path::PathBuf,
    pub story_bytes: Vec<u8>,
    pub story_path: std::path::PathBuf,
    pub data_base: std::path::PathBuf,
}

/// The one-time launch context resolved before the picker→play loop: parsed
/// args, config, the saves/sidecar base dir, and whether lanthorn was launched
/// against a directory (a story library) or a single file. `resolve_launch`
/// builds this ONCE; `boot_story` consumes it (by reference) per story so a
/// library launch can replay the build for each chosen story. (SQ-0435)
pub(crate) struct LaunchCtx {
    pub cli: Cli,
    pub cfg: Config,
    pub data_base: std::path::PathBuf,
    /// The story directory when launched from a library (the picker source),
    /// else `None`.
    pub library_dir: Option<std::path::PathBuf>,
    /// The single story file when launched with a file argument, else `None`.
    pub single_file: Option<std::path::PathBuf>,
    /// Set when the launch argument was a URL rather than a path (SQ-1086): the
    /// address, and the local file it was fetched to. `single_file` is that same
    /// local file, so everything downstream of `resolve_launch` sees an ordinary
    /// story path; this is here only so `boot_story` can raise the keep-it prompt
    /// for the right story.
    pub fetched: Option<app::story_url::FetchedStory>,
}

/// Resolve the one-time launch context: parse args + config, seed the style
/// template, apply the `default_story_dir` fallback (plus the first-use prompt,
/// which runs exactly ONCE), compute the data base, and classify the launch as a
/// story library (a directory) or a single file. May `std::process::exit(2)`
/// when there's nothing to open. Signal-handler registration lives in `main`
/// (before the loop); the per-story build lives in [`boot_story`]. (SQ-0435)
pub(crate) fn resolve_launch() -> LaunchCtx {
    // ── 1. Parse args + load config ───────────────────────────────────────────

    let cli = Cli::parse();

    // `--machines` is a question about the machine table, not a launch (SQ-0960).
    // Answered here — before the config is read, before a template is seeded and
    // before a story is required — for the reason clap answers `--help` there:
    // it describes the program, so demanding a story to see it would be the wrong
    // question. `zvm-cli --machines` prints this same string, from `zvm` itself,
    // because a reporter kept in one front-end is a reporter the other copies.
    if cli.machines {
        print!("{}", zvm::machines::table());
        std::process::exit(0);
    }

    let mut cfg = resolve(&cli);

    // Asked BEFORE the seed below creates the file, because "there is no
    // config.toml" is the whole definition of a first run (SQ-1104). Read after
    // it, the answer would be "there is one" every time and the font check would
    // never fire.
    let first_run = !cfg.config_file.exists();

    // Auto-seed a fresh style.toml (SQ-0309, Task 6b) on every startup — before the
    // story picker — so browsing (even without launching a story) leaves the fully
    // commented, registry-derived template, and the picker reads the same file the
    // game does. Never overwrites an existing file; best-effort (a read-only home
    // must not crash startup).
    app::theme::template::auto_seed(&cfg.user_dir);

    // …and the same treatment for config.toml (SQ-0573): a fully commented template
    // listing EVERY setting at its default, so what lanthorn can be told to do is
    // discoverable from the file rather than only from the source. Same contract as
    // the style seed — never overwrites, best-effort. Seeded at the RESOLVED config
    // path (`--config`/`--user-dir`/default), not `user_dir`, so the file we seed is
    // the file we read (SQ-0574). Runtime edits still go through `write_config_file`,
    // which is format-preserving and keeps these comments.
    app::config_template::auto_seed(&cfg.config_file);

    // The seed above only ever writes a file that is not there, so a config written
    // by an older release never learns about a setting added since — and one of them
    // is `adult_words`, which is a default rather than an invisible filter precisely
    // because its owner can read it in their own file (SQ-1122). Append what is
    // missing, commented, touching nothing already written (SQ-1129).
    //
    // Skipped when the file failed to load: `write_config_at` refuses to write over a
    // config it could not read, and so do we. Nothing here can change `cfg` — every
    // line added is either a comment or a key at the value `resolve` already assumed
    // for its absence — so this run reads exactly as it would have.
    if cfg.config_error.is_none() {
        app::config_template::top_up(&cfg.config_file);
    }

    // A path may be omitted; fall back to the configured default story dir.
    // With neither, there's nothing to open — tell the user how to fix it.
    let story_path = match cli.story.clone().or_else(|| cfg.default_story_dir.clone()) {
        Some(p) => p,
        None => {
            eprintln!(
                "lanthorn: no story given. Pass a story file or directory, or set \
                 `default_story_dir` in {}.",
                config_path(&cli).display(),
            );
            std::process::exit(2);
        }
    };

    // ── SQ-1086: a URL wherever a path is accepted ───────────────────────────
    //
    // Fetched HERE, before anything downstream has to know the difference. Past
    // this line `story_path` is an ordinary local file, so every filetype the
    // loader already opens — `.z3`–`.z8`, Blorb, Glulx, Scott Adams, release disk
    // images, ZIPs — comes along for free and cannot drift from what opening the
    // same file by name would do. There is no second loader.
    //
    // A failure exits with the same code as "no story given", and says what it
    // fetched as well as that it could not open it: a 404 page, a login redirect
    // and a PDF are three different mistakes and only the message tells them
    // apart.
    //
    // Asked of the ARGUMENT only, never of the resolved path: a bare `lanthorn`
    // falls back to `default_story_dir`, and re-fetching a config value on every
    // launch is not a thing this should be able to do.
    //
    // SQ-1096 inverts that order for ONE case. A download that is a zip of
    // release disk images holds nothing the loader can run, so it cannot be
    // booted and then offered — the offer has to come first, and what the player
    // answers decides whether there is anything to launch at all. See
    // [`unpack_fetched_archive`].
    let (story_path, fetched) = match cli.story.as_deref().and_then(fetch_launch_url) {
        Some(app::story_url::Fetched::Story(f)) => (f.path.clone(), Some(f)),
        Some(app::story_url::Fetched::DiskImages(a)) => {
            (unpack_fetched_archive(&a, &cfg), None)
        }
        None => (story_path, None),
    };

    // First time a directory is passed on the command line with no default set,
    // offer to remember it as the default story directory (persisted to config).
    if cfg.default_story_dir.is_none()
        // A headless --fetch has no one to answer a question.
        && cli.fetch.is_none()
        && cli.import_metadata.is_none()
        && cli.story.as_deref().map(|p| p.is_dir()).unwrap_or(false)
        && prompt_yes_no(&format!(
            "Set {} as your default story directory?",
            story_path.display()
        ))
    {
        // Store an absolute path so a later bare `lanthorn` resolves the same
        // directory regardless of the working dir it's launched from. The dir
        // exists (is_dir passed), so canonicalize should succeed; fall back to
        // the supplied path if it somehow doesn't.
        let to_store = std::fs::canonicalize(&story_path).unwrap_or_else(|_| story_path.clone());
        cfg.default_story_dir = Some(to_store.clone());
        match app::config::write_config_file(&cfg) {
            Ok(()) => eprintln!("lanthorn: saved default story directory ({}).", to_store.display()),
            Err(e) => eprintln!("lanthorn: could not save config: {e}"),
        }
    }

    // ── SQ-1104: does this terminal's font draw the icon glyphs? ─────────────
    //
    // lanthorn cannot look. It writes characters and the font belongs to the
    // terminal; the nearest thing to a probe — write a glyph, read the cursor
    // back — measures WIDTH, and a missing-glyph box is exactly one cell wide.
    // So the eye is the oracle, and one question here configures the arrows, the
    // portal and stairs icons and the Guiding Light's mark together instead of
    // each of them drifting apart one report at a time.
    //
    // Asked here rather than at the top of this function so that the two exits
    // above it — `--machines`, and "no story given" — never raise a dialog
    // about a session that is not going to happen.
    //
    // A terminal that cannot be made interactive is not asked and nothing is
    // written; the plain glyphs are already the defaults. The config seed above
    // means that launch is a first run only ONCE — so a piped first launch used
    // to spend the chance silently, and no later interactive run ever offered it
    // (SQ-1112). It now leaves itself a note instead, and `--font-check on` /
    // `/run-font-check` remain the way to ask for it deliberately.
    let ask_font = should_ask_font_check(cli.font_check, first_run, cfg.font_check_pending);
    if ask_font {
        match ask_font_check(&cfg) {
            FontCheckOutcome::Answered { nerdfont, diagonal } => {
                match app::style::style_write_path(cfg.style.as_deref(), &cfg.user_dir) {
                    Some(path) => {
                        if let Err(e) = app::style::write_font_check_answer(&path, nerdfont, diagonal) {
                            eprintln!("lanthorn: could not save the font choice: {e}");
                        }
                    }
                    // `style = "default"` names the built-in style, which lives in
                    // the binary; there is no file to record an answer in.
                    None => eprintln!(
                        "lanthorn: `style = \"default\"` has no file to write the font choice to."
                    ),
                }
                // Asked and answered, so nothing is owed. Only ever a WRITE when
                // the note was actually there — clearing a flag that is already
                // clear would rewrite config.toml on every ordinary launch.
                set_font_check_pending(&mut cfg, false);
            }
            // Seen and dismissed. Unchanged from before: nothing written, and
            // nothing owed — re-asking someone who pressed Ctrl-C is nagging.
            FontCheckOutcome::Refused => {}
            // Nobody was asked, so the question outlives this launch.
            FontCheckOutcome::CouldNotAsk => set_font_check_pending(&mut cfg, true),
        }
    }

    // Storage base for saves/sidecars (SQ-0284): `--data-dir` overrides the
    // default `<user_dir>/saves`. Each story gets `<data_base>/<story-key>/`.
    let data_base = cli.data_dir.clone().unwrap_or_else(|| saves_dir(&cfg.user_dir));

    // A directory launches the pre-game picker (a library); a file plays directly.
    let (library_dir, single_file) = if story_path.is_dir() {
        (Some(story_path), None)
    } else {
        (None, Some(story_path))
    };

    LaunchCtx { cli, cfg, data_base, library_dir, single_file, fetched }
}

/// Fetch `arg` when it is a URL, returning the local file the rest of the boot
/// should use; `None` when it is an ordinary path (SQ-1086).
///
/// Exits 2 — the same code `resolve_launch` uses for "no story given" — when the
/// address is one lanthorn will not fetch, or when the fetch fails. Both messages
/// name what happened rather than leaving a "no such file" about a path nobody
/// typed.
fn fetch_launch_url(arg: &std::path::Path) -> Option<app::story_url::Fetched> {
    let text = arg.to_str()?;
    if !app::story_url::is_story_url(text) {
        // A `file://` or `ftp://` argument is URL-shaped and not fetchable; say
        // so instead of letting it fall through to a confusing open failure.
        if let Some(why) = app::story_url::declined_scheme(text) {
            eprintln!("lanthorn: {why}");
            std::process::exit(2);
        }
        return None;
    }
    let url = text.trim().to_string();
    let dir = app::story_url::download_dir();
    // Said before the fetch, not after: on a slow mirror this is the only sign
    // that lanthorn is doing anything at all, and it is still the ordinary
    // terminal here — the alternate screen is entered much further down.
    eprintln!("lanthorn: fetching {url} …");
    match app::story_url::fetch_to_dir(&app::story_url::HttpSource::new(), &url, &dir) {
        Ok(f) => {
            let path = match &f {
                app::story_url::Fetched::Story(s) => &s.path,
                app::story_url::Fetched::DiskImages(a) => &a.path,
            };
            eprintln!("lanthorn: saved to {}", path.display());
            Some(f)
        }
        Err(e) => {
            eprintln!("lanthorn: could not open {url}: {e}");
            std::process::exit(2);
        }
    }
}

/// Ask whether a downloaded ZIP of release disk images should be unpacked into
/// the library, and answer the path to launch (SQ-1096).
///
/// **This is the resequencing.** Every other fetch is booted and then offered;
/// this one cannot be, because `hints::load_mounted_story` refuses a zip whose
/// entries are floppies and the ordinary prompt lives far below that failure. So
/// the offer is raised HERE — before `LaunchCtx` exists, before the picker,
/// before any engine — and only a "yes" produces a story path at all.
///
/// Never returns on a decline: there is nothing to play, so the launch ends,
/// with a message saying what lanthorn will not do and how to make it possible.
fn unpack_fetched_archive(
    archive: &app::story_url::FetchedArchive,
    cfg: &Config,
) -> std::path::PathBuf {
    let n = archive.images.len();
    // No library, no offer: `default_story_dir` is the directory the picker
    // reads, and unpacking floppies anywhere else would put them where nothing
    // lists them. Said rather than silently declined — the fix is one config key.
    let Some(library_dir) = cfg.default_story_dir.clone() else {
        let _ = std::fs::remove_file(&archive.path);
        eprintln!(
            "lanthorn: {} holds {n} disk image{} and no story, and lanthorn does not run disk \
             images from inside a zip.",
            archive.filename(),
            if n == 1 { "" } else { "s" },
        );
        eprintln!(
            "lanthorn: set `default_story_dir` in your config and lanthorn can unpack them into \
             your library for you."
        );
        std::process::exit(2);
    };

    let collision = app::story_url::archive_collision(archive, &library_dir);
    let prompt = app::state::FetchKeepPrompt {
        fetched: app::story_url::FetchedStory {
            url: archive.url.clone(),
            path: archive.path.clone(),
        },
        library_dir: library_dir.clone(),
        collision,
        disk_images: archive.names(),
    };
    let mode = match ask_fetch_keep(prompt, cfg) {
        app::render::fetch_keep_dialog::FetchKeepAction::Keep(mode) => mode,
        _ => {
            // DECLINED — and unlike SQ-1086's decline, nothing was booted from
            // this file. That is the whole of the reason the temp copy is kept
            // there (it IS the running game, and its basename is the save key),
            // so with no session and no save key the reason does not carry: the
            // download is removed rather than left as an orphan in the temp dir.
            let _ = std::fs::remove_file(&archive.path);
            eprintln!(
                "lanthorn: not unpacked. lanthorn does not run disk images from inside a zip — \
                 keeping them in your library is how to play them."
            );
            std::process::exit(0);
        }
    };

    let written = match app::story_url::unpack_disk_images(archive, &library_dir, mode) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("lanthorn: could not unpack {}: {e}", archive.filename());
            std::process::exit(2);
        }
    };
    // The archive has served its purpose; the library holds the images now.
    let _ = std::fs::remove_file(&archive.path);
    for p in &written {
        eprintln!("lanthorn: unpacked {}", p.display());
    }
    // Launch the first image BY NAME, not by archive order: a release's volumes
    // are named in reading order far more reliably than they are stored, and
    // `cli_host::disk_set::mount_at` finds the rest as siblings in the directory
    // this just wrote them to — which is why they were flattened. A five-floppy
    // release is one shelf, not five launches.
    written
        .first()
        .cloned()
        .expect("an archive with no images is never a Fetched::DiskImages")
}

/// Run the fetch-keep prompt on its own, before any game exists (SQ-1096).
///
/// The dialog, its focus ring, its buttons and its keyboard ladder are all
/// `render::fetch_keep_dialog`'s — this is only the small terminal loop that
/// stands in for the game's, since there is no game yet. Tab/Shift-Tab move
/// focus, Enter activates, Esc cancels; Space is left alone (widget-reserved),
/// exactly as the shared chrome does everywhere else.
///
/// A terminal that cannot be made interactive DECLINES. Writing several files
/// into somebody's library is not a thing to do on a guess.
fn ask_fetch_keep(
    prompt: app::state::FetchKeepPrompt,
    cfg: &Config,
) -> app::render::fetch_keep_dialog::FetchKeepAction {
    use app::render::fetch_keep_dialog::{
        button_count, draw_fetch_keep_dialog, fetch_keep_key_focused, FetchKeepAction,
    };
    use crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};

    // Themed the way the game and the browser are, so the prompt does not arrive
    // in a palette the player has never seen.
    let (base, _w1) = app::style::load_style(cfg.style.as_deref(), &cfg.user_dir);
    let (colors, _syms, _w2) = app::style::resolve(&base, &cfg.user_dir);

    let mut state = AppState::default();
    state.colors = colors;
    state.overlays.fetch_keep = Some(prompt);
    state.overlays.dialog_focus = 0;

    if enable_raw_mode().is_err() {
        return FetchKeepAction::Decline;
    }
    if execute!(stdout(), EnterAlternateScreen).is_err() {
        crate::restore_terminal();
        return FetchKeepAction::Decline;
    }
    // Mouse capture follows the same opt-in the browser uses (`mouse = true`), so
    // a player who clicks dialogs everywhere else can click this one too, and a
    // player who has it off is not suddenly handed motion reporting.
    if cfg.mouse {
        let _ = execute!(stdout(), EnableMouseCapture);
    }
    let mut terminal = match Terminal::new(CrosstermBackend::new(stdout())) {
        Ok(t) => t,
        Err(_) => {
            crate::restore_terminal();
            return FetchKeepAction::Decline;
        }
    };

    let collision = state.overlays.fetch_keep.as_ref().is_some_and(|p| p.collision);
    let answer = loop {
        let mut rects = None;
        if terminal
            .draw(|f| {
                rects = draw_fetch_keep_dialog(&state, f.area(), f.buffer_mut());
            })
            .is_err()
        {
            break FetchKeepAction::Decline;
        }
        let ev = match crossterm::event::read() {
            Ok(ev) => ev,
            Err(_) => break FetchKeepAction::Decline,
        };
        // Clicks map to exactly the buttons the game loop's own handler maps them
        // to (`overlays.rs`, `FetchKeepOverlay::mouse`) — the close box and the
        // decline button both mean no.
        if let Event::Mouse(m) = &ev {
            if !matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
                continue;
            }
            let Some(r) = &rects else { continue };
            let pt = (m.column, m.row);
            if r.keep.is_some_and(|b| b.contains(pt.into())) {
                break FetchKeepAction::Keep(if collision {
                    app::story_url::KeepMode::Replace
                } else {
                    app::story_url::KeepMode::KeepBoth
                });
            }
            if r.keep_both.is_some_and(|b| b.contains(pt.into())) {
                break FetchKeepAction::Keep(app::story_url::KeepMode::KeepBoth);
            }
            if r.decline.is_some_and(|b| b.contains(pt.into()))
                || r.close.is_some_and(|b| b.contains(pt.into()))
            {
                break FetchKeepAction::Decline;
            }
            continue;
        }
        let Event::Key(key) = ev else { continue };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        // Ctrl-C is not an answer; it is a refusal, and a refusal writes nothing.
        if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c'))
        {
            break FetchKeepAction::Decline;
        }
        let n = button_count(&state);
        match key.code {
            KeyCode::Tab | KeyCode::Right => {
                state.overlays.dialog_focus = (state.overlays.dialog_focus + 1) % n;
            }
            KeyCode::BackTab | KeyCode::Left => {
                state.overlays.dialog_focus = (state.overlays.dialog_focus + n - 1) % n;
            }
            code => match fetch_keep_key_focused(code, state.overlays.dialog_focus, collision) {
                FetchKeepAction::None => {}
                act => break act,
            },
        }
    };

    // THE canonical teardown, not a copy of its steps (SQ-0998). Repeating the
    // sequence here missed `drain_pending_input`, so a mouse report that arrived
    // between the last `read()` and the disable was still on the fd when raw mode
    // ended — and went to the shell. `restore_terminal` is idempotent and every
    // step of it is a no-op for state this prompt never set.
    crate::restore_terminal();
    answer
}

/// Does this launch put the font question in front of the player?
///
/// Extracted from `resolve_launch` because it IS the fix for SQ-1112 and a
/// four-line `match` buried in a hundred-line function cannot be tested. The
/// flag still wins outright in both directions — `off` never asks however much
/// is owed, `on` always asks — and only the absent case consults state.
fn should_ask_font_check(flag: Option<OnOff>, first_run: bool, pending: bool) -> bool {
    match flag {
        Some(OnOff::Off) => false,
        Some(OnOff::On) => true,
        None => first_run || pending,
    }
}

/// Record — or clear — the note that the font question is still owed (SQ-1112).
///
/// A no-op when the flag already reads `want`, which is the common case by a
/// long way: an ordinary answered launch must not rewrite `config.toml` just to
/// set a false that is already false. `write_config_file` is format-preserving,
/// so the note joins a hand-edited file without disturbing it, and `put` skips
/// the key at its default so answering the question takes the line back out.
///
/// Best-effort, like every other config write on this path: a read-only home is
/// a reason to lose the note, never to fail the launch.
fn set_font_check_pending(cfg: &mut Config, want: bool) {
    if cfg.font_check_pending == want {
        return;
    }
    cfg.font_check_pending = want;
    if let Err(e) = app::config::write_config_file(cfg) {
        eprintln!("lanthorn: could not record the font-check state: {e}");
    }
}

/// What a run of the font check ended in — three outcomes, because two of them
/// used to be one `None` (SQ-1112).
///
/// A terminal that could not be made interactive and a player who pressed Ctrl-C
/// both left with nothing written, and the caller could not tell them apart — so
/// the launch spent its one first-run chance either way. They want opposite
/// treatment: nobody saw the question in the first case and it is still owed; in
/// the second the player saw it and dismissed it, and asking again next launch is
/// nagging.
enum FontCheckOutcome {
    /// Stage one was reached and answered: `nerdfont` = the patched-font row
    /// (which Esc and the close box also mean at that stage). `diagonal` is
    /// stage two's answer (SQ-1245) — `Some` for either row, `None` for a stage-
    /// two Esc/close/failure, which leaves `diagonal_corners` untouched rather
    /// than forcing a choice for a question the player never reached an opinion
    /// on.
    Answered { nerdfont: bool, diagonal: Option<bool> },
    /// Ctrl-C, at either stage. Seen and dismissed — nothing written at all,
    /// nothing owed, even if stage one had already been answered: Ctrl-C is the
    /// "get me out of this entirely" signal, not a per-stage cancel.
    Refused,
    /// No interactive terminal, a pane too small to hold stage one's
    /// comparison, or a read that failed, before stage one could be answered.
    /// Nobody was asked, so the question survives the launch.
    CouldNotAsk,
}

/// Run the font check on its own, before any game exists (SQ-1104, SQ-1245).
///
/// The dialog, its focus ring, its buttons and its keyboard ladder are all
/// `render::font_check_dialog`'s — this is only the small terminal loop that
/// stands in for the game's, since there is no game yet. Exactly the shape
/// [`ask_fetch_keep`] has, for the same reason: two drivers, one dialog module.
/// Tab/Shift-Tab move focus, Enter activates, Esc cancels; Space is left alone
/// (widget-reserved), as the shared chrome does everywhere else.
///
/// Two stages, one loop shape run twice: stage one (icon glyphs) then stage two
/// (diagonal corner stubs), sharing one `AppState`/`Terminal` and torn down
/// exactly ONCE at the end regardless of which stage or path it exits through
/// (SQ-0998) — an early `restore_terminal()` per exit point is a copy of the
/// canonical teardown's steps, which is what that quest fixed.
///
/// Nothing is written by any path but [`FontCheckOutcome::Answered`]; the plain
/// glyphs and the orthogonal fallback stand meanwhile, which are the answers
/// that work in every font.
fn ask_font_check(cfg: &Config) -> FontCheckOutcome {
    use app::render::font_check_dialog::{
        diagonal_check_key_focused, draw_diagonal_check_always, draw_font_check_always,
        font_check_key_focused, DiagonalCheckAction, FontCheckAction,
    };
    use crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};

    // Themed the way the game and the browser are, so the question does not
    // arrive in a palette the player has never seen — and so the sample rows are
    // drawn in the colours the map will actually use.
    let (base, _w1) = app::style::load_style(cfg.style.as_deref(), &cfg.user_dir);
    let (colors, _syms, _w2) = app::style::resolve(&base, &cfg.user_dir);

    let mut state = AppState::default();
    state.colors = colors;
    // Row 2 — the answer that changes nothing — starts focused, matching the
    // dialog's declared default. Enter without reading is not a decision to
    // install glyphs a font may not have.
    state.overlays.dialog_focus = 1;

    if enable_raw_mode().is_err() {
        return FontCheckOutcome::CouldNotAsk;
    }
    if execute!(stdout(), EnterAlternateScreen).is_err() {
        crate::restore_terminal();
        return FontCheckOutcome::CouldNotAsk;
    }
    if cfg.mouse {
        let _ = execute!(stdout(), EnableMouseCapture);
    }
    let mut terminal = match Terminal::new(CrosstermBackend::new(stdout())) {
        Ok(t) => t,
        Err(_) => {
            crate::restore_terminal();
            return FontCheckOutcome::CouldNotAsk;
        }
    };

    const BUTTONS: usize = 2;
    // A labeled BLOCK, not a loop: each stage below runs exactly once, and the
    // label exists only so an early Ctrl-C/CouldNotAsk from either stage can
    // jump straight to the end without a second copy of the teardown.
    let outcome = 'stages: {
        // ── Stage one: the icon glyphs ────────────────────────────────────
        let nerdfont = loop {
            let mut rects = None;
            if terminal
                .draw(|f| {
                    rects = draw_font_check_always(&state, f.area(), f.buffer_mut());
                })
                .is_err()
            {
                break 'stages FontCheckOutcome::CouldNotAsk;
            }
            // A pane too small to hold the comparison cannot ask the question,
            // and a question nobody can read must not block the launch.
            if rects.is_none() {
                break 'stages FontCheckOutcome::CouldNotAsk;
            }
            let ev = match crossterm::event::read() {
                Ok(ev) => ev,
                Err(_) => break 'stages FontCheckOutcome::CouldNotAsk,
            };
            if let Event::Mouse(m) = &ev {
                if !matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
                    continue;
                }
                let Some(r) = &rects else { continue };
                let pt = (m.column, m.row);
                if r.nerd.is_some_and(|b| b.contains(pt.into())) {
                    break true;
                }
                if r.plain.is_some_and(|b| b.contains(pt.into()))
                    || r.close.is_some_and(|b| b.contains(pt.into()))
                {
                    break false;
                }
                continue;
            }
            let Event::Key(key) = ev else { continue };
            if key.kind == KeyEventKind::Release {
                continue;
            }
            // Ctrl-C is not an answer; it is a refusal, and a refusal writes
            // nothing — at either stage.
            if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c'))
            {
                break 'stages FontCheckOutcome::Refused;
            }
            match key.code {
                KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                    state.overlays.dialog_focus = (state.overlays.dialog_focus + 1) % BUTTONS;
                }
                KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                    state.overlays.dialog_focus =
                        (state.overlays.dialog_focus + BUTTONS - 1) % BUTTONS;
                }
                code => match font_check_key_focused(code, state.overlays.dialog_focus) {
                    FontCheckAction::None => {}
                    FontCheckAction::Nerd => break true,
                    FontCheckAction::Plain => break false,
                },
            }
        };

        // ── Stage two: the diagonal corner stubs (SQ-1245) ────────────────
        // Its own default focus, matching the dialog's declared default —
        // stage one may have left focus on row 1.
        state.overlays.dialog_focus = 1;
        let diagonal = loop {
            let mut rects = None;
            // A draw failure or too-small pane here does not cost stage one's
            // answer — it just leaves `diagonal_corners` untouched, the same as
            // an explicit skip.
            if terminal
                .draw(|f| {
                    rects = draw_diagonal_check_always(&state, f.area(), f.buffer_mut());
                })
                .is_err()
            {
                break None;
            }
            if rects.is_none() {
                break None;
            }
            let ev = match crossterm::event::read() {
                Ok(ev) => ev,
                Err(_) => break None,
            };
            if let Event::Mouse(m) = &ev {
                if !matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
                    continue;
                }
                let Some(r) = &rects else { continue };
                let pt = (m.column, m.row);
                if r.nerd.is_some_and(|b| b.contains(pt.into())) {
                    break Some(true);
                }
                if r.plain.is_some_and(|b| b.contains(pt.into())) {
                    break Some(false);
                }
                if r.close.is_some_and(|b| b.contains(pt.into())) {
                    break None;
                }
                continue;
            }
            let Event::Key(key) = ev else { continue };
            if key.kind == KeyEventKind::Release {
                continue;
            }
            if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c'))
            {
                break 'stages FontCheckOutcome::Refused;
            }
            match key.code {
                KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                    state.overlays.dialog_focus = (state.overlays.dialog_focus + 1) % BUTTONS;
                }
                KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                    state.overlays.dialog_focus =
                        (state.overlays.dialog_focus + BUTTONS - 1) % BUTTONS;
                }
                code => match diagonal_check_key_focused(code, state.overlays.dialog_focus) {
                    DiagonalCheckAction::None => {}
                    DiagonalCheckAction::Diagonal => break Some(true),
                    DiagonalCheckAction::Orthogonal => break Some(false),
                    DiagonalCheckAction::Skip => break None,
                },
            }
        };

        FontCheckOutcome::Answered { nerdfont, diagonal }
    };

    // THE canonical teardown, not a copy of its steps (SQ-0998).
    crate::restore_terminal();
    outcome
}

/// The real story-pane `(rows, cols)` a v1–8 Z-machine session should be BOOTED
/// with — measured BEFORE the engine exists, so a v4/v5 story's boot-time
/// status-bar layout (Zork 1: paints its reverse bar once, at whatever width
/// header byte $21 held at that moment, then only re-cursors to the two field
/// columns it derived from it) already targets the real pane instead of the
/// zvm 80×24 fallback `init_caps` seeds absent a hint (SQ-0679/SQ-0680).
///
/// Runs the SAME split this frame's [`compute_pane_layout`]/[`story_screen_dims`]
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
/// `None` when the terminal size can't be queried (piped/non-terminal stdout,
/// e.g. some test harnesses) or the query reports a zero-area frame; the
/// constructor then falls back to the 80×24 boot default exactly as before this
/// change.
fn pre_boot_host_screen(
    cfg: &Config,
    cs: &app::colors::ColorScheme,
    garglk_overlay: &Option<app::garglk_ini::GarglkOverlay>,
    layout: app::state::Layout,
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
    boot_state.pane_sizes = app::state::PaneSizes {
        split_ratio: cfg.split_ratio,
        band_height: cfg.command_band.height,
        inv_dock_pct: cfg.inv_dock_pct,
        room_dock_pct: cfg.room_dock_pct,
    };
    host_story_screen(&boot_state)
}

/// The story pane the LIVE terminal gives `state`, in character cells — the
/// `host_screen` a boot is seeded with.
///
/// Split out of [`pre_boot_host_screen`] so a RESTART can ask the same question
/// of the real `AppState` (SQ-1061). Restarting passed a bare `None` here, under
/// a comment three dozen lines above promising "the same four links `startup.rs`
/// resolves, in the same order" — so `GameSession::new_for_machine` took neither
/// the `set_screen_dims` branch nor the `boot_screen_cols` one, and a v3/v4/v5
/// story whose status routine lays itself out once at boot came back on zvm's
/// 80x24 fallback. The launch had to synthesise an `AppState` because it runs
/// before there is one; a restart holds the real one, and the only thing that
/// kept the two apart was that this was a positional argument nobody had to fill.
///
/// `None` when the terminal size cannot be queried (piped/non-terminal stdout,
/// e.g. some test harnesses) or the query reports a zero-area frame; the
/// constructor then falls back to the 80x24 boot default.
pub(crate) fn host_story_screen(state: &AppState) -> Option<(u16, u16)> {
    let (term_cols, term_rows) = crossterm::terminal::size().ok()?;
    let frame = Rect::new(0, 0, term_cols, term_rows);
    if frame.width == 0 || frame.height == 0 {
        return None;
    }
    let pane_layout = app::layout::compute_pane_layout(frame, state, 0);
    app::render::screen::story_screen_dims(pane_layout.story, state)
}

/// Build the per-story engine + mapper + UI state + terminal for `story_path`,
/// using the one-time [`LaunchCtx`]. This is the per-story half of the old
/// `boot()` (load the story, build the engine, load the mapper/archive, seed the
/// state, and enter the alternate screen); a library launch calls it once per
/// chosen story. May `std::process::exit` on an unrecoverable per-story error
/// (unreadable/invalid story, terminal init failure) exactly as before.
///
/// `cfg` is cloned from `ctx` per story because the per-game overlays (garglk.ini
/// colours, per-game honor/borderless) mutate it — each story must start from the
/// pristine launch config. (SQ-0435)
pub(crate) fn boot_story(
    ctx: &LaunchCtx,
    story_path: std::path::PathBuf,
    disk_entry: Option<&str>,
    overrides: &app::launch_options::LaunchOverrides,
) -> BootResult {
    let cli = &ctx.cli;
    let mut cfg = ctx.cfg.clone();
    let data_base = ctx.data_base.clone();

    // `disk_entry` is which story on the image the browser row stood for
    // (SQ-0859) — `None` for every loose file and every single-story floppy, and
    // then this is byte-for-byte the load it always was.
    let (loaded, disk_image) = match hints::load_mounted_story_from(&story_path, disk_entry) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("lanthorn: cannot read '{}': {}", story_path.display(), e);
            std::process::exit(1);
        }
    };
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
        &story_key_for(app::storage::StoryOrigin {
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
    let picture_override = if cfg.images {
        app::graphics::PictureOverride::resolve_with_session(
            &story_path,
            &game_dir,
            cfg.pictures_override.as_deref(),
        )
    } else {
        app::graphics::PictureOverride::Unset
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
        let unnamed = !matches!(picture_override, app::graphics::PictureOverride::Loaded { .. });
        (cfg.images && disk_image.is_some() && unnamed)
            .then(|| app::graphics::unpaired_art_warning(&story_path, disk_entry))
            .flatten()
    });
    if let Some(msg) = &picture_warning {
        eprintln!("lanthorn: warning: {msg}");
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
        .or_else(|| app::styles::read_per_game_interpreter_number(&game_dir))
    {
        cfg.interpreter_number = Some(n);
        cfg.one_run.pin(app::config::keys::INTERPRETER_NUMBER, n);
    }
    // Ride with the story for the session: the restart path re-resolves artwork
    // and has no other way to know which game on the disc this is (SQ-0876).
    cfg.disk_entry = disk_entry.map(str::to_string);
    // SQ-0928: and WHERE the answer came from, which is what decides whether this
    // launch may present the machine's own colours. `IbmPc` is two answers wearing
    // one name — the machine a DOS floppy names, and the thing every story with no
    // medium falls through to — and only the first has a machine to be faithful to.
    (cfg.interpreter_profile, cfg.interpreter_source) =
        app::interpreter::InterpreterProfile::resolve_with_source(
            &story_path,
            cfg.interpreter_number,
            picture_override.flavour(),
            // The medium THIS story came off, already resolved by the mount above —
            // which on a hybrid disc is not the same as the image's own format
            // (SQ-0876).
            disk_image,
        );
    // SQ-0939: the palette, asked ONCE and asked HERE — before the session
    // constructor runs the story, and before the host resolves a single colour.
    //
    // Which table, and why the story's Version is part of the question, lives on
    // `Config::machine_text_palette` — with the licence, because an unlicensed
    // launch resolves through §8.3.1's own table (SQ-0928's rule, and SQ-1154's
    // `--colour theme|terminal`, which withholds the licence on original media).
    // The suites that measure a booted frame call the same function.
    //
    // Every consumer reads this one global: the VM's own `true_value` for window
    // properties 17/18, the ColorScheme's standard-colour seed, the v6 pixel path
    // and the CLI's SGR path. Setting it late, or per-path, is how one colour
    // number comes to look like two colours on one screen.
    zvm::screen::set_palette(cfg.machine_text_palette(story_bytes.first().copied()));
    // SQ-0885: an experiment knob for header `$1F`, set beside the palette
    // because it is the same kind of fact — one machine per run — and because
    // the session constructor runs the story, so it has to be in force before
    // the boot below. Re-asserted every launch (with `None` when the flag is
    // absent) so a picker→play loop cannot carry one story's override into the
    // next, exactly as the palette is.
    zvm::screen::set_interpreter_version(cli.interpreter_version);

    // Booting a large story to its first prompt can take several seconds, and this
    // happens before the alternate screen is entered — so the normal terminal would
    // otherwise sit frozen. Spin a tiny indicator on a side thread; it only starts
    // drawing after a short grace period, so quick loads never flicker.
    let loading_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let loading_spinner = {
        use std::io::Write as _;
        use std::sync::atomic::Ordering;
        let done = loading_done.clone();
        let name = story_path.display().to_string();
        let bytes = story_bytes.len();
        std::thread::spawn(move || {
            const FRAMES: [char; 4] = ['|', '/', '-', '\\'];
            const TICK_MS: u64 = 60;
            let (mut waited, mut i, mut shown) = (0u64, 0usize, false);
            while !done.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(TICK_MS));
                waited += TICK_MS;
                if waited >= 180 {
                    eprint!("\r{}", loading_line(&name, bytes, FRAMES[i % FRAMES.len()]));
                    let _ = std::io::stderr().flush();
                    i += 1;
                    shown = true;
                }
            }
            if shown {
                eprint!("\r\x1b[2K"); // erase the spinner line before the UI starts
                let _ = std::io::stderr().flush();
            }
        })
    };

    // In-game graphics Picker (None when --images off or unavailable). Built once
    // and reused both for the Glulx session's char-cell pixel size and, below,
    // AppState.game_picker (the render side already tolerates None).
    let game_picker = if cfg.images { picker_ui::build_cover_picker(cfg.image_protocol) } else { None };
    // Probe the terminal's own default fg/bg (OSC 10/11) in the same pre-UI query
    // window as the image-protocol Picker above (SQ-0510). Seeds the v6 raster
    // canvas's default ink/page so "terminal default" theme colours follow the
    // real terminal instead of a hardcoded light-grey-on-black. Never hangs;
    // terminals that don't answer leave both as None and keep today's fallbacks.
    // SQ-0769: the probe hands back a sweep as well as the colours. A terminal
    // busy with the picker's last frame answers after the drain has given up, and
    // the sweep is what keeps those replies out of the story — see `query_sweep`.
    let (term_default_colors, query_sweep) = app::term_colors::query_terminal_default_colors();
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
    let vfs_sidecar = app::vfs_store::read_vfs(&game_dir);
    app::trace::hostio(&cfg.user_dir, cfg.trace.hostio,
        format!("vfs_read({} bytes)", vfs_sidecar.len()));

    // Resolve the look from style.toml (the single styling source) BEFORE the
    // engine builds: a Glulx game may probe glk_style_measure for the host's
    // rendered colours during boot (SQ-0315; Kerkerkruip measures its style_User2
    // slot there and branches its whole presentation on the answer, SQ-0803), so
    // the theme pairs must be in the backend first — and the garglk.ini overlay
    // below must land in `cs` before they are derived. `state.colors` is assigned
    // from these below.
    let (style_doc, style_w1) = app::style::load_style(cfg.style.as_deref(), &cfg.user_dir);
    let (mut cs, set, style_w2) = app::style::resolve(&style_doc, &cfg.user_dir);
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
    let garglk_overlay = app::garglk_ini::discover(&story_path);
    let garglk_line = garglk_overlay.as_ref().map(|ov| {
        let summary = ov.apply(&mut cs);
        // …unless `--game-colours` was typed on this launch, which outranks both
        // per-game layers for the same reason `--interpreter` outranks the sidecar:
        // a flag is a deliberate instruction for the run, and a file beside the story
        // is not (SQ-0855). In BOTH directions since SQ-1082 — `--game-colours on`
        // is as much an instruction as `off` was.
        if let Some(h) = ov.honor_game_colours.filter(|_| cli.game_colours.is_none()) {
            // A garglk.ini found beside THIS story speaks for this story, so it is
            // pinned as one-run: the global config must not learn it (SQ-0807).
            cfg.honor_game_colours = h;
            cfg.one_run.pin(app::config::keys::HONOR_GAME_COLOURS, h);
        }
        summary.console_line()
    });
    // SQ-0318: apply the user's persisted per-game honor override (if any) ON TOP
    // of garglk/global, so the engine builds — and turn one renders — with the
    // user's explicit choice in force. The IFID is computed here (from the raw
    // bytes) and reused for the map dir / identity below.
    let ifid = compute_ifid(&story_bytes);
    if let Some(v) =
        app::styles::read_per_game_honor(&game_dir).filter(|_| cli.game_colours.is_none())
    {
        // The sidecar's key is this game's, not the global default's — pinned for
        // the same reason the garglk overlay above is (SQ-0807). `--game-colours`
        // outranks it, as above.
        cfg.honor_game_colours = v;
        cfg.one_run.pin(app::config::keys::HONOR_GAME_COLOURS, v);
    }
    // SQ-0341: per-game borderless-windows override (default off → honor the Glk
    // border hint). Applies to Glulx layout from the first relayout at boot.
    // SQ-0344: precedence mirrors honor_game_colours — an explicit per-game
    // `config.toml` value wins, else a discovered garglk.ini's `wborderx`/
    // `wbordery` (0 → borderless), else off.
    let borderless = app::styles::read_per_game_borderless(&game_dir)
        .or_else(|| garglk_overlay.as_ref().and_then(|o| o.borderless))
        .unwrap_or(false);
    // SQ-0304: per-game map-panel visibility. `Some(false)` → start with the map
    // hidden (captured here before `cfg` is moved into the engine build below).
    let start_map_hidden = app::styles::read_per_game_show_map(&game_dir) == Some(false);
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
        app::styles::read_per_game_v6_pixel_lock(&game_dir).filter(|_| cli.v6_pixel_lock.is_none())
    {
        cfg.v6_pixel_lock = v;
        cfg.one_run.pin(app::config::keys::V6_PIXEL_LOCK, v);
    }
    // SQ-1123: the border controls persist what they switch, so the two switches
    // that were session-only until now arrive with the game as well. Same
    // precedence and the same pin as the pixel lock above — a flag typed on this
    // launch outranks a file, and one game's choice can never be written back
    // into the user's global config.toml by a later settings-screen save.
    let guidance_base = cfg.guidance;
    if let Some(v) = app::styles::read_per_game_guidance(&game_dir).filter(|_| cli.guidance.is_none())
    {
        cfg.guidance = v;
        cfg.one_run.pin(app::config::keys::GUIDANCE, v);
    }
    // SQ-0785: the return probe is off by default and per-game before it is
    // global, for the reason the pixel lock is — how much silent work a story is
    // worth is a fact about the story.
    let return_probe_base = cfg.return_probe;
    if let Some(v) = app::styles::read_per_game_return_probe(&game_dir) {
        cfg.return_probe = v;
        cfg.one_run.pin(app::config::keys::RETURN_PROBE, v);
    }
    let v6_render_base = cfg.v6_render;
    if let Some(m) = app::styles::read_per_game_v6_render(&game_dir)
        .filter(|_| cli.v6_render.is_none())
        .and_then(|t| app::config::v6_render_from_key(&t))
    {
        cfg.v6_render = m;
        cfg.one_run.pin(app::config::keys::V6_RENDER, app::config::v6_render_key(m));
    }
    let theme_colours = app::glk_backend::theme_style_colours(&cs);
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
    let mut host_default_colours = app::colors::host_default_colours(
        &cfg,
        cfg.machine_default_colours(),
        cs.theme.get("transcript").style,
        term_default_colors.fg.map(|c| (c.0[0], c.0[1], c.0[2])),
        term_default_colors.bg.map(|c| (c.0[0], c.0[1], c.0[2])),
    );
    // SQ-0679/SQ-0680: the real story-pane `(rows, cols)`, measured before the
    // engine exists, so a v4/v5 story's boot-time status-bar layout already
    // targets it instead of the zvm 80×24 fallback. `None` (size query failed,
    // or a zero-area frame) leaves the constructor's existing fallback in place.
    // The map's visibility is resolved above, and it MUST reach the width the
    // story is told (SQ-1084) — see `pre_boot_host_screen`.
    let boot_layout = if start_map_hidden {
        app::state::Layout::TranscriptFull
    } else {
        app::state::Layout::Split
    };
    let host_screen = pre_boot_host_screen(&cfg, &cs, &garglk_overlay, boot_layout);

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
    let mut launch_text_face: Option<app::native_font::TextFace> = None;
    // The story's Version, for SQ-0873's period look — which belongs only to a
    // v1-v4 story, since colour arrives with v5 and anything shown before it is
    // presentation rather than a fact the story can read. `None` for Glulx and
    // Scott Adams, which have no §11.1.3 machine to have a look of.
    let mut story_zversion: Option<u8> = None;
    // Build the engine: a Z-machine GameSession for Z-code, a GlulxSession for
    // Glulx — both boxed behind the neutral Engine trait. Z-machine-specific
    // setup (screen dims, undo cap) runs in its arm before boxing.
    let mut session: Box<dyn Engine> = match loaded {
        app::hints::LoadedStory::ZCode(bytes) => {
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
                app::graphics::PictSource::resolve_with_override(
                    &story_path,
                    picture_override,
                    disk_entry,
                )
            } else {
                app::graphics::PictSource::new(None)
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
                cfg.one_run.pin(app::config::keys::HONOR_GAME_COLOURS, false);
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
                zvm::screen::set_palette(palette);
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
            let user_disks = app::system_fonts::UserDisks::new(&cfg.system_font_disk);
            let launch_faces = app::native_font::resolve(&app::native_font::FaceRequest {
                story_path: &story_path,
                entry: disk_entry,
                profile: cfg.interpreter_profile,
                source: cfg.interpreter_source,
                art_scale: picts.art_scale(),
                disks: Some(&user_disks),
            });
            let boot = app::machine_boot::MachineBoot::resolve(
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
            let mut s = match GameSession::new_for_machine(bytes, cfg.honor_game_colours, cfg.enable_sound, cli.debug, picture_dims, host_screen, Some(random_seed), &boot) {
                Ok(s) => s,
                Err(e) => {
                    use zvm::error::ZError;
                    let msg = match e {
                        ZError::UnsupportedVersion(v) => format!("unsupported Z-machine version {v}"),
                        ZError::NotAStoryFile => "file is not a valid Z-machine story file".to_string(),
                        ZError::Truncated => "story file is truncated".to_string(),
                        _ => format!("{e:?}"),
                    };
                    eprintln!("lanthorn: {msg}");
                    std::process::exit(1);
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
        app::hints::LoadedStory::Glulx(bytes) => {
            let pict_blorb = resolve_pict_blorb(&story_path, cfg.images);
            match GlulxSession::new_in(
                game_dir.clone(),
                bytes,
                cfg.virtual_screen_cols.unwrap_or(app::config::FALLBACK_SCREEN_COLS) as u32,
                cfg.virtual_screen_rows.unwrap_or(app::config::FALLBACK_SCREEN_ROWS) as u32,
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
                cli.debug,
                Some(random_seed),
            ) {
                Ok(s) => Box::new(s),
                Err(e) => {
                    eprintln!("lanthorn: cannot load Glulx story: {e:?}");
                    std::process::exit(1);
                }
            }
        }
        app::hints::LoadedStory::Scott(bytes) => match app::scott_session::ScottSession::new_with_trace(
            bytes,
            resolve_pict_blorb(&story_path, cfg.images),
            // `--debug` (SQ-0449/SQ-0464): trace from boot so the opening
            // occurrence pass (run inside the VM constructor) is captured.
            cli.debug,
            Some(random_seed),
        ) {
            Ok(s) => Box::new(s),
            Err(e) => {
                eprintln!("lanthorn: cannot load Scott Adams story: {e}");
                std::process::exit(1);
            }
        },
    };
    // Strip the game's own inline read prompt only when the dedicated command
    // bar is on (SQ-0264); otherwise inline-prompt mode keeps the game's ">".
    session.set_strip_prompt(cfg.command_bar);

    // `--debug` (SQ-0449): tracing is already on from the first boot instruction
    // (the Z-machine arm used `GameSession::new_with_trace` above), so the boot
    // PCs are already in the cumulative set. Here we just seed prior runs' coverage
    // from the per-story sidecar so those lines colour immediately too.
    if cli.debug {
        let loaded = app::pcset_store::read_pcs(&game_dir);
        if !loaded.is_empty() {
            session.seed_executed_pcs(&loaded);
        }
    }

    // Engine is up — stop the loading spinner and let it erase its line.
    loading_done.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = loading_spinner.join();

    // SQ-0319: announce the imported garglk config (after the spinner erased its
    // line, so the message isn't clobbered). Printed only when a sidecar applied.
    if let Some(line) = &garglk_line {
        eprintln!("lanthorn: {line}");
    }

    // SQ-0811: name the seed the story just booted on. A run that turns out
    // interesting is only replayable if the player can find out what it was
    // seeded with, and this is the last moment before the alternate screen takes
    // the terminal — so it stays in the scrollback afterwards, like the warnings
    // above it. Said on every launch, because the interesting run is never the
    // one you thought to ask about beforehand.
    eprintln!("lanthorn: {}", random_seed_line(random_seed, cfg.random_seed.is_some()));

    // ── 2. IFID + map dir + load/create mapper ────────────────────────────────

    // `ifid` was computed above (before the engine build) so the per-game honor
    // override could feed the engine; `game_dir` (per-story storage) was computed
    // and created before the engine build too. The IFID stays for title/hint/
    // display and the per-game style reload below.
    let arc_file = default_state_path(&game_dir);

    // Load mapper (and optionally restore the game save) from the archive.
    let mut startup_transcript: app::state::LoadedTranscript = None;
    // Rewind/replay history carried from the archive when the game is auto-restored.
    let mut startup_history: Vec<std::sync::Arc<app::history::TurnRecord>> = Vec::new();
    // Command history (Up/Down recall) carried from the archive, always loaded.
    let mut startup_command_history: Vec<String> = Vec::new();
    // Turn counter carried from the archive when the game is auto-restored, so a
    // later save records the cumulative count rather than only post-resume moves.
    let mut startup_turns: Option<u32> = None;
    // When auto_load is false but a save exists and prompt_load_on_launch is true,
    // stash the save for the launch dialog instead of discarding it.
    let mut pending_resume_stash: app::state::PendingResume = None;
    let mut mapper = if arc_file.exists() {
        match load_archive(&arc_file) {
            Ok(ac) => {
                // Restore the machine from the saved game state only when auto_load is enabled.
                if cfg.auto_load {
                    match session.restore_state(&ac.engine_save()) {
                        Ok(()) => {
                            if let Some(scr) = ac.screen.clone() {
                                if let Some(zs) = zvm_session_opt_mut(&mut *session) {
                                    app::session::restore_screen(zs, scr);
                                }
                            }
                            // The v6 screen: display list where the archive has one
                            // (SQ-0588), else canvas PNGs. No-op for non-v6 archives.
                            crate::engine_helpers::apply_v6_pictures(&mut *session, &ac);
                            // Hand Glulx back the room it was saved in (SQ-0523);
                            // no-op for zvm.
                            crate::engine_helpers::seed_resumed_location(&mut *session, &ac.meta);
                            startup_transcript = Some((ac.transcript, ac.transcript_kinds, ac.transcript_runs, ac.transcript_para, ac.transcript_images));
                            startup_history = ac.history;
                            // Restore the turn counter from the same archive (SQ-0429):
                            // the auto_load resume path mirrors the interactive restore,
                            // which sets state.turns = ac.meta.turns. Without this, a
                            // resumed game's later save records only post-resume moves.
                            startup_turns = Some(ac.meta.turns);
                        }
                        Err(e) => {
                            eprintln!("lanthorn: warning: could not restore game from archive: {}; starting fresh", restore_error_msg(e));
                        }
                    }
                } else if cfg.prompt_load_on_launch && !ac.save.is_empty() {
                    pending_resume_stash = Some((ac.engine_save(), ac.transcript, ac.transcript_kinds, ac.screen));
                }
                if cfg.aux_storage != app::config::AuxStorage::Global {
                    session.set_aux_data(ac.aux.clone());
                }
                startup_command_history = ac.command_history;
                // The map is part of the game's state: it loads only when the state is
                // auto-resumed here. When auto_load is off it either rides the launch-resume
                // dialog (adopted on accept, see apply_launch_resume) or stays blank.
                if cfg.auto_load { ac.mapper } else { Mapper::default() }
            }
            Err(e) => {
                eprintln!("lanthorn: warning: could not load archive {}: {}", arc_file.display(), e);
                Mapper::default()
            }
        }
    } else {
        Mapper::default()
    };

    // Startup: pre-load the per-game aux table from the global file when in
    // global mode.  In archive mode the table was populated above from the
    // loaded archive (if any).
    if cfg.aux_storage == app::config::AuxStorage::Global {
        session.set_aux_data(app::aux_store::read_global_aux(&game_dir));
    }

    // The per-story Glk file VFS sidecar was loaded into the VM before boot
    // (GlulxSession::new). A Glulx game may write a Glk file during boot (e.g.
    // CM's init cache); flush it now so it persists before the first turn and
    // survives an immediate quit (SQ-0290). For a Z-machine session vfs_dirty()
    // is always false, so this is a no-op there.
    if session.vfs_dirty() {
        let _ = app::vfs_store::write_vfs(&game_dir, &session.vfs_bytes());
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
    let (keymap, keymap_warnings) = app::keymap::KeyMap::resolve(&cfg.keymap);
    state.keymap = keymap;
    // Surface any keymap conflict warnings once in the transcript.
    for w in keymap_warnings {
        state.push_notice(&format!("[{}]", w));
    }
    let (hotkeys, hotkey_warnings) = app::keymap::HotkeyLayout::resolve(&cfg.hotkeys);
    state.hotkeys = hotkeys;
    for w in hotkey_warnings {
        state.push_notice(&format!("[{}]", w));
    }
    state.show_room_numbers = cfg.show_room_numbers;
    state.show_status_bar = cfg.show_status_bar;
    state.game_picker = game_picker;
    state.term_default_colors = term_default_colors;
    state.query_sweep = query_sweep;
    state.pane_sizes = app::state::PaneSizes {
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
    let initial_panel = app::styles::read_per_game_panel(&game_dir).unwrap_or(
        if cfg.command_band.auto_open {
            app::state::SidePanel::Command
        } else {
            app::state::SidePanel::None
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
    state.game_colours_cli = cli.game_colours.map(bool::from);
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

    // Debug trace (trace feature): start a fresh log for this run and arm the
    // engine's screen-trace buffer per config; no-op when no section is active.
    if state.config.trace.any() {
        app::trace::truncate(&state.config.user_dir);
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
    state.disk_sounds = app::native_sound::from_medium(&story_path);
    if !state.disk_sounds.is_empty() {
        let mut effects: Vec<u16> = state.disk_sounds.keys().copied().collect();
        effects.sort_unstable();
        eprintln!(
            "lanthorn: {} sound effect{} on the medium ({})",
            effects.len(),
            if effects.len() == 1 { "" } else { "s" },
            effects.iter().map(u16::to_string).collect::<Vec<_>>().join(", "),
        );
    }
    state.sound_blorb = match app::graphics::resource_blorb(&story_path).found {
        Some((b, path)) => {
            let count = |usage: &[u8; 4]| b.resources().iter().filter(|r| &r.usage == usage).count();
            let (sounds, images) = (count(b"Snd "), count(b"Pict"));
            let own = path == story_path;
            eprintln!(
                "lanthorn: loaded resources from {}{} ({} sound{}, {} image{})",
                path.display(),
                if own { " (self)" } else { " (sidecar)" },
                sounds, if sounds == 1 { "" } else { "s" },
                images, if images == 1 { "" } else { "s" },
            );
            Some(b)
        }
        None => None,
    };
    if state.config.enable_sound {
        state.audio = Some(audio::AudioBackend::new(state.config.volume));
    }

    // Seed autocomplete with the story's parser vocabulary (room nouns are added live).
    state.dict_words = session.introspect().map(|i| i.vocabulary()).unwrap_or_default();

    // Open whichever panel this story starts with (SQ-1123, widened to a
    // three-state cycle by SQ-1237): the per-game override, or the global
    // `[command_panel] auto_open` fallback resolved into `initial_panel` above.
    // Instant (no slide) so the first frame is already the settled layout.
    match initial_panel {
        app::state::SidePanel::Command => {
            let mut mapper_noop = mapper::mapper::Mapper::default();
            // Not through `Action::OpenCommandBand`: that action PERSISTS the
            // panel state per-game (SQ-1123), and a global `auto_open` must not
            // pin itself to whichever story you happened to launch. The state
            // change without the persistence is exactly what this helper is.
            app::input::open_command_band(&mut state, &mut mapper_noop, true);
            state.band_dock.toggle_to(true, true);
        }
        app::state::SidePanel::Inventory => {
            // Same non-persisting rule as the command panel above.
            app::input::open_inventory_panel(&mut state, true);
            state.inv_dock.toggle_to(true, true);
        }
        app::state::SidePanel::None => {}
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
                app::session::TranscriptElem::Text { text, .. } => Some(text.as_str()),
                app::session::TranscriptElem::Image(_)
                | app::session::TranscriptElem::ScreenClear => None,
            })
            .collect()
    };
    let banner_title = app::session::title_from_banner(&banner);
    // SQ-0766: ask the story browser's own metadata resolver first — the `IFmd`
    // chunk, the fetched IFDB sidecar, then the bundled tables — so the pane
    // names the game the way the list does. The banner heuristic is the tier
    // below it, and the filename stem is the last resort it was meant to be.
    let meta_title = app::picker::metadata_title_in(&story_path, &game_dir, &ifid, is_scott);
    state.title =
        app::session::resolve_title(None, meta_title.as_deref(), banner_title.as_deref(), &story_path);
    let story_filename = story_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    state.pane_title =
        app::session::format_pane_title(&state.title, story_filename, disk_image.is_some());
    state.ifid = ifid.clone();
    state.game_dir = game_dir.clone();
    // Restore the per-game map-panel visibility (SQ-0304): if the user last hid
    // the map for this story, start with it hidden.
    if start_map_hidden {
        state.layout = app::state::Layout::TranscriptFull;
    }
    // Now that game_dir is set, re-resolve through reload_style so the per-game
    // override (<game_dir>/style.toml) is merged over the global at startup — the
    // initial resolve above is global-only (game_dir wasn't set yet). On a per-game
    // parse error the global look already set above stands.
    let _ = app::reload::reload_style(&mut state);
    if banner_elems.is_empty() {
        state.push_transcript(&banner);
    } else {
        app::state::apply_transcript_elems(&mut state, &banner_elems);
    }

    // The opening room description is already on screen, so its words are already
    // completable — waiting for the first turn would leave Tab with only the flat
    // dictionary for exactly the move a player is most likely to want help with
    // (SQ-1116).
    app::input::refresh_seen_words(&mut state, &*session);
    app::input::refresh_scope_words(&mut state, &*session);

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
        state.push_transcript_internal(&msg, app::state::TranscriptKind::Warning);
    }

    // SQ-0734: the per-game `pictures` key named an archive that is missing or
    // will not decode. Surfaced the same way a broken config.toml is — a warning
    // line in the transcript, which stays put instead of expiring like a toast —
    // because the alternative is a player who thinks they are seeing the native
    // art they asked for and is quietly seeing the Blorb's instead.
    if let Some(msg) = picture_warning {
        state.push_transcript_internal(&msg, app::state::TranscriptKind::Warning);
    }

    // SQ-0663: the theme just rebuilt by reload_style above may carry its own
    // non-fatal diagnostics (a `parent` re-root naming an unknown selector, or
    // a cycle — see `Theme::warnings`), which used to fall back to registry
    // defaults with no visible sign anything was wrong. Surface them the same
    // way the broken-config.toml case just above is surfaced: one transcript
    // Warning line per issue (collapsed to a summary beyond a few — see
    // `describe_theme_warnings`), so a typo like `parent = "acent"` in
    // style.toml no longer degrades silently.
    for line in app::theme::resolve::describe_theme_warnings(state.colors.theme.warnings()) {
        state.push_transcript_internal(&line, app::state::TranscriptKind::Warning);
    }

    // One-time notice: config.toml no longer carries style — those moved to style.toml.
    if let Ok(raw_cfg) = std::fs::read_to_string(app::config::config_path(cli)) {
        if app::config::config_has_style_sections(&raw_cfg) {
            state.push_transcript_internal(
                "config.toml [colors]/[symbols] are no longer used — styling lives in style.toml ([colors] there; map glyph presets are now [map] keys)",
                app::state::TranscriptKind::Warning,
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
        crate::turn::flush_screen_trace(&state.config.user_dir, &mut *session, state.config.trace.screen);
        crate::turn::flush_v6_trace(&state.config.user_dir, &mut *session, state.config.trace.v6);
        if state.config.trace.any() {
            let ptr = format!(
                "[trace → {}: {}]",
                state.config.user_dir.join("trace.log").display(),
                state.config.trace.active_list(),
            );
            state.push_transcript_internal(&ptr, app::state::TranscriptKind::Meta);
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
    if startup_transcript.is_none()
        && app::pager::should_arm(session.pending_input(), app::pager::more_suppressed(&*session))
    {
        state.pager.arm(0);
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
        app::input::refresh_seen_words(&mut state, &*session);
    }
    if !startup_history.is_empty() {
        state.history = startup_history;
    }
    state.command_history = startup_command_history;
    if let Some(turns) = startup_turns {
        state.turns = turns;
    }

    // SQ-1086: this story came off a URL, so offer to keep it. Raised BEFORE the
    // resume prompt below so that prompt's `dialog_focus = 0` wins while it is
    // up — it sits above this one in the ladder and has to be answered first.
    // Only offered when there is a library to keep it IN: `default_story_dir` is
    // the directory the picker reads, and inventing another location would put
    // the file somewhere nothing lists.
    if ctx.fetched.as_ref().is_some_and(|f| f.path == story_path) {
        let fetched = ctx.fetched.clone().expect("checked just above");
        match state.config.default_story_dir.clone() {
            Some(library_dir) => {
                let collision = app::story_url::library_collision(&fetched.path, &library_dir);
                state.overlays.fetch_keep = Some(app::state::FetchKeepPrompt {
                    fetched,
                    library_dir,
                    collision,
                    // A story, not an archive: an archive never gets this far —
                    // it is answered before `boot_story` is called at all
                    // (SQ-1096, `unpack_fetched_archive`).
                    disk_images: Vec::new(),
                });
                state.overlays.dialog_focus = 0;
            }
            None => state.push_notice(
                "[Downloaded to a temporary folder. Set `default_story_dir` in your config to keep fetched stories.]",
            ),
        }
    }

    // If a save was found but auto_load is off and prompt_load_on_launch is on,
    // open the launch dialog so the user can choose to resume or start fresh.
    if let Some(stash) = pending_resume_stash {
        state.pending_resume = Some(stash);
        state.overlays.launch_dialog = true;
        state.overlays.dialog_focus = 0;
    }

    // If the game quit immediately (e.g. czech.z5 or the glk-dev self-checking
    // file tests), bail without entering raw mode. Such stories run to completion
    // and quit before ever asking for input, so their output IS the point — print
    // the captured transcript to stdout instead of discarding it.
    if session.has_quit() {
        for line in &state.transcript {
            println!("{}", line);
        }
        eprintln!("lanthorn: story ended without asking for input.");
        std::process::exit(0);
    }

    // `--debug` (SQ-0449): persist the cumulative coverage on story-end, and
    // auto-open the debug inspector now (mirrors `/debug`'s open recipe). Tracing
    // was already enabled above; `set_debug_trace(true)` here is idempotent.
    state.persist_debug_trace = cli.debug;
    if cli.debug && session.debugger().is_some() {
        session.set_debug_trace(true);
        let dbg = session.debugger().expect("checked above");
        let mut panel = app::debug_panel::DebugPanelState::new(dbg.pc());
        panel.apply_engine_layout(dbg);
        panel.refresh(dbg);
        state.debug = Some(panel);
        state.focus = app::state::Focus::Map;
    }

    // ── 4. Terminal setup ─────────────────────────────────────────────────────

    // Install the panic hook FIRST so that any panic after this point (including
    // one between enable_raw_mode and EnterAlternateScreen) restores the terminal.
    install_panic_hook(state.config.user_dir.clone());

    // SQ-0586: from here until teardown, fd 2 goes to <user_dir>/stderr.log instead
    // of the terminal. C libraries (libasound through rodio/cpal) write there
    // directly — no Rust hook can catch them — and an ALSA underrun repeated during
    // power-save lands mid-frame and corrupts the render. Installed AFTER the panic
    // hook, whose `restore_terminal` puts fd 2 back before it prints, and after the
    // CLI/picker phases so ordinary terminal output is unaffected. A failure here is
    // not worth refusing to start over: the game runs, the chatter just stays visible.
    if let Err(e) = app::stderr_redirect::install(&state.config.user_dir.join("stderr.log")) {
        eprintln!("lanthorn: could not redirect OS error output ({e}); it may corrupt the display");
    }

    if let Err(e) = enable_raw_mode() {
        eprintln!("lanthorn: cannot enable raw mode (not a TTY?): {}", e);
        std::process::exit(1);
    }

    // From here on, raw mode is active — MUST restore on every exit path.

    if let Err(e) = execute!(stdout(), EnterAlternateScreen) {
        restore_terminal();
        eprintln!("lanthorn: cannot enter alternate screen: {}", e);
        std::process::exit(1);
    }
    // Bracketed paste (SQ-0653). Without it the terminal replays a paste as raw
    // keystrokes, and the app cannot tell them from typing: a Tab fired
    // autocomplete, a leading '/' opened the command palette, and every newline
    // SUBMITTED a line to the game — so pasting a walkthrough played it. With the
    // mode on, the paste arrives as one `Event::Paste` and lands in the focused
    // field as literal text. Best-effort: a terminal that ignores the sequence
    // simply never sends `Event::Paste`, which is exactly today's behavior.
    // `restore_terminal()` always issues DisableBracketedPaste.
    let _ = execute!(stdout(), EnableBracketedPaste);

    // Mouse capture is opt-in (config `mouse = true`). Capture puts the terminal
    // in any-motion reporting mode, so every mouse movement wakes the event loop
    // and forces a full redraw; leaving it off keeps idle/scroll responsive and
    // preserves the terminal's native text selection. restore_terminal() always
    // issues DisableMouseCapture, which is a harmless no-op when it was never on.
    if state.config.mouse {
        let _ = execute!(stdout(), EnableMouseCapture);
    }

    // The fork-and-probe seam (SQ-1121). Armed with the story's own bytes and the
    // boot facts that change how it runs, so the shadow a vetted suggestion is
    // tried in is the SAME game on the SAME machine — a shadow that differs in
    // any of them answers plausibly about a game the player is not playing. The
    // shadow itself is not booted here: most sessions never ask it anything.
    state.probe.arm(app::probe::ShadowRecipe {
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
            state.config.virtual_screen_cols.unwrap_or(app::config::FALLBACK_SCREEN_COLS) as u32,
            state.config.virtual_screen_rows.unwrap_or(app::config::FALLBACK_SCREEN_ROWS) as u32,
        ),
    });

    // Every byte the backend writes is counted on the way out, so `/dump-terminal`
    // can answer "why does this feel slow?" with numbers (SQ-0994). The handle is
    // shared with `AppState`, which is the only thing that reads them; the
    // `execute!(stdout(), …)` escapes above deliberately bypass it, because they
    // are session setup rather than frame traffic.
    let traffic: app::terminal_dump::TrafficHandle = Default::default();
    state.term_traffic = Some(std::sync::Arc::clone(&traffic));
    // And buffered before it reaches the tty (SQ-1192): raw `Stdout` is a
    // mutex-locked LineWriter with a ~1 KiB buffer, so a dense frame was
    // thousands of lock/flush rounds — one per queued crossterm command. The
    // buffer sits INSIDE the counter so the traffic numbers keep meaning what
    // they meant: bytes when the backend writes them, a flush per drawn frame.
    // Writes larger than the buffer (a base64 image transmit) bypass it whole.
    let terminal = match Terminal::new(CrosstermBackend::new(app::terminal_dump::CountingWriter::new(
        std::io::BufWriter::with_capacity(256 * 1024, stdout()),
        traffic,
    ))) {
        Ok(t) => t,
        Err(e) => {
            restore_terminal();
            eprintln!("lanthorn: cannot create terminal: {}", e);
            std::process::exit(1);
        }
    };

    BootResult {
        session,
        mapper,
        state,
        terminal,
        game_dir,
        ifid,
        arc_file,
        story_bytes,
        story_path,
        data_base,
    }
}

/// Cooked-mode y/N prompt on the normal terminal (before the alt-screen is
/// entered). A non-interactive stdin (piped or EOF) reads as "no".
fn prompt_yes_no(question: &str) -> bool {
    use std::io::Write as _;
    // THE CONSOLE MAY NOT BE ABLE TO GIVE US A LINE (SQ-1007).
    //
    // `read_line` does not read keys; it waits for the console driver to hand it
    // an assembled line, which the driver only does with `ENABLE_LINE_INPUT` and
    // `ENABLE_ECHO_INPUT` set. In raw mode those are off, so every keystroke
    // vanishes and the call blocks for ever. This prompt runs BEFORE anything
    // else in lanthorn touches the terminal — the colour query is at
    // `query_terminal_default_colors`, raw mode at `enable_raw_mode`, both far
    // below — so it inherits whatever the console was left in, and on Windows a
    // console's input mode outlives the process that set it.
    //
    // Reported on 0.2.0: the first launch in a fresh terminal answered normally,
    // and a second launch in the SAME window ignored every keypress. Only Ctrl-Z
    // got through — the console's EOF signal — so `read_line` returned `Ok(0)`,
    // the arm below read that as "no", and startup carried on to the story list.
    //
    // Three observations, and between them they name the three bits crossterm
    // clears for raw mode (`NOT_RAW_MODE_MASK`) one at a time, which is what
    // makes this a diagnosis rather than a guess:
    //
    //   * nothing echoed             → `ENABLE_ECHO_INPUT` is off
    //   * keys never formed a line   → `ENABLE_LINE_INPUT` is off
    //   * Ctrl-C did not interrupt   → `ENABLE_PROCESSED_INPUT` is off, since
    //     Windows only raises CTRL_C_EVENT when it is set and otherwise delivers
    //     a plain 0x03 byte
    //
    // The shell looking FINE in between is not evidence against that, though it
    // reads like it: PSReadLine reads key events itself and draws the line it is
    // editing, so a console left raw behaves normally there. lanthorn's own TUI is
    // immune for the same reason — it sets raw mode deliberately. This prompt is
    // the one cooked-mode consumer in the whole program, which is why it is the
    // only thing that broke.
    //
    // One call fixes it, for different reasons on each platform. crossterm's
    // Windows `disable_raw_mode` SETS the cooked bits (`mode | LINE | ECHO |
    // PROCESSED`) rather than restoring a mode it saved earlier, so it repairs a
    // console this process never broke. On unix it restores the termios saved at
    // `enable_raw_mode` and is a no-op when there is none — which is always here,
    // since nothing has enabled raw mode yet. Unix untouched, Windows repaired,
    // and neither depending on the previous run having exited tidily.
    let _ = crossterm::terminal::disable_raw_mode();
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) | Err(_) => false,
        Ok(_) => matches!(line.trim().chars().next(), Some('y') | Some('Y')),
    }
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::should_ask_font_check;
    use app::config::OnOff;

    /// SQ-1112: the reported bug, and the guard that made it hard to fix.
    ///
    /// The bug is the third case. A first launch that could not show the prompt
    /// used to leave nothing behind, and `config.toml` was seeded on that same
    /// launch regardless — so "there is no config.toml", which IS the first-run
    /// flag, was spent by a launch that never asked anything. Every later
    /// interactive run then read `first_run = false` and stayed silent.
    ///
    /// The FIRST case is why the fix could not simply be "ask until answered".
    /// The test harnesses seed an empty `config.toml` precisely so `first_run` is
    /// false, and an empty file parses with every key at its default — so the
    /// default has to mean "nothing owed", or SQ-1104's guard 2 breaks and the
    /// prompt reappears in front of fourteen group binaries. Owing is opt-in.
    #[test]
    fn a_font_check_is_owed_only_when_a_launch_could_not_ask() {
        // A seeded harness home: not a first run, nothing owed. Silence.
        assert!(!should_ask_font_check(None, false, false));
        // A genuine first run asks, exactly as it always did.
        assert!(should_ask_font_check(None, true, false));
        // …and a later run asks when a previous one could not — the fix.
        assert!(should_ask_font_check(None, false, true));
    }

    /// The flag is an override in both directions and consults nothing.
    ///
    /// `off` has to beat a pending note or there is no way to say "stop asking",
    /// and `on` has to beat a settled config or `--font-check on` could not be
    /// the answer to "I changed terminal fonts", which is what it is for.
    #[test]
    fn the_flag_outranks_both_the_first_run_and_the_owed_note() {
        for first_run in [true, false] {
            for pending in [true, false] {
                assert!(
                    !should_ask_font_check(Some(OnOff::Off), first_run, pending),
                    "off never asks (first_run={first_run}, pending={pending})"
                );
                assert!(
                    should_ask_font_check(Some(OnOff::On), first_run, pending),
                    "on always asks (first_run={first_run}, pending={pending})"
                );
            }
        }
    }
}
