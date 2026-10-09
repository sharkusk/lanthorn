//! SQ-1539: the rest of a session through the library — game clocks, the
//! pane size, exit save and resume, reset, and the game's own file prompts —
//! with no terminal.
//!
//! | fixture | what | where |
//! |---|---|---|
//! | a hand-assembled v5 story | a timed `read_char` whose routine counts | built below |
//! | `Kerkerkruip.gblorb` | a Glulx intro driven by Glk timer events | `stories/` (skips when absent) |
//! | `Tangle.z5` (Spider and Web r4) | exit save, reset, SAVE/RESTORE | fetched (`fixture_path`) |

use std::path::{Path, PathBuf};
use std::time::Duration;

use app::config::Config;
use app::engine::Engine;
use app::host::ingame_io::{answer_file_prompt, pending_file_prompt, FilePrompt};
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts, TurnOutcome};
use app::launch_options::LaunchOverrides;
use app::session::InputKind;

use crate::fixture_paths::fixture_path;

fn headless_config(home: &Path) -> Config {
    Config {
        user_dir: home.to_path_buf(),
        config_file: home.join("config.toml"),
        random_seed: Some(1),
        auto_save: true,
        ..Config::default()
    }
}

fn boot(story: PathBuf, home: &Path) -> BootedStory {
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: story,
        disk_entry: None,
        overrides: &overrides,
        cfg: headless_config(home),
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: false,
    };
    boot_story(req, &mut QuietBoot).expect("the story boots headlessly")
}

fn command(b: &mut BootedStory, cmd: &str) -> TurnOutcome {
    let result = b.session.submit(cmd);
    let mut tidy = 0u32;
    app::host::finish_command_turn(
        cmd, true, result, &mut b.state, &mut b.mapper, &mut *b.session, &b.game_dir, &b.ifid,
        &b.arc_file, None, &mut tidy,
    )
}

fn here(b: &BootedStory) -> Option<String> {
    b.session.current_location().map(|l| l.name)
}

fn fire(b: &mut BootedStory, at: std::time::Instant) -> app::host::clock::Fired {
    app::host::clock::fire_due(&mut b.state, &mut b.mapper, &mut *b.session, &b.game_dir, None, at)
}

// ── Clocks ────────────────────────────────────────────────────────────────────

/// A v5 story whose `read_char` times out every tenth of a second and runs a
/// routine that counts the timeouts in `G1` and keeps waiting.
///
/// ```text
/// 0x40  read_char 1 1 routine=0x0200 (packed 0x80) -> G0
/// 0x47  quit
/// 0x200 routine: inc G1 ; rfalse
/// ```
fn timed_story() -> Vec<u8> {
    let mut buf = vec![0u8; 0x0800];
    buf[0x00] = 5;
    buf[0x04] = 0x04; // high memory 0x0400
    buf[0x07] = 0x40; // initial PC
    buf[0x08] = 0x04; // dictionary 0x0400 (empty), in static memory as a story's is
    buf[0x401] = 4;
    buf[0x0A] = 0x01; // objects 0x0100
    buf[0x0C] = 0x03; // globals 0x0300
    buf[0x0E] = 0x04; // static memory 0x0400
    buf[0x12..0x18].copy_from_slice(b"260923");
    buf[0x19] = 0x60; // abbreviations
    // read_char: operand types small, small, large, omit = 01 01 00 11.
    let code: &[u8] = &[0xF6, 0x53, 0x01, 0x01, 0x00, 0x80, 0x10, 0xBA];
    buf[0x40..0x40 + code.len()].copy_from_slice(code);
    buf[0x200..0x204].copy_from_slice(&[0x00, 0x95, 0x11, 0xB1]); // 0 locals; inc G1; rfalse
    buf
}

/// The host's clock loop — `refresh_input`, `next_deadline`, `fire_due` — runs a
/// Z-machine story's timed-input routine each time its interval elapses, and
/// the read goes on waiting because the routine said to.
#[test]
fn a_zmachine_timed_read_fires_through_the_host_clock() {
    let home = app::scratch_dir("host-clock-z");
    let story = home.join("timed.z5");
    std::fs::write(&story, timed_story()).unwrap();
    let mut b = boot(story, &home);
    let g1 = |b: &BootedStory| app::engine_helpers::zvm_session_opt(&*b.session).unwrap().machine.global(1);

    let req = app::host::clock::input_request(&*b.session);
    assert_eq!(req.kind, InputKind::Char);
    assert_eq!(req.timeout, Some(Duration::from_millis(100)), "a tenth of a second, ZMSD §15");
    assert_eq!(app::host::clock::next_deadline(&b.state), None, "nothing armed before the host refreshes");

    for n in 1..=3 {
        let _ = app::host::clock::refresh_input(&mut b.state, &mut *b.session);
        let due = app::host::clock::next_deadline(&b.state).expect("the timed read arms a deadline");
        let early = fire(&mut b, due - Duration::from_millis(1));
        assert!(!early.redraw, "nothing fires before its deadline");
        assert_eq!(g1(&b), n - 1);
        let fired = fire(&mut b, due);
        assert!(fired.redraw && !fired.quit);
        assert_eq!(g1(&b), n, "the routine ran once per elapsed interval");
        assert_eq!(b.session.pending_input(), InputKind::Char, "and the read goes on waiting");
    }
    let _ = std::fs::remove_dir_all(&home);
}

/// Kerkerkruip's intro is driven by Glk timer events (see
/// `sq1514_kerkerkruip_panel_links`): its title animation paints into windows
/// of its own rather than the transcript, so what the timer moves is the screen
/// — with nothing but the host clock ticking, the frame changes under it.
#[test]
fn a_glulx_timer_fires_through_the_host_clock() {
    let story = fixture_path("Kerkerkruip.gblorb");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-clock-glulx");
    let mut b = boot(story, &home);
    let _ = app::host::clock::refresh_input(&mut b.state, &mut *b.session);
    let req = app::host::clock::input_request(&*b.session);
    assert!(req.timeout.is_some(), "premise: the intro runs on a Glk timer: {req:?}");
    let frame = |b: &BootedStory| format!("{:?}", b.session.screen().root);
    let before = frame(&b);
    let mut ticks = 0;
    while ticks < 20 {
        let due = app::host::clock::next_deadline(&b.state).expect("the timer stays armed through the intro");
        let early = fire(&mut b, due - Duration::from_millis(1));
        assert!(!early.redraw, "nothing fires before its deadline");
        let fired = fire(&mut b, due);
        assert!(fired.redraw && !fired.quit, "tick {ticks}");
        assert!(b.state.glulx_timer_next_fire.is_none(), "a fired timer disarms until the host refreshes");
        let _ = app::host::clock::refresh_input(&mut b.state, &mut *b.session);
        ticks += 1;
        if frame(&b) != before {
            break;
        }
    }
    assert_ne!(frame(&b), before, "timer events alone moved the game's screen on ({ticks} ticks)");
    let _ = std::fs::remove_dir_all(&home);
}

// ── The pane size ─────────────────────────────────────────────────────────────

/// The host names the pane in cells and a v5 story is told it in its header —
/// no pane rectangles, no terminal.
#[test]
fn the_host_sets_a_zmachine_storys_screen_size() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-screen");
    let mut b = boot(story, &home);
    let header = |b: &BootedStory| {
        let z = app::engine_helpers::zvm_session_opt(&*b.session).unwrap();
        (z.machine.mem.read_byte(0x20), z.machine.mem.read_byte(0x21))
    };
    assert!(app::host::screen::set_story_pane(&mut *b.session, &b.state, (120, 40)));
    let (rows, cols) = header(&b);
    assert!(cols >= 110 && rows >= 30, "the story is told a pane near 120x40: {cols}x{rows}");
    assert!(
        !app::host::screen::set_story_pane(&mut *b.session, &b.state, (120, 40)),
        "the same size again tells the story nothing new"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// SQ-1750: the pane's last column goes to the TUI's scrollbar by default, so a
/// v4+ story is told one column less than the pane; a host with no scrollbar
/// column turns that off and the story gets the whole pane, as Glulx does.
#[test]
fn a_host_without_a_scrollbar_column_gives_the_story_the_whole_pane() {
    let story = fixture_path("LostPig.z8");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let width_told = |gutter: Option<bool>| {
        let home = app::scratch_dir("host-gutter");
        let mut b = boot(story.clone(), &home);
        if let Some(g) = gutter {
            b.state.config.story_scrollbar_gutter = g;
        }
        assert!(app::host::screen::set_story_pane(&mut *b.session, &b.state, (90, 28)));
        let z = app::engine_helpers::zvm_session_opt(&*b.session).unwrap();
        let w = z.machine.mem.read_byte(0x21);
        let _ = std::fs::remove_dir_all(&home);
        w
    };
    assert_eq!(width_told(None), 89, "default: the scrollbar column is reserved");
    assert_eq!(width_told(Some(true)), 89);
    assert_eq!(width_told(Some(false)), 90, "no gutter: the story gets the whole pane");
}

// ── Exit save, reset, and the game's own SAVE/RESTORE ─────────────────────────

/// The exit save leaves a resume point the next boot picks up, and the game
/// plays on from it exactly as the original would have.
#[test]
fn an_exit_save_is_what_the_next_boot_resumes() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-exit-save");
    let mut first = boot(story.clone(), &home);
    assert!(!command(&mut first, "south").quit);
    assert_eq!(here(&first).as_deref(), Some("Mouth of Alley"), "premise: the move went somewhere");
    let saved = app::host::persist::exit_save(
        &mut *first.session, &first.mapper, &first.state, &first.ifid, &first.arc_file,
    );
    assert_eq!(saved, app::host::persist::ExitSave::Saved);

    let mut second = boot(story, &home);
    assert_eq!(here(&second), here(&first), "the next boot resumes where the exit save left off");
    assert_eq!(second.state.turns, first.state.turns);
    // Perturb before trusting it.
    let _ = command(&mut first, "north");
    let _ = command(&mut second, "north");
    assert_eq!(here(&second), here(&first));
    assert_eq!(second.state.transcript.last(), first.state.transcript.last());
    let _ = std::fs::remove_dir_all(&home);
}

/// **SQ-1549**: `app::host::set_guidance` persists exactly the way
/// `/set-guidance` does — a fresh boot of the same story, from the same
/// per-game save directory, comes up with the override already in force.
#[test]
fn set_guidance_persists_across_a_reboot() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-guidance");
    let mut first = boot(story.clone(), &home);
    assert!(first.state.config.guidance, "premise: guidance ships on");

    let effective =
        app::host::set_guidance(&mut first.state, &first.game_dir, app::slash::GuidanceArg::Off)
            .expect("the sidecar writes");
    assert!(!effective, "off is off");
    assert!(!first.state.config.guidance, "and takes effect immediately, live");

    let second = boot(story, &home);
    assert!(!second.state.config.guidance, "the reboot reads the same per-game override");
    let _ = std::fs::remove_dir_all(&home);
}

/// A reset with `clear_map` puts the story back at its start with a fresh
/// transcript, turn count and map, exactly as the TUI's `/reset-game map`.
#[test]
fn a_reset_clears_the_game_the_map_and_the_counters() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-reset");
    let mut b = boot(story.clone(), &home);
    let start = here(&b);
    let _ = command(&mut b, "south");
    b.state.turns = 1;
    assert!(b.mapper.graph.rooms().count() >= 2, "premise: the walk mapped two rooms");

    app::host::reset::reset_game(
        &mut *b.session,
        &mut b.mapper,
        &mut b.state,
        &b.story_bytes,
        &b.story_path,
        &b.game_dir,
        None,
        app::host::reset::ResetOptions { clear_map: true, delete_data: false },
    );
    assert_eq!(here(&b), start, "back at the start");
    assert_eq!(b.state.turns, 0);
    assert_eq!(b.mapper.graph.rooms().count(), 1, "the map holds only the start room");
    let text = b.state.transcript.join("\n");
    assert!(text.contains("Spider And Web"), "the transcript is the fresh banner: {text}");
    assert!(!text.contains("Mouth of Alley"), "and nothing from before the reset");
    let _ = command(&mut b, "south");
    assert_eq!(here(&b).as_deref(), Some("Mouth of Alley"), "and the game plays on from there");
    let _ = std::fs::remove_dir_all(&home);
}

/// The game's own SAVE, answered with a name, writes the save file; the game's
/// own RESTORE, answered with the same name, reads it back.
#[test]
fn a_game_save_answered_by_name_is_what_a_later_restore_reads() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-file-prompt");
    let mut b = boot(story, &home);
    let _ = command(&mut b, "south");
    assert_eq!(here(&b).as_deref(), Some("Mouth of Alley"));

    let _ = command(&mut b, "save");
    assert_eq!(pending_file_prompt(&b.state), Some(FilePrompt::Save), "the game asks where to save");
    let out = answer_file_prompt(
        &mut *b.session, &mut b.mapper, &mut b.state, &b.game_dir, &b.ifid, Some("in the alley"), None,
    );
    assert!(!out.quit);
    assert_eq!(pending_file_prompt(&b.state), None, "the save was answered");
    assert!(b.game_dir.join("in-the-alley.lanthorn").is_file(), "and written");
    assert_eq!(b.session.pending_input(), InputKind::Line, "the game took its turn back");

    let _ = command(&mut b, "north");
    assert_eq!(here(&b).as_deref(), Some("End of Alley"), "premise: we moved on after saving");

    let _ = command(&mut b, "restore");
    assert_eq!(pending_file_prompt(&b.state), Some(FilePrompt::Restore), "the game asks what to restore");
    let out = answer_file_prompt(
        &mut *b.session, &mut b.mapper, &mut b.state, &b.game_dir, &b.ifid, Some("In The Alley"), None,
    );
    assert!(!out.quit);
    assert_eq!(pending_file_prompt(&b.state), None);
    assert_eq!(here(&b).as_deref(), Some("Mouth of Alley"), "the restore read back the saved game");
    // Perturb: the restored game plays on from the saved room.
    let _ = command(&mut b, "north");
    assert_eq!(here(&b).as_deref(), Some("End of Alley"));
    let _ = std::fs::remove_dir_all(&home);
}

/// Cancelling the game's prompt tells the game it failed, and the game goes on.
#[test]
fn a_cancelled_game_save_resumes_the_game() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-file-cancel");
    let mut b = boot(story, &home);
    let _ = command(&mut b, "save");
    assert_eq!(pending_file_prompt(&b.state), Some(FilePrompt::Save));
    let _ = answer_file_prompt(&mut *b.session, &mut b.mapper, &mut b.state, &b.game_dir, &b.ifid, None, None);
    assert_eq!(pending_file_prompt(&b.state), None);
    assert_eq!(b.session.pending_input(), InputKind::Line, "the game is back at its prompt");
    let _ = std::fs::remove_dir_all(&home);
}

// ── `handle_save_as`'s outcome (SQ-1545) ────────────────────────────────────

/// `handle_save_as` answers a submitted save name directly — the same call the
/// TUI's save-name dialog submit makes — with no dialog of its own to read the
/// result off. A fresh name writes and reports `Saved`; the same name again
/// with `force: false` reports `Exists` (what the TUI turns into its
/// overwrite-confirm dialog) instead of a host having to notice the dialog
/// state changed; and a `dir` that cannot be written to reports `Failed` with
/// the reason, instead of a host scraping `[Save failed: …]` out of a notice.
#[test]
fn handle_save_as_reports_saved_exists_and_failed_with_no_dialog_to_read() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-save-as-outcome");
    let mut b = boot(story, &home);

    let out = app::host::ingame_io::handle_save_as(
        "dup".into(), &b.game_dir, &b.ifid, &mut b.mapper, &mut *b.session, &mut b.state, false,
    );
    assert_eq!(out, app::host::ingame_io::SaveAsOutcome::Saved, "a fresh name writes");
    assert!(b.game_dir.join("dup.lanthorn").is_file(), "and the file is really there");

    let out = app::host::ingame_io::handle_save_as(
        "dup".into(), &b.game_dir, &b.ifid, &mut b.mapper, &mut *b.session, &mut b.state, false,
    );
    match out {
        app::host::ingame_io::SaveAsOutcome::Exists { path, .. } => {
            assert_eq!(path, b.game_dir.join("dup.lanthorn"), "names the colliding file");
        }
        other => panic!("expected Exists for a name already on disk, got {other:?}"),
    }

    // A `dir` this process cannot write into: the write itself fails. Unix
    // only (mirrors `storage::deny_new_files_in`'s own reasoning) — `dir`
    // creation on-write (`storage::atomic_write_with`) means a merely-absent
    // directory is not enough to provoke a failure, it gets created.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked_dir = home.join("locked-game-dir");
        std::fs::create_dir_all(&locked_dir).unwrap();
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        // root (common in CI containers) ignores the mode bits — probe first
        // and skip rather than assert on an unenforceable permission.
        if std::fs::File::create(locked_dir.join(".probe")).is_ok() {
            eprintln!("SKIP: cannot enforce a read-only directory in this environment (root?)");
        } else {
            let out = app::host::ingame_io::handle_save_as(
                "elsewhere".into(), &locked_dir, &b.ifid, &mut b.mapper, &mut *b.session, &mut b.state, false,
            );
            match out {
                app::host::ingame_io::SaveAsOutcome::Failed(reason) => {
                    assert!(!reason.is_empty(), "carries a reason a host can show");
                }
                other => panic!("expected Failed for an unwritable dir, got {other:?}"),
            }
        }
        let _ = std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o700));
    }
    let _ = std::fs::remove_dir_all(&home);
}

// ── `resolve_zcolour` (SQ-1545) ─────────────────────────────────────────────

/// `render::resolve_zcolour` used to be `pub(crate)`, unreachable from a host
/// that draws its own transcript `StyleRun`s rather than letting the TUI's
/// renderer draw them. It is `pub` now — callable straight from here, exactly
/// as a host would, with no need to restate the packed-zcolour-to-`Color`
/// mapping.
#[test]
fn resolve_zcolour_is_callable_from_outside_render() {
    use app::colors::ColorScheme;
    use ratatui::style::Color;
    use zvm::screen::ZColour;

    let scheme = ColorScheme::terminal_default();
    assert_eq!(app::render::resolve_zcolour(ZColour::Default, &scheme), Color::Reset);
    assert_eq!(
        app::render::resolve_zcolour(ZColour::Standard(4), &scheme),
        scheme.palette[(4 - 2) as usize],
        "a Standard colour routes through the theme palette"
    );
    assert_eq!(
        app::render::resolve_zcolour(ZColour::True24(0x102030), &scheme),
        Color::Rgb(0x10, 0x20, 0x30),
        "a 24-bit true colour is exact"
    );
}

// ── `packed_to_rgba` (SQ-1609) ──────────────────────────────────────────────

/// `render::v6_layout::packed_to_rgba` used to be `pub(crate)`, unreachable from
/// a host resolving a packed z-colour off [`app::render::screen::V6FrameInputs::host_pair`]
/// (or any other packed value it holds) to a concrete pixel colour — it had to
/// reimplement this function's own body from the public primitives
/// (`app::colors::standard_colour_rgb`, `app::render::resolve_zcolour`). It is
/// `pub` now — callable straight from here, exactly as a host would, with no
/// need to restate the packed-colour-to-RGBA mapping.
#[test]
fn packed_to_rgba_is_callable_from_outside_render() {
    use app::colors::ColorScheme;
    use app::render::v6_layout::packed_to_rgba;
    use app::state::pack_zcolour;
    use image::Rgba;
    use zvm::screen::ZColour;

    let scheme = ColorScheme::terminal_default();
    let fallback = Rgba([1, 2, 3, 255]);

    // True24: direct RGB, no theme involved.
    let true24_packed = pack_zcolour(ZColour::True24(0x102030));
    assert_eq!(
        packed_to_rgba(true24_packed, fallback, &scheme),
        Rgba([0x10, 0x20, 0x30, 255]),
        "a 24-bit true colour resolves to its exact RGB"
    );

    // Standard(4): resolves through the ZMSD §8.3.1 fixed pixel RGB, the same
    // table the internal pixel path (`standard_pixel_rgb`) reads.
    let standard_packed = pack_zcolour(ZColour::Standard(4));
    let (r, g, b) =
        app::colors::standard_colour_rgb(scheme.machine_palette, 4).expect("Standard(4) has a §8.3.1 RGB");
    assert_eq!(
        packed_to_rgba(standard_packed, fallback, &scheme),
        Rgba([r, g, b, 255]),
        "a Standard colour resolves through the fixed §8.3.1 pixel RGB"
    );

    // Default (0) → fallback, unconditionally.
    assert_eq!(
        packed_to_rgba(0, fallback, &scheme),
        fallback,
        "packed 0 (Default) always answers the caller's fallback"
    );
}

// ── Opening-banner pager after reset (SQ-1575) ─────────────────────────────

/// Render one frame of the story pane and resolve any pending pager arm against
/// it — the same measure-then-`apply_frame` step the TUI's render loop performs
/// after every frame, driven headlessly.
fn settle_pager(b: &mut BootedStory, w: u16, h: u16) {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let char_mode = b.session.pending_input() == InputKind::Char;
    let m = app::render::screen::render_story_pane(&b.session.screen(), char_mode, None, &b.state, area, &mut buf);
    app::pager::apply_frame(&mut b.state, m.max_scroll, m.viewport_rows, m.prompt_rows, m.total_rows, m.transcript_surface, m.top_anchored_fits);
}

/// A restarted game's banner must pause at `[more]` exactly where a fresh boot
/// of the same story would — SQ-1575: `reset_game` used to arm nothing at all,
/// so the restarted view sat at the bottom of the new banner while a fresh
/// launch of the identical story paused partway through it.
///
/// Compared against a genuinely FRESH boot's own armed state rather than a
/// hardcoded row, per the acceptance criteria: both boots use the same story
/// fixture at the same pane, in separate scratch homes so neither auto-resumes
/// a save the other left behind. Measured (`zzz_probe_zork0_boot_state`, since
/// removed): Zork Zero r393's 13-line banner wraps to 20 rows; at 80x20 the
/// viewport is 18 rows (1 for the `[more]` bar), so it overflows by 2 — a fresh
/// boot's own opening arm already engages here, which is this case's premise.
#[test]
fn reset_game_arms_the_opening_pager_like_a_fresh_boot() {
    let story = fixture_path("zork0-r393-s890714.z6");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let pane = (80u16, 20u16);

    // The reference: a genuinely fresh boot, never played, never reset.
    let fresh_home = app::scratch_dir("host-reset-pager-fresh");
    let mut fresh = boot(story.clone(), &fresh_home);
    settle_pager(&mut fresh, pane.0, pane.1);
    assert!(
        fresh.state.pager.active,
        "premise: Zork Zero's prologue overflows an {}x{} pane on a fresh boot — this case is only \
         about SQ-1575 while that premise holds",
        pane.0, pane.1
    );

    // The subject: boot the same story in its own home, settle its own opening
    // frame, play a move, then reset it — the exact action `/reset-game` and
    // "Play again" perform.
    let subject_home = app::scratch_dir("host-reset-pager-subject");
    let mut subject = boot(story.clone(), &subject_home);
    settle_pager(&mut subject, pane.0, pane.1);
    let _ = command(&mut subject, "look");
    app::host::reset::reset_game(
        &mut *subject.session,
        &mut subject.mapper,
        &mut subject.state,
        &subject.story_bytes,
        &subject.story_path,
        &subject.game_dir,
        None,
        app::host::reset::ResetOptions { clear_map: false, delete_data: false },
    );
    settle_pager(&mut subject, pane.0, pane.1);

    assert!(subject.state.pager.active, "a restarted game's banner must pause too, same as a fresh boot");
    assert_eq!(
        subject.state.transcript_scroll, fresh.state.transcript_scroll,
        "and park at the SAME row a fresh boot of the identical story parks at"
    );

    let _ = std::fs::remove_dir_all(&fresh_home);
    let _ = std::fs::remove_dir_all(&subject_home);
}

/// The other half of the ruleset: a story whose banner fits the pane does not
/// pause after reset, same as it doesn't after a fresh boot — a generously
/// large pane so Tangle's banner (a handful of lines) fits outright.
#[test]
fn reset_game_does_not_pause_when_the_new_banner_fits() {
    let story = fixture_path("Tangle.z5");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let pane = (80u16, 60u16);
    let home = app::scratch_dir("host-reset-pager-fits");
    let mut b = boot(story, &home);
    settle_pager(&mut b, pane.0, pane.1);
    assert!(!b.state.pager.active, "premise: Tangle's banner fits an 80x60 pane on a fresh boot");

    let _ = command(&mut b, "south");
    app::host::reset::reset_game(
        &mut *b.session,
        &mut b.mapper,
        &mut b.state,
        &b.story_bytes,
        &b.story_path,
        &b.game_dir,
        None,
        app::host::reset::ResetOptions { clear_map: false, delete_data: false },
    );
    settle_pager(&mut b, pane.0, pane.1);

    assert!(!b.state.pager.active, "the fresh banner fits the pane — no pager after reset, same as a fresh boot");
    assert_eq!(b.state.transcript_scroll, 0, "and the view stays at the bottom, on the prompt");

    let _ = std::fs::remove_dir_all(&home);
}

// ── `resume_from_turn` (SQ-1619) ────────────────────────────────────────────

/// `host::persist::resume_from_turn` is the library core `Action::ReplayResume`
/// now calls: restore the engine to a recorded turn's snapshot, rewind the
/// transcript/map/history to match it, and re-observe (and recenter on) the
/// resumed room. Exercised directly here, the way a non-terminal host would —
/// not only through the TUI's key handler.
///
/// Driven against minizork, a v3 story: its location is Global Variable 0, read
/// straight out of restored dynamic memory, so it resolves immediately after a
/// rewind's bare Quetzal snapshot with no screen carried alongside it (unlike a
/// v4+ status-line story, whose location needs the upper window `restore_state`
/// deliberately blanks — see its own doc comment — so it stays undetected until
/// the game's next repaint; that is a pre-existing, separate limitation of a
/// snapshot with no screen, not something this test is about).
#[test]
fn resume_from_turn_rewinds_engine_transcript_map_and_recenters() {
    let story = fixture_path("minizork-r34-s871124.z3");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-resume-from-turn");
    let mut b = boot(story, &home);
    b.state.config.record_turn_history = true;

    let start_room = here(&b);
    assert_eq!(start_room.as_deref(), Some("West of House"));
    let _ = command(&mut b, "south"); // turn 1: a new room -> map snapshot recorded
    assert_eq!(here(&b).as_deref(), Some("South of House"));
    let _ = command(&mut b, "west"); // turn 2: back to the start
    assert_eq!(here(&b), start_room);
    let _ = command(&mut b, "south"); // turn 3: forward again
    assert_eq!(here(&b).as_deref(), Some("South of House"));

    assert_eq!(b.state.history.len(), 3, "premise: three recorded turns");
    assert!(b.state.history[0].map_snapshot.is_some(), "premise: turn 1 added a room");
    assert_eq!(b.state.turns, 3);

    // Rewind to turn 1 (history index 0): back in South of House, one turn in.
    b.state.scroll = (999, 999); // sentinel: proves the recenter below actually moved it
    let result = app::host::persist::resume_from_turn(
        &mut *b.session, &mut b.mapper, &mut b.state, 0, Some((80, 24)),
    );
    assert!(result.ok && result.redraw, "a resume within range succeeds: {result:?}");
    assert_eq!(here(&b).as_deref(), Some("South of House"), "the engine restored to turn 1's snapshot");
    assert_eq!(b.state.turns, 1, "the turn counter rewound with it");
    assert_eq!(b.state.history.len(), 1, "later turns are discarded (linear rewind)");
    assert_eq!(
        b.state.transcript_kinds.len(), b.state.transcript.len(),
        "transcript and its kinds stay length-matched"
    );
    assert_eq!(
        b.state.transcript_runs.len(), b.state.transcript.len(),
        "and its style-run/paragraph sidecars"
    );
    assert_eq!(b.state.transcript_para.len(), b.state.transcript.len());
    let transcript = b.state.transcript.join("\n");
    // SQ-1715: the live transcript is cut back, so it is the screen the player saw
    // -- the game's own `>south` echo and the opening banner included -- not a
    // rebuild's `> south`.
    assert!(transcript.contains(">south"), "the echoed command survives the cut-back: {transcript:?}");
    assert!(
        transcript.contains("facing the south side"),
        "and turn 1's own output: {transcript:?}"
    );
    assert!(
        !transcript.contains(">west") && !transcript.contains("> west"),
        "only turn 1 is kept, nothing from the discarded turns 2-3: {transcript:?}"
    );
    assert_ne!(b.state.scroll, (999, 999), "the map pane recenters on the resumed room");
    assert_eq!(
        b.state.notifications.latest_text(), Some("Resumed from turn 1"),
        "a notice names the turn resumed to"
    );
    assert!(!b.state.unsaved_progress, "resuming a recorded (already-saved) turn leaves nothing unsaved");

    // Perturb, then assert (restore tests must move past the resume point before
    // trusting it): the resumed session plays on from turn 1, not from where it
    // was before the rewind.
    let _ = command(&mut b, "west");
    assert_eq!(here(&b), start_room, "play continues from the resumed turn");
    assert_eq!(b.state.turns, 2);

    let _ = std::fs::remove_dir_all(&home);
}

/// `idx` past the end of `state.history` is a no-op: nothing about the engine,
/// transcript, map or history is touched, and no notice is pushed.
#[test]
fn resume_from_turn_out_of_range_index_is_a_no_op() {
    let story = fixture_path("minizork-r34-s871124.z3");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-resume-from-turn-oob");
    let mut b = boot(story, &home);
    b.state.config.record_turn_history = true;
    let _ = command(&mut b, "south");
    assert_eq!(b.state.history.len(), 1, "premise: one recorded turn");

    let before_loc = here(&b);
    let before_turns = b.state.turns;
    let before_transcript = b.state.transcript.clone();

    let result = app::host::persist::resume_from_turn(
        &mut *b.session, &mut b.mapper, &mut b.state, 1, Some((80, 24)),
    );
    assert_eq!(result, app::host::persist::Rewound::default(), "out of range: nothing happened");
    assert_eq!(here(&b), before_loc);
    assert_eq!(b.state.turns, before_turns);
    assert_eq!(b.state.transcript, before_transcript);
    assert_eq!(b.state.history.len(), 1, "history untouched");
    assert_eq!(b.state.notifications.latest_text(), None, "no notice for a no-op");

    let _ = std::fs::remove_dir_all(&home);
}

// ── SQ-1621 / SQ-1622 ────────────────────────────────────────────────────────

/// `TurnRecord::location_name` (SQ-1621) reflects the room the player actually
/// stood in on THAT turn, even when the turn made no structural change and so
/// has no `map_snapshot` of its own. And the replay overlay's map reconstruction
/// — `from_json` off the latest at-or-before snapshot, then `MapGraph::set_current`
/// with the SELECTED turn's own `location` (the fix landed in `main.rs`'s
/// `replay_graph`) — flags the correct room current, not whichever room was
/// current when that older snapshot was serialized.
///
/// Minizork's own geography: `n` → North of House (new room), `e` → Behind
/// House (new room), `open window` (no move), `west` → Kitchen (new room,
/// through the window), `west` → Living Room (new room), `east` → back to
/// Kitchen — an ALREADY-KNOWN room, so this last turn makes no structural
/// change and gets no `map_snapshot` of its own; the latest snapshot
/// at-or-before it is Living Room's.
#[test]
fn replay_location_names_the_turns_own_room_not_an_older_snapshots() {
    let story = fixture_path("minizork-r34-s871124.z3");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-replay-location");
    let mut b = boot(story, &home);
    b.state.config.record_turn_history = true;

    let _ = command(&mut b, "n"); // turn 1: North of House (new room, new conn)
    let _ = command(&mut b, "e"); // turn 2: Behind House (new room, new conn)
    let _ = command(&mut b, "open window"); // turn 3: no move
    let _ = command(&mut b, "west"); // turn 4: Kitchen (new room, new conn)
    assert_eq!(here(&b).as_deref(), Some("Kitchen"));
    let _ = command(&mut b, "west"); // turn 5: Living Room (new room, new conn Kitchen--west-->LivingRoom)
    assert_eq!(here(&b).as_deref(), Some("Living Room"));
    let _ = command(&mut b, "east"); // turn 6: Kitchen again — already-known room, but the
    // REVERSE connection (LivingRoom--east-->Kitchen) hasn't been walked before, so this still
    // counts as a structural change and gets its OWN snapshot (current = Kitchen at turn 6).
    assert_eq!(here(&b).as_deref(), Some("Kitchen"));
    let _ = command(&mut b, "west"); // turn 7: Living Room — the SAME connection as turn 5's
    // (Kitchen--west-->LivingRoom), already known, so this turn is truly non-structural: no new
    // room, no new connection, no map_snapshot of its own.
    assert_eq!(here(&b).as_deref(), Some("Living Room"));

    assert_eq!(b.state.history.len(), 7, "premise: seven recorded turns");
    let last = &b.state.history[6];
    assert_eq!(last.turn, 7);
    assert!(last.map_snapshot.is_none(), "premise: turn 7 made no structural change");
    assert_eq!(
        last.location_name.as_deref(), Some("Living Room"),
        "SQ-1621: this turn's own room, not Kitchen's (turn 6's frozen snapshot)"
    );

    // Reconstruct the map exactly the way `main.rs`'s fixed `replay_graph` does:
    // the latest at-or-before snapshot, then override `current` with THIS
    // turn's own recorded location.
    let snap = app::history::map_at_turn(&b.state.history, last.turn)
        .and_then(|json| mapper::persist::from_json(json).ok())
        .expect("a baseline snapshot exists at-or-before turn 7 (turn 6's)");
    let mut graph = snap.graph;
    let living_room_id = last.location.expect("turn 7 recorded a location");
    let kitchen_id = graph.current().expect("turn 6's snapshot has a current room");
    assert_ne!(
        living_room_id, kitchen_id,
        "premise: turn 6's frozen snapshot's own baked `current` (Kitchen) differs from turn 7's \
         own room (Living Room) — the bug this fix closes"
    );
    assert_eq!(graph.room(kitchen_id).map(|r| r.name.as_str()), Some("Kitchen"));
    graph.set_current(living_room_id);

    let rm = mapper::render::render(&graph);
    let living_room_row = rm.rooms.iter().find(|r| r.id == living_room_id)
        .expect("Living Room is in the reconstructed graph");
    assert!(living_room_row.is_current, "the reconstructed graph flags Living Room — this turn's room — current");
    for r in &rm.rooms {
        if r.id != living_room_id {
            assert!(!r.is_current, "only Living Room is current, not room {} ({})", r.id, r.label);
        }
    }

    let _ = std::fs::remove_dir_all(&home);
}

/// SQ-1622: recording history switched on mid-game — after the mapper has
/// already explored rooms — must not leave the retained history with zero
/// `map_snapshot`s anywhere until the map next structurally changes. The
/// quest's own named repro: Lost Pig, explore first with recording off, turn
/// recording on, then make one move that finds no new room.
///
/// Lost Pig opens in pitch dark: Grunk must `smell` (repeatedly — the direction
/// is randomized per attempt, but `random_seed: Some(1)` in `headless_config`
/// makes the sequence deterministic run to run) until it reveals which way the
/// pig went, then walk that way to fall into the Hole — the mapper's second
/// room. Recording then switches on, and `look` (no move) is the first
/// recorded turn: no structural change of its own.
#[test]
fn recording_switched_on_mid_game_still_has_a_baseline_snapshot() {
    let story = fixture_path("LostPig.z8");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-recording-mid-game");
    let mut b = boot(story, &home);
    assert!(!b.state.config.record_turn_history, "premise: recording starts off");

    // Explore a little with recording off — the mapper accumulates rooms, but
    // nothing lands in `state.history`.
    let mut dir = None;
    for _ in 0..10 {
        let before = b.state.transcript.len();
        let _ = command(&mut b, "smell");
        let new_lines = &b.state.transcript[before..];
        dir = ["northeast", "northwest", "southeast", "southwest", "north", "south", "east", "west"]
            .into_iter()
            .find(|d| new_lines.iter().any(|l| l.contains(d)));
        if dir.is_some() {
            break;
        }
    }
    let _ = command(&mut b, dir.expect("smell reveals a direction within 10 tries (deterministic seed)"));
    assert!(b.state.history.is_empty(), "premise: nothing recorded yet");
    assert!(b.mapper.graph.rooms().count() >= 2, "premise: the mapper already has rooms (fell into the Hole)");

    // Switch recording on, then make a move that finds no new room — `look`
    // never moves — no structural change on this first recorded turn.
    b.state.config.record_turn_history = true;
    let rooms_before = b.mapper.graph.rooms().count();
    let _ = command(&mut b, "look");
    assert_eq!(b.mapper.graph.rooms().count(), rooms_before, "premise: no new room this turn");

    assert_eq!(b.state.history.len(), 1, "premise: exactly one recorded turn so far");
    assert!(
        b.state.history[0].map_snapshot.is_some(),
        "SQ-1622: the very first recorded turn always carries a baseline snapshot"
    );
    assert!(
        app::history::map_at_turn(&b.state.history, b.state.turns).is_some(),
        "SQ-1622: so a viewer of the current turn has a map to show, not an empty one"
    );

    let _ = std::fs::remove_dir_all(&home);
}

// ── SQ-1629 ──────────────────────────────────────────────────────────────────

/// SQ-1629 Fix 2: the room a story STARTS in never got a `description` at all — no command
/// triggers `finish_command_turn`/`finish_resumed_turn`'s `apply_room_description` for it, and
/// the player has not typed `look` yet either. `Engine::seed_turn`'s boot drain (Z-machine
/// override in `session.rs`) now captures it the same way an ordinary arrival does, wired into
/// `host::boot::boot_story` right beside the pre-existing `apply_turn` seed call.
#[test]
fn boot_captures_the_starting_rooms_own_description() {
    let story = fixture_path("minizork-r34-s871124.z3");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-boot-starting-room-description");
    let b = boot(story, &home);
    assert_eq!(here(&b).as_deref(), Some("West of House"), "premise: minizork starts here");
    let start = b.mapper.graph.current().expect("the starting room is observed at boot");
    let desc = b.mapper.graph.room(start).and_then(|r| r.description.clone());
    assert!(
        desc.as_deref().is_some_and(|d| !d.trim().is_empty()),
        "West of House must carry a description straight off boot, before any command: {desc:?}"
    );
}

/// SQ-1629 Fix 1: minizork (a v1-3 game) prints its own inline `>` read prompt as part of the
/// SAME turn's transcript as its room description — the mapper's captured description must stop
/// before it (reusing [`strip_read_prompt`], the same prompt-recognition the Glk tier's own
/// `take_room_description` already runs), not carry a trailing "\n\n>". Covers both the boot
/// capture (Fix 2, West of House) and an ordinary arrival (Forest Path, reached via `n`, `n`
/// per the quest's own repro).
#[test]
fn a_room_description_never_carries_the_games_own_trailing_prompt() {
    let story = fixture_path("minizork-r34-s871124.z3");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-room-description-prompt-strip");
    let mut b = boot(story, &home);

    let start = b.mapper.graph.current().expect("the starting room is observed at boot");
    let start_desc = b.mapper.graph.room(start).and_then(|r| r.description.clone());
    assert!(start_desc.is_some(), "premise: West of House has a captured description (Fix 2)");
    let start_desc = start_desc.unwrap();
    assert!(!start_desc.contains('>'), "the boot's own inline read prompt must not leak in: {start_desc:?}");

    let _ = command(&mut b, "n");
    let _ = command(&mut b, "n");
    assert_eq!(here(&b).as_deref(), Some("Forest Path"), "premise: n, n reaches Forest Path");
    let id = b.mapper.graph.current().expect("Forest Path is on the map");
    let desc = b.mapper.graph.room(id).and_then(|r| r.description.clone());
    assert!(desc.is_some(), "Forest Path must have a captured description");
    let desc = desc.unwrap();
    assert!(!desc.contains('>'), "the trailing inline read prompt must not leak into the description: {desc:?}");
    assert!(!desc.trim_end().ends_with('>'), "and no residual blank-line-then-prompt tail: {desc:?}");

    let _ = std::fs::remove_dir_all(&home);
}

/// SQ-1629: the Glk tier already strips its own trailing prompt (`AppGlk::take_room_description`
/// via `strip_read_prompt`, unchanged by Fix 1) — this confirms an ordinary Glulx boot-and-look
/// sequence is unaffected by Fix 1's Z-machine-side change, and that Fix 2's boot-time wiring
/// (`apply_room_description`/`apply_item_observations` in `host::boot::boot_story`, engine-
/// neutral) DOES reach Glulx too: `GlulxSession::new_with_store` already resolves the opening
/// room's HEADING at construction (to seed `last_room`/the room-lock), and a small companion
/// change stashes the paired DESCRIPTION `take_room_heading` leaves ready right beside it
/// (`GlulxSession::boot_description`), which `GlulxSession`'s own `seed_turn` override now hands
/// to the mapper — the same shape as the Z-machine fix, just reading a different capture.
#[test]
fn glulx_boot_captures_the_starting_rooms_own_description_too() {
    let story = fixture_path("chlorophyll.gblorb");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-glulx-boot-and-look");
    let b = boot(story, &home);
    let start = b.mapper.graph.current().expect("Chlorophyll's opening room is seeded at boot");
    let desc = b.mapper.graph.room(start).and_then(|r| r.description.clone());
    assert!(
        desc.as_deref().is_some_and(|d| !d.trim().is_empty()),
        "the starting room must carry a description straight off boot: {desc:?}"
    );
    assert!(!desc.unwrap().contains('>'), "Glk's own prompt-stripping still holds");

    let _ = std::fs::remove_dir_all(&home);
}
