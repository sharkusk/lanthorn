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
//!
//! Since SQ-1537 the terminal-independent half of the per-story build is the
//! library's [`app::host::boot_story`]; [`boot_story`] here probes the terminal,
//! calls it, and then sets the terminal up around what it returns.

use std::io::{stdout, Stdout};

use crossterm::event::{EnableBracketedPaste, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{enable_raw_mode, EnterAlternateScreen};
use mapper::mapper::Mapper;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use clap::Parser;

use app::config::{config_path, resolve, Cli, Config, OnOff};
use app::engine::Engine;
use app::state::AppState;

use crate::{install_panic_hook, loading_line, picker_ui, restore_terminal, saves_dir};

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
    // No story is booted yet, so there is no machine to resolve a colour number
    // through: §8.3.1's own table (SQ-1393).
    let (colors, _syms, _w2) =
        app::style::resolve(&base, &cfg.user_dir, zvm::screen::Palette::Standard);

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
    // No story is booted yet, so there is no machine to resolve a colour number
    // through: §8.3.1's own table (SQ-1393).
    let (colors, _syms, _w2) =
        app::style::resolve(&base, &cfg.user_dir, zvm::screen::Palette::Standard);

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

/// Re-issue bracketed paste and (when `mouse` is on) mouse capture.
///
/// Written once here so the launch path below and every `Event::Resize` arm
/// call the SAME two `execute!`s instead of hand-copying them — they cannot
/// drift apart, and this is also why both the launch site's original modes
/// and a resize's re-assertion trace to one function.
///
/// The resize call exists for the web image (SQ-1340): `docker/serve-session.sh`
/// runs the game inside `dtach -A ... -r winch`, and a browser tab reattaching
/// to that session gets a brand-new xterm.js instance that never saw the
/// escapes this function's launch-site caller sent at boot — mouse capture and
/// bracketed paste are per-terminal state, not per-session state, and dtach
/// only reconnects the byte stream, not the terminal mode. `-r winch` delivers
/// SIGWINCH on every attach, which crossterm surfaces as `Event::Resize`, so a
/// resize is the only hook a reattach gives us. Before this fix, the picker
/// happened to be the sole thing re-enabling mouse capture (it does so
/// unconditionally on open), which is why a reattached iPad regained mouse
/// input only after opening the story list, and never regained bracketed
/// paste at all. Both sequences are idempotent on every terminal we support,
/// so calling this on a plain local resize (no reattach involved) is harmless.
pub(crate) fn reassert_terminal_modes<W: std::io::Write>(w: &mut W, mouse: bool) -> std::io::Result<()> {
    execute!(w, EnableBracketedPaste)?;
    if mouse {
        execute!(w, EnableMouseCapture)?;
    }
    Ok(())
}

/// What the TUI does while [`app::host::boot_story`] runs: print its console
/// lines on the ordinary terminal (the alternate screen is not up yet, so they
/// stay in the scrollback), and spin a loading indicator while the engine builds.
#[derive(Default)]
struct TuiBootHooks {
    spinner: Option<(std::sync::Arc<std::sync::atomic::AtomicBool>, std::thread::JoinHandle<()>)>,
}

impl app::host::BootHooks for TuiBootHooks {
    fn console(&mut self, line: &str) {
        eprintln!("lanthorn: {line}");
    }

    // Booting a large story to its first prompt can take several seconds, and this
    // happens before the alternate screen is entered — so the normal terminal would
    // otherwise sit frozen. Spin a tiny indicator on a side thread; it only starts
    // drawing after a short grace period, so quick loads never flicker.
    fn engine_starting(&mut self, story: &std::path::Path, bytes: usize) {
        use std::io::Write as _;
        use std::sync::atomic::Ordering;
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = done.clone();
        let name = story.display().to_string();
        let handle = std::thread::spawn(move || {
            const FRAMES: [char; 4] = ['|', '/', '-', '\\'];
            const TICK_MS: u64 = 60;
            let (mut waited, mut i, mut shown) = (0u64, 0usize, false);
            while !flag.load(Ordering::Relaxed) {
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
        });
        self.spinner = Some((done, handle));
    }

    // Engine is up — stop the loading spinner and let it erase its line.
    fn engine_ready(&mut self) {
        if let Some((done, handle)) = self.spinner.take() {
            done.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = handle.join();
        }
    }
}

/// Build the per-story engine + mapper + UI state + terminal for `story_path`,
/// using the one-time [`LaunchCtx`]. The terminal-independent build is
/// [`app::host::boot_story`] (SQ-1537), which a headless host calls too; this
/// wraps it with what only a terminal can do — the image-protocol and OSC colour
/// probes going in, and the alternate screen coming out. A library launch calls
/// it once per chosen story. May `std::process::exit` on an unrecoverable
/// per-story error (unreadable/invalid story, terminal init failure) exactly as
/// before.
pub(crate) fn boot_story(
    ctx: &LaunchCtx,
    story_path: std::path::PathBuf,
    disk_entry: Option<&str>,
    overrides: &app::launch_options::LaunchOverrides,
) -> BootResult {
    let cfg = &ctx.cfg;
    // In-game graphics Picker (None when --images off or unavailable) — probed
    // here because it queries the terminal; the boot reuses it for the Glulx
    // session's char-cell pixel size, the Scott pictures and `AppState`.
    let game_picker = if cfg.images { picker_ui::build_cover_picker(cfg.image_protocol, cfg.kitty_shared_memory) } else { None };
    // SQ-1511: did that build's query — if it ran one — get any answer at all?
    // See `picker_ui::picker_query_answered`'s doc (SQ-1520 shared it with
    // `run_story_picker`'s own cover-art preview picker) for why this is read
    // the same way `loop_tick::poll_picker_requery` skips a font-change
    // requery on either case rather than paying its stdio round trip on a
    // terminal that will only ever answer with nothing.
    let game_picker_query_answered = picker_ui::picker_query_answered(game_picker.as_ref());
    // Probe the terminal's own default fg/bg (OSC 10/11) in the same pre-UI query
    // window as the image-protocol Picker above (SQ-0510). Seeds the v6 raster
    // canvas's default ink/page so "terminal default" theme colours follow the
    // real terminal instead of a hardcoded light-grey-on-black. Never hangs;
    // terminals that don't answer leave both as None and keep today's fallbacks.
    // SQ-0769: the probe hands back a sweep as well as the colours. A terminal
    // busy with the picker's last frame answers after the drain has given up, and
    // the sweep is what keeps those replies out of the story — see `query_sweep`.
    let (term_default_colors, query_sweep) = app::term_colors::query_terminal_default_colors();
    let terminal = app::host::TerminalFacts {
        game_picker,
        game_picker_query_answered,
        term_default_colors,
        query_sweep,
        // The pane the story is BOOTED with is measured from this (SQ-0679/0680);
        // `None` on a non-terminal stdout keeps the 80x24 fallback.
        size: crossterm::terminal::size().ok(),
    };
    let req = app::host::BootRequest {
        story_path,
        disk_entry,
        overrides,
        cfg: cfg.clone(),
        data_base: ctx.data_base.clone(),
        flags: app::host::LaunchFlags::from(&ctx.cli),
        terminal,
    };
    let mut hooks = TuiBootHooks::default();
    let booted = app::host::boot_story(req, &mut hooks);
    // A failure mid-build must not leave the spinner drawing over the message.
    app::host::BootHooks::engine_ready(&mut hooks);
    let app::host::BootedStory {
        session,
        mapper,
        mut state,
        game_dir,
        ifid,
        arc_file,
        story_bytes,
        story_path,
        data_base,
    } = match booted {
        Ok(b) => b,
        Err(e) => {
            eprintln!("lanthorn: {e}");
            std::process::exit(1);
        }
    };

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
    // Bracketed paste (SQ-0653) and mouse capture (opt-in via config `mouse`),
    // via the shared [`reassert_terminal_modes`] so the launch site and every
    // `Event::Resize` re-assertion (SQ-1340) send the same bytes.
    //
    // Bracketed paste: without it the terminal replays a paste as raw
    // keystrokes, and the app cannot tell them from typing: a Tab fired
    // autocomplete, a leading '/' opened the command palette, and every newline
    // SUBMITTED a line to the game — so pasting a walkthrough played it. With the
    // mode on, the paste arrives as one `Event::Paste` and lands in the focused
    // field as literal text. Best-effort: a terminal that ignores the sequence
    // simply never sends `Event::Paste`, which is exactly today's behavior.
    // `restore_terminal()` always issues DisableBracketedPaste.
    //
    // Mouse capture puts the terminal in any-motion reporting mode, so every
    // mouse movement wakes the event loop and forces a full redraw; leaving it
    // off keeps idle/scroll responsive and preserves the terminal's native text
    // selection. restore_terminal() always issues DisableMouseCapture, which is
    // a harmless no-op when it was never on.
    let _ = reassert_terminal_modes(&mut stdout(), state.config.mouse);

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
    use super::{reassert_terminal_modes, should_ask_font_check};
    use app::config::OnOff;

    /// SQ-1340: a resize re-asserts bracketed paste always, and mouse capture
    /// only when the config asked for it — the same rule the launch site
    /// applies, since both call the one function.
    ///
    /// This assertion is Unix-only. crossterm enables mouse capture on Windows
    /// through the console API, not an escape sequence, so there are no bytes to
    /// assert on there; the function itself is exercised by the launch path on
    /// every platform.
    #[test]
    #[cfg(not(windows))]
    fn reassert_terminal_modes_gates_mouse_on_config() {
        let mut with_mouse = Vec::new();
        reassert_terminal_modes(&mut with_mouse, true).unwrap();
        let with_mouse = String::from_utf8(with_mouse).unwrap();
        assert!(with_mouse.contains("\x1b[?2004h"), "bracketed paste: {with_mouse:?}");
        assert!(with_mouse.contains("\x1b[?1000h"), "mouse capture: {with_mouse:?}");

        let mut without_mouse = Vec::new();
        reassert_terminal_modes(&mut without_mouse, false).unwrap();
        let without_mouse = String::from_utf8(without_mouse).unwrap();
        assert!(without_mouse.contains("\x1b[?2004h"), "bracketed paste: {without_mouse:?}");
        assert!(!without_mouse.contains("\x1b[?1000h"), "mouse capture: {without_mouse:?}");
    }

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
