//! SQ-1462: "hint menu not showing in Counterfeit Monkey" — the user's own
//! diagnosis was exact: "the text color in the hint menus [is] very light,
//! almost matching the background."
//!
//! # What was actually invisible
//!
//! Not lanthorn's own `/open-hints` companion panel (`render::hints_panel`) —
//! Counterfeit Monkey ships no local InvisiClues-style hint file, so that panel
//! never opens for it at all (`hints::resolve_hint_source` finds nothing, and
//! `open_hints` in `main.rs` only sets a status message). The game's OWN
//! in-game `HINT` menu is what the report is about: reaching it means typing
//! through Counterfeit Monkey's accessibility gate first ("Can you hear me?",
//! a name, a blank keypress, `TUTORIAL OFF`) and every one of those typed
//! answers gets ECHOED back into the transcript by the game itself, with Glk
//! style `Input` (glk_style 8) and no colour of its own — Glk leaves "how the
//! player's own input looks" to the interpreter, a convention, not a game
//! choice. (`HINT` itself is typed too, but Inform 7's menu clears and
//! redraws the primary buffer the same turn — SQ-0403's "an Inform 7 menu
//! clears the primary buffer and reprints" — so that one specific echo never
//! survives to be rendered; "yes"/"andra"/"tutorial off", the answers reaching
//! the SAME menu, do.)
//!
//! Counterfeit Monkey sets its Normal style to black-on-white
//! (`glulx_game_colours.rs`'s `cm_intro_pane_adopts_the_games_black_on_white`
//! confirms the story pane adopts it), and the theme's generic `text` role —
//! what an UNcoloured Glk run falls through to — is white with no background,
//! a guess for a dark terminal. So every echoed keystroke rendered white text
//! on the same white page the surrounding black prose sits on: not merely low
//! contrast, invisible. The player could not see what they had typed while
//! working through the game's accessibility gate on the way into (and every
//! typed answer around) its hint system, which reads exactly like "the hint
//! menu isn't showing."
//!
//! The fix: `render::TextInk` carries the game's own known page/ink pair
//! (`render::screen::game_input_style`, already computed once per frame for the
//! live, unconfirmed input line — SQ-0847) as a floor `draw_str_runs` patches
//! under `base_style` before resolving an uncoloured Glk run, so the SAME
//! already-honoured page the surrounding prose sits on is what an uncoloured
//! echo floors on too, instead of the generic theme guess. A game that never
//! declares a page (`game_input` is `None`) — every plain Z-machine story,
//! `minizork` included — renders exactly as before.
//!
//! Two engines, per CLAUDE.md's testing conventions (colour/render areas pin
//! real fixtures on more than one engine so a fix for one cannot regress the
//! other): Counterfeit Monkey (Glulx, `stories/`-only — release 11 / serial
//! 230220, the only local copy; `glulx_game_colours.rs` already established it
//! sets the white-on-black style hint the way this needs. Release 10, the IF
//! Archive's fetchable copy, does NOT set it the same way at boot per SQ-1454
//! and is not used here) and `minizork` (Z-machine, tracked in the checkout at
//! `crates/zvm/tests/fixtures/minizork.z3`) as the regression guard — it never
//! declares a page, so `game_input` must be `None` and its rendering must be
//! unchanged.

use app::engine::Engine;
use app::glulx_session::GlulxSession;
use app::session::GameSession;
use app::state::{AppState, TranscriptKind};
use blorb::Blorb;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

const PANE: Rect = Rect { x: 0, y: 0, width: 80, height: 30 };

fn fresh_state(honor: bool) -> AppState {
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.honor_game_colours = honor;
    state
}

fn render(model: &app::engine::ScreenModel, state: &AppState) -> Buffer {
    let mut buf = Buffer::empty(PANE);
    let _ = app::render::screen::render_story_pane(model, false, None, state, PANE, &mut buf);
    buf
}

/// The `(fg, bg)` of the first cell of the first row containing `needle` as an
/// exact, whole-line match (so "andra" doesn't accidentally match inside a
/// longer sentence), or `None` if no row equals it.
fn cell_colour_of_exact_line(buf: &Buffer, needle: &str) -> Option<(Color, Color)> {
    for y in PANE.y..PANE.bottom() {
        let row: String = (PANE.x..PANE.right())
            .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        if row.trim_end() == needle {
            let cell = buf.cell((PANE.x, y)).unwrap();
            return Some((cell.fg, cell.bg));
        }
    }
    None
}

/// Squared Euclidean distance between two concrete RGB colours — a simple
/// colour difference is enough to tell "invisible" from "legible" (SQ-1462
/// doesn't need WCAG). `Color::Reset` (an unpainted channel, "whatever the
/// pane painted") resolves against `page_fallback`, since that's what a real
/// terminal actually shows there.
fn rgb(c: Color, page_fallback: (u8, u8, u8)) -> (i32, i32, i32) {
    match c {
        Color::Rgb(r, g, b) => (r as i32, g as i32, b as i32),
        Color::White => (255, 255, 255),
        Color::Black => (0, 0, 0),
        Color::Reset => (page_fallback.0 as i32, page_fallback.1 as i32, page_fallback.2 as i32),
        other => panic!("unexpected colour in this test: {other:?}"),
    }
}

fn contrast(a: Color, b: Color, page_fallback: (u8, u8, u8)) -> i64 {
    let (ar, ag, ab) = rgb(a, page_fallback);
    let (br, bg, bb) = rgb(b, page_fallback);
    ((ar - br).pow(2) + (ag - bg).pow(2) + (ab - bb).pow(2)) as i64
}

// ── Counterfeit Monkey (Glulx): the reported symptom ───────────────────────

/// Boot Counterfeit Monkey release 11 and answer its accessibility gate —
/// "Can you hear me?" (yes), a name (andra), a blank keypress, then
/// `TUTORIAL OFF` — landing on the game's own hint-menu topics list. Every one
/// of those FOUR typed answers is echoed by the game with an uncoloured
/// `Input`-class run — this is the SQ-1462 symptom's own reproduction, not a
/// synthetic one. Skips (returns `None`) when the gitignored, `stories/`-only
/// fixture is absent.
fn boot_cm_past_accessibility_gate(honor: bool) -> Option<(GlulxSession, AppState)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../stories/CounterfeitMonkey-11.gblorb");
    if !path.exists() {
        eprintln!("SKIP: stories/CounterfeitMonkey-11.gblorb absent (gitignored, stories/-only)");
        return None;
    }
    let blorb = Blorb::parse(std::fs::read(&path).unwrap()).expect("parse gblorb");
    let image = blorb.executable().expect("exec chunk").1.to_vec();
    let mut sess =
        GlulxSession::new(image, 80, 24, true, false, false, (1, 1), None, &[]).expect("boot CM");

    let mut state = fresh_state(honor);
    state.push_transcript_runs(&sess.take_transcript(), TranscriptKind::Story, &[]);
    for cmd in ["yes", "andra", "", "tutorial off"] {
        let r = sess.submit(cmd);
        state.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
    }
    Some((sess, state))
}

fn app_screen(sess: &GlulxSession) -> app::engine::ScreenModel {
    Engine::screen(sess)
}

/// The headline case: `honor_game_colours = true` (the shipped default) —
/// Counterfeit Monkey's echoed "andra" and "tutorial off" must be legible
/// against its own white page, not the invisible white-on-white the bug
/// produced.
///
/// Falsify: reverting `TextInk::game_input`/`draw_str_runs`'s `base_style`
/// patch (SQ-1462) reproduces `contrast == 0` (white text, `Color::Reset`
/// background over the white pane) on both lines — the exact symptom.
#[test]
#[ignore = "slow (~2.7s): boots Counterfeit Monkey through the Glulx VM; run with --include-ignored"]
fn cm_echoed_answers_are_legible_honor_on() {
    let Some((_sess, state)) = boot_cm_past_accessibility_gate(true) else { return };
    let buf = render(&app_screen(&_sess), &state);
    // Counterfeit Monkey's own known page (established by `glulx_game_colours.rs`).
    let page = (255u8, 255u8, 255u8);
    for needle in ["andra", "tutorial off"] {
        let (fg, bg) = cell_colour_of_exact_line(&buf, needle)
            .unwrap_or_else(|| panic!("the echoed {needle:?} must appear in the transcript"));
        let c = contrast(fg, bg, page);
        assert!(
            c > 10_000,
            "echoed {needle:?} must be legible against its own page: fg={fg:?} bg={bg:?} \
             (contrast={c}) — white-on-white (contrast 0) is the exact bug this quest fixes"
        );
        assert_ne!(
            (fg, bg),
            (Color::White, Color::Reset),
            "{needle:?} must not be the generic theme's white floating over the page — \
             the SQ-1462 symptom exactly"
        );
    }
}

/// `honor_game_colours = false`: `--game-colours off` declares the interpreter
/// colourless, so the fix must NOT apply — a colourless pane has no game
/// white-on-white to produce (the whole story renders through the theme
/// uniformly), and this confirms the fix's honor-gate actually gates. Per
/// CLAUDE.md's testing conventions: colour/render areas pin BOTH
/// `honor_game_colours` modes.
#[test]
#[ignore = "slow (~2.7s): boots Counterfeit Monkey through the Glulx VM; run with --include-ignored"]
fn cm_echoed_answers_honor_off_use_theme_uniformly() {
    let Some((_sess, state)) = boot_cm_past_accessibility_gate(false) else { return };
    let buf = render(&app_screen(&_sess), &state);
    let (fg, _bg) = cell_colour_of_exact_line(&buf, "andra")
        .expect("the echoed 'andra' must appear in the transcript");
    assert_eq!(fg, Color::White, "honor off: the generic theme base, never the game's page");
}

// ── minizork (Z-machine): the regression guard ─────────────────────────────

/// minizork never declares a page (no `set_colour` in its boot/intro path), so
/// `game_input_style` must answer `None` and an ordinary printed room heading
/// must render through the plain theme exactly as it did before SQ-1462 — the
/// fix must be inert for the overwhelming majority of stories, which never set
/// a custom page at all.
#[test]
fn minizork_unaffected_by_the_fix() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/zvm/tests/fixtures/minizork.z3");
    if !path.exists() {
        eprintln!("SKIP: crates/zvm/tests/fixtures/minizork.z3 absent");
        return;
    }
    let bytes = std::fs::read(&path).unwrap();
    let mut sess = GameSession::new(bytes, true, false, None).expect("boot minizork");

    let mut state = fresh_state(true);
    state.push_transcript_runs(&sess.take_transcript(), TranscriptKind::Story, &[]);
    let r = sess.submit("look");
    state.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);

    let model = Engine::screen(&sess);
    let buf = render(&model, &state);
    // minizork's Normal prose ("West of House", the room name LOOK prints)
    // must still be the plain theme white — unmoved by SQ-1462, since this
    // story's model declares no page of its own.
    let (fg, _bg) = cell_colour_of_exact_line(&buf, "West of House")
        .expect("minizork's LOOK must print the room heading 'West of House'");
    assert_eq!(fg, Color::White, "a story with no declared page renders the plain theme, unchanged");
}
