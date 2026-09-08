//! SQ-1411: a Save State taken on a v6 full-screen picture takeover resumes
//! with lanthorn's own `[more]` pager parked at the TOP of the restored
//! scrollback, so the first keypress after the resume pages down through the
//! entire transcript the player already read — while a Save State at a `>`
//! prompt (the very same session, one turn earlier) and an in-game `@restore`
//! do not.
//!
//! Root cause (see the quest note): `AppState::last_transcript_total_rows`,
//! the pager's baseline, is never re-seeded when `transcript` is replaced
//! wholesale by a resume. That is invisible whenever the resumed frame HAS a
//! transcript surface — `pager::apply_frame` calibrates the baseline before
//! the player can act — but a v6 full-screen picture takeover (Zork Zero's
//! "Q to resume story" splash, its map/rebus screens) reports NO transcript
//! surface at all, so the surfaceless-frame skip (SQ-0578, correct on its own
//! terms) leaves the stale baseline standing until the player's first
//! keypress arms the pager from it — 0, or whatever the pre-resume session
//! last cached. The next real frame then measures the WHOLE restored
//! backlog as "output since the last frame" and pages through it.
//!
//! The fix: `AppState::reset_transcript_sidecars` — already the one funnel
//! every wholesale transcript replacement goes through — marks
//! `Pager::baseline_stale`. `pager::apply_frame` treats the first frame that
//! actually has a transcript surface, while stale, as a CALIBRATION rather
//! than a measurement: drop any pending arm, land at the bottom, adopt the
//! frame's total as the new baseline.
//!
//! Case 1 below is synthetic and always runs (no story needed) — it pins the
//! module-level contract directly. Case 2 reproduces the reported symptom on
//! the real medium (gitignored, skips cleanly when absent).

use std::path::{Path, PathBuf};

use app::archive::{ArchiveContents, Meta, SaveTrigger, SessionRecord};
use app::engine::Engine;
use app::graphics::PictSource;
use app::hints::DiskImage;
use app::interpreter::InterpreterProfile;
use app::machine_boot::MachineBoot;
use app::pager::{self, Driver};
use app::session::{GameSession, InputKind, TurnResult};
use app::state::{apply_transcript_elems, AppState, TranscriptKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

// ── Case 1: synthetic, always runs ──────────────────────────────────────────

/// The module-level contract, driven directly through `pager::apply_frame`
/// exactly as `more_pager_arming.rs` and `more_pager_first_new_row.rs` do.
///
/// Falsify by reverting the `baseline_stale` branch in `pager::apply_frame`:
/// this case then fails with `pager.active == true` and `transcript_scroll`
/// parked near the TOP of the 400-row backlog instead of at the bottom.
#[test]
fn resumed_transcript_calibrates_to_the_bottom_instead_of_paging_from_the_top() {
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();

    // Install a long pre-existing transcript — hundreds of rows, several
    // [more] screenfuls at a normal terminal height.
    for i in 0..400 {
        state.push_transcript(&format!("resumed transcript line {i}"));
    }

    // The moment of resume: every restore site funnels wholesale transcript
    // replacement through this call (startup.rs, engine_helpers.rs, turn.rs,
    // main.rs's rewind/replay rebuild — see the quest note).
    state.reset_transcript_sidecars();
    assert!(state.pager.baseline_stale, "a wholesale transcript replacement must mark the baseline stale");

    // The frame the resume lands on: a v6 full-screen picture takeover (Zork
    // Zero's splash) — no transcript surface at all (SQ-0578).
    pager::apply_frame(&mut state, 0, 40, 0, 0, false);
    assert!(state.pager.baseline_stale, "a surfaceless frame cannot calibrate anything");
    assert_eq!(state.last_transcript_total_rows, 0, "still unseeded — no real frame has drawn yet");

    // The key that dismisses the splash finishes a turn; `turn.rs` arms the
    // pager from the (still-zero) cached baseline exactly as it does for any
    // other turn (`baseline_before(last_transcript_total_rows, continued_row)`).
    state.pager.arm(pager::baseline_before(state.last_transcript_total_rows, false));

    // The next frame lays the whole restored transcript out: 400 rows into a
    // 24-row viewport — an overflow that would normally engage the pager.
    pager::apply_frame(&mut state, 376, 24, 0, 400, true);
    assert!(!state.pager.active, "a resumed transcript must not page — the reader already read it");
    assert_eq!(state.transcript_scroll, 0, "the view calibrates to the bottom, not the top");
    assert_eq!(state.last_transcript_total_rows, 400, "the baseline is calibrated to the resumed total");
    assert!(!state.pager.baseline_stale, "calibration clears the flag");

    // Perturb: a further turn whose output fits within one screen must not
    // reopen the pager, and the view stays at the bottom with the baseline
    // tracking the new total.
    state.pager.arm_after_turn(state.last_transcript_total_rows, InputKind::Line, false, Driver::PlayerInput);
    pager::apply_frame(&mut state, 381, 24, 0, 405, true);
    assert!(!state.pager.active, "a small post-resume turn must not page");
    assert_eq!(state.transcript_scroll, 0, "still at the bottom");
    assert_eq!(state.last_transcript_total_rows, 405, "baseline tracks the new total");
}

// ── Case 2: the real medium (gitignored, skips cleanly) ─────────────────────

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn amiga_zork0_path() -> PathBuf {
    stories_dir().join("Zork Zero - The Revenge of Megaboz.adf")
}

/// Boot the Amiga floppy the way `startup.rs` does: the profile and picture
/// space come from the MEDIUM, and every per-machine fact travels through one
/// `MachineBoot::resolve` call (SQ-1021/SQ-1022) so this harness cannot omit a
/// fact the way `ring_scout` once did. This is release 366/serial 890323 — a
/// DIFFERENT build than the bare `zork0-r393-s890714.z6` (CLAUDE.md's "a disk
/// image is a different release" rule; pinned in `real_media_releases.rs`).
/// `None` when the gitignored medium is absent.
fn boot_amiga_zork0() -> Option<GameSession> {
    let path = amiga_zork0_path();
    let (loaded, mounted) = match app::hints::load_mounted_story(&path) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return None;
        }
    };
    assert_eq!(mounted, Some(DiskImage::Adf), "expected the Amiga ADF mount");
    let bytes = loaded.bytes().to_vec();
    assert_eq!(bytes[0], 6, "Zork Zero is a Version 6 story");
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]),
        366,
        "not the pinned r366 Amiga build (see real_media_releases.rs)"
    );
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let boot = MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    let mut s = GameSession::new_for_machine(bytes, true, false, false, picture_dims, None, None, &boot)
        .expect("Zork Zero boots off the Amiga floppy");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

fn render_state() -> AppState {
    let mut st = AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    st.config.v6_render = app::config::V6RenderMode::Hybrid;
    st
}

/// Push a turn's output into `state` through the same fork the real app's
/// turn handlers use: elems when the engine fills them (Glulx), plain runs
/// otherwise (the Z-machine path never fills `transcript_elems`).
fn push_turn(state: &mut AppState, r: &TurnResult) {
    if r.transcript_elems.is_empty() {
        state.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
    } else {
        apply_transcript_elems(state, &r.transcript_elems);
    }
}

/// Render the story pane through the REAL renderer and return its metrics —
/// the numbers the run loop feeds `pager::apply_frame` after every frame.
fn measure(session: &GameSession, state: &AppState, w: u16, h: u16) -> app::render::screen::StoryPaneMetrics {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let char_mode = session.pending_input() == InputKind::Char;
    app::render::screen::render_story_pane(&session.screen(), char_mode, None, state, area, &mut buf)
}

/// Save a host Save State + archive exactly as the app does: `Engine::save_state`
/// for the Quetzal memory/stack/pc, `screen.bin` for the v6 window table, the
/// graphics canvases as per-window PNGs, and `SessionRecord::of` so the whole
/// session (transcript, history, command history) rides along — the same
/// fields `startup.rs` reads back on resume.
fn save_state_archive(path: &Path, mapper: &mapper::mapper::Mapper, session: &GameSession, state: &AppState) {
    let es = Engine::save_state(session);
    let pics = session.pictures_png();
    app::archive::save_archive_meta_pics(
        path,
        mapper,
        &es,
        Some(&session.machine.screen),
        &session.machine.aux_data,
        Meta {
            format_version: app::archive::CURRENT_FORMAT_VERSION,
            ifid: None,
            name: None,
            turns: 0,
            saved_at: String::new(),
            location: None,
            score: None,
            trigger: SaveTrigger::HostState,
        },
        &SessionRecord::of(state),
        &pics,
        None,
        None,
    )
    .expect("save_archive_meta_pics");
}

/// Restore `ac` into `session`/`state` the way every host-mediated resume site
/// does (`startup.rs`, `engine_helpers::apply_archive_state`, `turn.rs`): the
/// VM state, the v6 screen + graphics canvases, then the WHOLE transcript
/// funneled through `reset_transcript_sidecars` — which is where SQ-1411's fix
/// lives.
fn restore_into(state: &mut AppState, session: &mut GameSession, ac: &ArchiveContents) {
    Engine::restore_state(session, &ac.engine_save()).expect("restore_state (Quetzal memory/stack/pc)");
    session.machine.screen = ac.screen.clone().expect("persisted v6 screen");
    session.load_pictures_png(&ac.pictures);
    state.transcript = ac.transcript.clone();
    state.clear_anchor = None;
    state.transcript_kinds = ac.transcript_kinds.clone();
    state.transcript_runs = ac.transcript_runs.clone();
    state.transcript_para = ac.transcript_para.clone();
    state.reset_transcript_sidecars();
    state.transcript_images = ac.transcript_images.clone();
    state.history = ac.history.clone();
    state.command_history = ac.command_history.clone();
}

/// SQ-1411: reproduced on the Amiga floppy. Eleven turns from boot reach the
/// rebus picture the Banquet Hall survivors are shown, which parks the VM on
/// a `read_char` waiting for the player to dismiss a full-screen v6 takeover:
///
/// `ne, s, w, e, w, "dive under table", wait, "stand up and get parchment",
/// u, s, "examine rebus"`
///
/// Two "[Hit any key to continue.]" reads land between those eleven lines
/// (drained with a space before the next command) — matching the
/// investigation's Gallery capture (`command_history` of eleven lines,
/// `pending=Char`).
///
/// A Save State taken one turn earlier — after the tenth command, at a normal
/// `>` prompt — is the control: same session, same medium, the only
/// difference is whether the resumed frame has a transcript surface.
///
/// Skip-if-missing (gitignored medium).
#[test]
fn zork_zero_amiga_splash_resume_does_not_replay_the_transcript() {
    let Some(mut session) = boot_amiga_zork0() else { return };
    let mut state = render_state();
    let _ = session.take_transcript(); // discard the boot banner

    let commands = [
        "ne",
        "s",
        "w",
        "e",
        "w",
        "dive under table",
        "wait",
        "stand up and get parchment",
        "u",
        "s",
        "examine rebus",
    ];

    // Drive the first ten commands — the `>`-prompt control's vantage point,
    // one turn before the rebus.
    for cmd in &commands[..10] {
        while session.pending_input() == InputKind::Char {
            let r = session.submit_char(b' ');
            push_turn(&mut state, &r);
        }
        let r = session.submit(cmd);
        push_turn(&mut state, &r);
    }
    assert_eq!(
        session.pending_input(),
        InputKind::Line,
        "premise: the control save sits at a normal '>' prompt"
    );
    assert!(
        state.transcript.len() > 20,
        "premise: ten turns of Zork Zero produce a real scrollback, got {} lines",
        state.transcript.len()
    );

    let scratch = app::scratch_dir("sq1411-zork0-amiga");
    let mapper = mapper::mapper::Mapper::default();

    // ── The `>`-prompt control ──────────────────────────────────────────
    let control_path = scratch.join("control.lanthorn");
    save_state_archive(&control_path, &mapper, &session, &state);

    let mut fresh_control = boot_amiga_zork0().expect("medium confirmed present above");
    let mut control_state = render_state();
    let ac = app::archive::load_archive(&control_path).expect("load_archive (control)");
    restore_into(&mut control_state, &mut fresh_control, &ac);
    assert!(control_state.pager.baseline_stale, "the restore marked the baseline stale");

    let m = measure(&fresh_control, &control_state, 80, 30);
    assert!(m.transcript_surface, "premise: a `>` prompt frame has a transcript surface");
    pager::apply_frame(&mut control_state, m.max_scroll, m.viewport_rows, m.prompt_rows, m.total_rows, m.transcript_surface);
    assert!(!control_state.pager.active, "control: the `>`-prompt resume must not page");
    assert_eq!(control_state.transcript_scroll, 0, "control: the view sits at the bottom");

    // ── Continue the ORIGINAL session one more turn: the rebus splash ──────
    while session.pending_input() == InputKind::Char {
        let r = session.submit_char(b' ');
        push_turn(&mut state, &r);
    }
    let r = session.submit(commands[10]);
    push_turn(&mut state, &r);
    assert_eq!(
        session.pending_input(),
        InputKind::Char,
        "premise: 'examine rebus' parks on a read_char (the splash)"
    );

    let splash_path = scratch.join("splash.lanthorn");
    save_state_archive(&splash_path, &mapper, &session, &state);

    let mut fresh_splash = boot_amiga_zork0().expect("medium confirmed present above");
    let mut splash_state = render_state();
    let ac = app::archive::load_archive(&splash_path).expect("load_archive (splash)");
    restore_into(&mut splash_state, &mut fresh_splash, &ac);
    assert!(splash_state.pager.baseline_stale, "the restore marked the baseline stale");

    // The frame the resume lands on: a full-screen v6 picture, no surface.
    let m = measure(&fresh_splash, &splash_state, 80, 30);
    assert!(
        !m.transcript_surface,
        "premise: the rebus splash is a full-screen picture takeover with no transcript surface"
    );
    pager::apply_frame(&mut splash_state, m.max_scroll, m.viewport_rows, m.prompt_rows, m.total_rows, m.transcript_surface);
    assert!(splash_state.pager.baseline_stale, "a surfaceless frame cannot calibrate anything");

    // Act once: the key that dismisses the splash, exactly as `turn.rs`'s
    // `apply_game_driven_result` arms the pager for any other game-driven turn.
    assert_eq!(fresh_splash.pending_input(), InputKind::Char);
    let r = fresh_splash.submit_char(b' ');
    push_turn(&mut splash_state, &r);
    splash_state.pager.arm_after_turn(
        pager::baseline_before(splash_state.last_transcript_total_rows, false),
        fresh_splash.pending_input(),
        pager::more_suppressed(&fresh_splash),
        Driver::PlayerInput,
    );

    let m2 = measure(&fresh_splash, &splash_state, 80, 30);
    assert!(m2.transcript_surface, "premise: the frame past the splash has a real transcript surface");
    pager::apply_frame(&mut splash_state, m2.max_scroll, m2.viewport_rows, m2.prompt_rows, m2.total_rows, m2.transcript_surface);
    assert!(!splash_state.pager.active, "splash: the resumed transcript must not page — it was already read");
    assert_eq!(splash_state.transcript_scroll, 0, "splash: the view calibrates to the bottom, not the top");
}
