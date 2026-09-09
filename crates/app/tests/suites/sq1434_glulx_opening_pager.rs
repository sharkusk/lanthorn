//! SQ-1434: the opening `[more]` must not be raised by the blank rows a story
//! opens with — and must not eat the first keystroke of the first command.
//!
//! # The report
//!
//! *"On some Glulx games the intro text prints, the `>` prompt appears, but the
//! first keystroke is swallowed — the `[more]` pager is still active at that
//! moment, so the key dismisses the pager instead of reaching the input line.
//! Plenty of screen remains."* Two reproducers, both Inform 7 Glulx with a
//! status grid over a buffer window: `stories/chlorophyll.gblorb` (Release 1 /
//! serial 150212 / I7 build 6L02) and `stories/Alias 'The Magpie'.gblorb`
//! (Release 5 / serial 220210 / I7 build 6M62).
//!
//! # Measured, before the fix
//!
//! | fixture | pane | `total_rows` | `viewport_rows` | leading blank rows |
//! |---|---|---|---|---|
//! | `chlorophyll.gblorb` | 80x30 | 31 | 29 | 3 |
//! | `Alias 'The Magpie'.gblorb` | 120x44 | 44 | 42 | 3 |
//!
//! Both overflow by two rows, and in both the ENTIRE overflow is the three
//! newlines the story prints before its prologue — every Inform 7 Glulx story
//! opens with two or three of them (`advent.blb` prints six). `startup.rs`'s
//! opening-banner arm measured from row 0, so `activation_target` saw
//! `added > viewport`, engaged, and parked the view three rows back. The
//! resulting frame put the three BLANK rows across the top of the screen and
//! pushed three rows of real prose below the fold — strictly worse than not
//! paging — and, with a `Line` read pending, `input::key_to_command`'s pager gate
//! turned the player's first character into `Action::PagerDismiss`.
//!
//! That is exactly "plenty of screen remains": the screen the player is looking
//! at opens with blank rows and still says `[more]`.
//!
//! # The fix
//!
//! `pager::opening_baseline` — the banner arm starts at the first row of the boot
//! output that carries prose, not at row 0. A blank row is not text the reader
//! can miss. Nothing else about the pager moves: a banner that genuinely
//! overflows (Magpie at 80x30, where fifty rows of prologue meet a 28-row
//! viewport) still pages, and now parks on its first real row instead of on a
//! blank one.
//!
//! # Falsification
//!
//! Make `pager::opening_baseline` return `0` and both Glulx cases fail with
//! `pager.active == true` and the typed `x` resolving to `Action::PagerDismiss`
//! instead of `Action::InputChar('x')` — the reported symptom. Revert
//! `startup.rs` to `state.pager.arm(0)` and [`the_startup_arm_uses_the_opening_
//! baseline`](the_startup_arm_uses_the_opening_baseline) fails: the behavioural
//! cases mirror the startup arm rather than calling that 2,000-line function, so
//! the CALL SITE needs a guard of its own — a convention nobody can see is how
//! this regresses (CLAUDE.md, "a guard beats a convention").
//!
//! Every fixture here is gitignored except Mini-Zork I (tracked, see
//! `fixture_paths`), so the Glulx cases skip vacuously and loudly on CI.

use app::engine::Engine;
use app::glulx_session::GlulxSession;
use app::graphics::PictSource;
use app::input::{key_to_action, key_to_command, Action, KeyResolve};
use app::pager;
use app::session::{GameSession, InputKind};
use app::state::{apply_transcript_elems, AppState};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;

/// A plain, unmodified keypress — the player typing at the prompt.
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// The app's own render state, minus everything the pager does not read.
fn render_state() -> AppState {
    let mut st = AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    st
}

/// Render the story pane through the REAL renderer and return the metrics the run
/// loop feeds `pager::apply_frame` after every frame.
fn measure(engine: &dyn Engine, state: &AppState, w: u16, h: u16) -> app::render::screen::StoryPaneMetrics {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let char_mode = engine.pending_input() == InputKind::Char;
    app::render::screen::render_story_pane(&engine.screen(), char_mode, None, state, area, &mut buf)
}

/// What `startup.rs` does between the boot and the player's first key, in the
/// same order: take the banner, push it, drain the boot with `seed_turn`, arm the
/// opening-banner pager, render one frame, and let `apply_frame` resolve the arm.
///
/// Returns the leading-blank-row count and the frame's metrics alongside the
/// state, so a case can guard its own premise rather than assert into a fixture
/// that has quietly changed.
fn boot_frame(engine: &mut dyn Engine, w: u16, h: u16) -> (AppState, u16, app::render::screen::StoryPaneMetrics) {
    let mut state = render_state();
    let banner_elems = engine.take_transcript_elems();
    if banner_elems.is_empty() {
        let banner = engine.take_transcript();
        state.push_transcript(&banner);
    } else {
        apply_transcript_elems(&mut state, &banner_elems);
    }
    let _seed = engine.seed_turn();
    // Counted here rather than through `pager::opening_baseline`, so a case's
    // PREMISE never depends on the function under test: neutering that function
    // must fail these cases on the reported symptom, not on their own guard.
    let blanks = state.transcript.iter().take_while(|l| l.trim().is_empty()).count() as u16;
    if pager::should_arm(engine.pending_input(), pager::more_suppressed(engine)) {
        state.pager.arm(pager::opening_baseline(&state));
    }
    let m = measure(engine, &state, w, h);
    pager::apply_frame(&mut state, m.max_scroll, m.viewport_rows, m.prompt_rows, m.total_rows, m.transcript_surface);
    (state, blanks, m)
}

/// Boot a Glulx story the way `glulx_session.rs` is driven at startup, at the
/// pane size the case is about. `None` when the gitignored fixture is absent.
fn boot_glulx(file: &str, w: u16, h: u16) -> Option<GlulxSession> {
    let path = fixture_path(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let pict_blorb = blorb::Blorb::parse(bytes.clone()).ok();
    let app::hints::LoadedStory::Glulx(image) =
        app::hints::extract_story(bytes).expect("readable container")
    else {
        panic!("{file} is a Glulx story");
    };
    Some(
        GlulxSession::new(image, w.into(), h.into(), true, false, false, (8, 16), pict_blorb, &[])
            .expect("Glulx story boots"),
    )
}

/// Boot a Z-machine story off its bare file — the control's engine.
fn boot_zmachine(file: &str) -> Option<GameSession> {
    let path = fixture_path(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut session =
        GameSession::new_with_trace(bytes, true, false, None, false, dims, std_window, None, None)
            .expect("story boots");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    Some(session)
}

/// The whole report, on one fixture: the opening frame must not be paging, and
/// the player's first character must reach the input line.
///
/// `expect_overflow_without_the_fix` is the non-vacuity guard the pinned table
/// above exists for — the case is only about SQ-1434 while the banner really does
/// out-measure the viewport by no more than its leading blanks. If a fixture or
/// the wrap changes so that it fits outright, this fails rather than passing for
/// the wrong reason.
fn assert_opening_prompt_takes_the_first_key(
    engine: &mut dyn Engine,
    what: &str,
    w: u16,
    h: u16,
    expected_blanks: u16,
) {
    let (state, blanks, m) = boot_frame(engine, w, h);

    assert_eq!(
        blanks, expected_blanks,
        "{what}: premise — this story opens with {expected_blanks} blank rows before its prologue"
    );
    assert!(
        m.transcript_surface,
        "{what}: premise — the opening frame lays the transcript out"
    );
    assert!(
        m.total_rows > m.viewport_rows,
        "{what}: premise — the banner out-measures the viewport ({} rows into {}), which is what \
         raised the [more] before the fix; without that this case proves nothing",
        m.total_rows,
        m.viewport_rows,
    );
    assert!(
        m.total_rows.saturating_sub(blanks) <= m.viewport_rows,
        "{what}: premise — the overflow is entirely the leading blank rows ({} rows, {blanks} \
         blank, {} viewport)",
        m.total_rows,
        m.viewport_rows,
    );

    // ── The report ──────────────────────────────────────────────────────────
    assert!(
        !state.pager.active,
        "{what}: the opening banner fits once its blank rows stop counting as prose — the pager \
         must not engage"
    );
    assert_eq!(
        state.transcript_scroll, 0,
        "{what}: and the view stays at the bottom, on the prompt"
    );
    assert_eq!(
        key_to_action(&state, key(KeyCode::Char('x'))),
        Action::InputChar('x'),
        "{what}: the player's first character must reach the input line, not the pager"
    );
    assert!(
        matches!(
            key_to_command(&state, key(KeyCode::Char('x'))),
            KeyResolve::Action(Action::InputChar('x'))
        ),
        "{what}: and the key gate must not intercept it either"
    );
}

/// Chlorophyll at 80x30: 31 wrapped rows into a 29-row viewport, three of them
/// the newlines before the prologue.
#[test]
fn chlorophyll_opening_prompt_takes_the_first_key() {
    let Some(mut s) = boot_glulx("chlorophyll.gblorb", 80, 30) else { return };
    assert_opening_prompt_takes_the_first_key(&mut s, "chlorophyll.gblorb 80x30", 80, 30, 3);
}

/// Alias 'The Magpie' at 120x44: 44 wrapped rows into a 42-row viewport (its
/// status grid is two rows), three of them the newlines before the prologue.
#[test]
fn magpie_opening_prompt_takes_the_first_key() {
    let Some(mut s) = boot_glulx("Alias 'The Magpie'.gblorb", 120, 44) else { return };
    assert_opening_prompt_takes_the_first_key(&mut s, "Alias 'The Magpie'.gblorb 120x44", 120, 44, 3);
}

/// The other half of the ruleset, on the same fixture: a banner that genuinely
/// overflows still pages. Magpie's fifty rows of prologue into a 28-row viewport
/// at 80x30 is not a blank-row artefact, and the fix must not silence it — the
/// pager exists for exactly this frame.
#[test]
fn magpie_still_pages_when_the_prologue_really_does_overflow() {
    let Some(mut s) = boot_glulx("Alias 'The Magpie'.gblorb", 80, 30) else { return };
    let (state, blanks, m) = boot_frame(&mut s, 80, 30);
    assert!(
        m.total_rows.saturating_sub(blanks) > m.viewport_rows,
        "premise: {} rows of prose ({blanks} blank) genuinely overflow a {}-row viewport",
        m.total_rows,
        m.viewport_rows,
    );
    assert!(state.pager.active, "a banner that really does overflow still raises [more]");
    assert!(state.transcript_scroll > 0, "and the view parks back on the first screenful");
}

/// The call site, guarded at the source (SQ-1434).
///
/// The cases above mirror `startup.rs`'s opening-banner arm rather than calling
/// `startup.rs` — booting the app for real needs a terminal, a config directory
/// and a story picker — so nothing in them can see the arm going back to a bare
/// `arm(0)`. `state.pager.arm(…)` appears exactly once in production, and this
/// asks that the once names `opening_baseline`.
#[test]
fn the_startup_arm_uses_the_opening_baseline() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/startup.rs"),
    )
    .expect("startup.rs is in this crate");
    let arms: Vec<&str> = src
        .match_indices("state.pager.arm(")
        .map(|(i, _)| src[i..].lines().next().unwrap_or_default())
        .collect();
    assert_eq!(
        arms.len(),
        1,
        "startup.rs should arm the opening-banner pager exactly once; found {arms:?}"
    );
    assert!(
        arms[0].contains("opening_baseline"),
        "the opening-banner arm must start at the first row of PROSE, not at a literal row \
         (SQ-1434) — found `{}`",
        arms[0].trim()
    );
}

/// The engine control: a Z-machine boot shows neither the symptom nor a change.
/// Mini-Zork I (r34 / s871124) is tracked, so this runs on CI where the Glulx
/// cases above skip.
#[test]
fn zmachine_opening_prompt_takes_the_first_key() {
    let Some(mut s) = boot_zmachine("minizork-r34-s871124.z3") else { return };
    let (state, _blanks, m) = boot_frame(&mut s, 80, 30);
    assert!(m.transcript_surface, "premise: the opening frame lays the transcript out");
    assert!(
        m.total_rows <= m.viewport_rows,
        "premise: Mini-Zork's banner fits ({} rows into {}) — the control never had the symptom",
        m.total_rows,
        m.viewport_rows,
    );
    assert!(!state.pager.active, "a Z-machine boot does not page a banner that fits");
    assert_eq!(
        key_to_action(&state, key(KeyCode::Char('x'))),
        Action::InputChar('x'),
        "the first character reaches the input line"
    );
}
