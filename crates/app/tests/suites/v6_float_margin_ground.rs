//! SQ-0827 — the margin a window-0 inline float reserves must sit on the same
//! ground as the prose that flows beside it.
//!
//! **Reported by eye**, on `Zork Zero - The Revenge of Megaboz.adf` (release 366
//! / serial 890323 — the Amiga floppy, a DIFFERENT build from
//! `zork0-r393-s890714.z6`) under the Amiga profile: a dark band down the RIGHT
//! EDGE of every inline float, beside the ornate drop-cap letter and beside the
//! small room icon alike. Sampled from the screenshot, the game's page read
//! RGB(163,163,163) and the band RGB(58,58,58).
//!
//! Measured cause. A `MarginLeft` float reserves `pad` columns and the wrap emits
//! them as LEADING SPACES on every row of prose beside the picture
//! (`wrap_lines_kinded`); `shift_runs` then moves the line's own style runs right
//! past them, so those spaces are covered by no run at all and draw in the row's
//! BASE style. Off the Amiga that base is the theme's `transcript` — which names
//! no background, so the reserved cells simply keep the story page already
//! flooded under them and the band is invisible. Under §8.3's Amiga interpreter
//! the base carries the MACHINE's pair instead (SQ-0740 / SQ-0822,
//! `v6_machine_page`), whose page is *dark grey*, while Zork Zero's window 0
//! declares its own *light grey* page that reaches the cells through the prose's
//! per-run backgrounds. The two grounds part company, and the reserved columns
//! are where you see it. Measured at 120x50 on a halfblocks picker: the drop-cap
//! float reserves 18 columns for a 16-column picture and the room icon 11 for an
//! 8-column one, so the stripe is 2 and 3 cells wide there — the user's single
//! cell is the same defect at their cell size.
//!
//! So the Amiga profile is the only place the bug can show, and the IBM PC
//! profile is the CONTROL rather than merely a regression risk: the user
//! confirmed "the zork zero color issue is not seen with the ibmpc interpreter".
//! Every case below pins the RELATIONSHIP — the reserved cell's ground equals the
//! prose's ground — rather than a colour constant, so it survives a palette or
//! theme change that a hardcoded RGB would not.
//!
//! Both `honor_game_colours` modes are pinned. With the game's colours declined
//! the machine publishes no pair either, so the two grounds coincide again and
//! nothing may move.
//!
//! The story media are gitignored, so every case **skips cleanly** when absent.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::GameSession;
use app::state::AppState;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The Amiga release floppy the defect was reported on.
const AMIGA_RELEASE: &str = "Zork Zero - The Revenge of Megaboz.adf";

/// A pane roomy enough for the chrome ring plus a real story viewport.
const PANE: Rect = Rect { x: 0, y: 0, width: 120, height: 50 };

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Zork Zero off its Amiga floppy under `profile`, and accumulate the boot
/// banner through the real elems pipeline so `transcript_images` carries the
/// window-0 floats (the drop-cap and the room icon) exactly as a live session
/// would.
fn zork0_with_floats(profile: InterpreterProfile, honor: bool) -> Option<(GameSession, AppState)> {
    let story_path = stories_dir().join(AMIGA_RELEASE);
    let story_bytes = match app::hints::load_story(&story_path) {
        Ok(app::hints::LoadedStory::ZCode(b)) => b,
        _ => {
            eprintln!("SKIP: gitignored media missing at {}", story_path.display());
            return None;
        }
    };
    let mut picts = PictSource::resolve(&story_path, None);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut session = GameSession::new_with_trace(
        story_bytes,
        honor,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .expect("Zork Zero (v6) should load and boot without a ZError");
    // SQ-1393: the machine's own colour table. `new_with_trace` is the
    // no-machine door and presents §8.3.1's own, so a harness that boots a
    // press states the press's table here.
    session.machine.set_palette(profile.palette());
    assert!(!session.quit && session.machine.fault_trace.is_none(), "Zork Zero booted cleanly");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    let elems = Engine::take_transcript_elems(&mut session);
    app::state::apply_transcript_elems(&mut state, &elems);
    Some((session, state))
}

/// One float row's measurement: the row, the column its prose starts at, the
/// background of that first prose cell (the ground the prose sits on) and the
/// background of the reserved cell immediately left of it (the band's right edge
/// — the very cell the report points at).
#[derive(Debug)]
struct FloatRow {
    row: u16,
    prose_col: u16,
    prose_bg: ratatui::style::Color,
    margin_bg: ratatui::style::Color,
}

/// Render the frame and measure every row where prose flows beside a picture.
///
/// A float row is recognised from the frame alone, so the test never
/// re-implements the wrap's geometry: its prose starts at some column `t > 0` and
/// at least one column left of `t` carries a glyph (the picture, drawn as
/// halfblock cells). That excludes an ordinary paragraph indent, whose leading
/// columns are all blank, and a band row, which has no prose at all.
///
/// The cell at `t - 1` is reserved margin and never picture: the wrap always
/// leaves a gutter between the two (`float_text_indent` adds a column when the
/// game names no margin, and Zork Zero's own `set_margins` values are wider than
/// its pictures — 18 columns for a 16-column drop-cap, 11 for an 8-column icon).
fn float_rows(buf: &Buffer, vp: Rect) -> Vec<FloatRow> {
    let glyph = |x: u16, y: u16| buf.cell((x, y)).and_then(|c| c.symbol().chars().next()).unwrap_or(' ');
    let mut out = Vec::new();
    for y in vp.y..vp.bottom() {
        let Some(t) = (vp.x..vp.right()).find(|&x| glyph(x, y).is_alphanumeric()) else { continue };
        if t == vp.x {
            continue; // flush-left prose: no float
        }
        if !(vp.x..t).any(|x| glyph(x, y) != ' ') {
            continue; // blank leading columns: a paragraph indent, not a picture
        }
        assert_eq!(glyph(t - 1, y), ' ', "row {y}: the cell left of the prose is reserved margin, not art");
        out.push(FloatRow {
            row: y,
            prose_col: t,
            prose_bg: buf.cell((t, y)).unwrap().bg,
            margin_bg: buf.cell((t - 1, y)).unwrap().bg,
        });
    }
    out
}

/// Boot, render, and hand back the state and the float rows of the frame.
fn measure(profile: InterpreterProfile, honor: bool) -> Option<(AppState, Vec<FloatRow>)> {
    let (session, state) = zork0_with_floats(profile, honor)?;
    let model = session.screen();
    let mut buf = Buffer::empty(PANE);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, PANE, &mut buf);
    let vp = state
        .transcript_geom
        .get()
        .expect("hybrid renders window 0 as a terminal transcript")
        .area;
    let rows = float_rows(&buf, vp);
    assert!(
        rows.len() >= 4,
        "premise: the boot banner really does float prose beside its drop-cap and its room icon \
         (found {} such rows)",
        rows.len()
    );
    Some((state, rows))
}

/// **Amiga, colours honoured — the reported case.**
///
/// The machine publishes a pair, so the transcript's base style is the machine's
/// page while the prose sits on the page window 0 declared. Every reserved cell
/// must take the prose's page: that is the whole fix.
#[test]
fn amiga_float_margin_takes_the_page_the_prose_sits_on() {
    let Some((state, rows)) = measure(InterpreterProfile::Amiga, true) else { return };

    // The premise that makes this case non-vacuous: this is the one machine whose
    // own pair underlies the transcript, and the story window declares a page of
    // its own for the prose to sit on.
    assert!(
        state.v6_page_pair.get().is_some(),
        "premise: under §8.3's Amiga interpreter the machine publishes a screen pair — without one \
         the base and the page coincide and nothing here could go wrong"
    );
    let page = state.v6_story_page.get().expect("Zork Zero declares its own window-0 page");
    let page = ratatui::style::Color::Rgb(page.0, page.1, page.2);

    for f in &rows {
        eprintln!("Amiga float row {} prose@{} prose_bg={:?} margin_bg={:?}", f.row, f.prose_col, f.prose_bg, f.margin_bg);
    }
    for f in &rows {
        assert_eq!(
            f.prose_bg, page,
            "row {}: the prose sits on the game's own page",
            f.row
        );
        assert_eq!(
            f.margin_bg, f.prose_bg,
            "row {}: the column reserved beside the float ({}) must carry the page the prose sits \
             on, not the transcript's base style — this is the dark band down the float's right edge",
            f.row,
            f.prose_col - 1
        );
    }
}

/// **IBM PC, colours honoured — the control.** The machine publishes no pair, so
/// the transcript's base is the theme's own and the two grounds already agree.
/// The relationship must hold here exactly as it did before the fix; nothing may
/// move on this profile.
#[test]
fn ibmpc_float_margin_and_prose_already_share_a_ground() {
    let Some((state, rows)) = measure(InterpreterProfile::IbmPc, true) else { return };

    assert!(
        state.v6_page_pair.get().is_none(),
        "premise: outside the Amiga profile no machine pair is published — which is exactly why the \
         band was never visible here"
    );
    for f in &rows {
        eprintln!("IbmPc float row {} prose@{} prose_bg={:?} margin_bg={:?}", f.row, f.prose_col, f.prose_bg, f.margin_bg);
        assert_eq!(
            f.margin_bg, f.prose_bg,
            "row {}: the reserved column and the prose share one ground on the IBM PC too",
            f.row
        );
    }
}

/// **Amiga, colours declined.** The theme is authoritative: no machine pair, no
/// story page, and a run's own background is dropped — so the reserved column and
/// the prose fall back to the same base and must still agree.
#[test]
fn amiga_float_margin_holds_with_the_games_colours_declined() {
    let Some((state, rows)) = measure(InterpreterProfile::Amiga, false) else { return };

    assert!(state.v6_page_pair.get().is_none(), "colours declined: no machine pair to lay down");
    for f in &rows {
        assert_eq!(
            f.margin_bg, f.prose_bg,
            "row {}: with the game's colours declined the margin still shares the prose's ground",
            f.row
        );
    }
}

/// **IBM PC, colours declined.** The fourth corner of the pair, for completeness.
#[test]
fn ibmpc_float_margin_holds_with_the_games_colours_declined() {
    let Some((_state, rows)) = measure(InterpreterProfile::IbmPc, false) else { return };
    for f in &rows {
        assert_eq!(f.margin_bg, f.prose_bg, "row {}: margin and prose share one ground", f.row);
    }
}
