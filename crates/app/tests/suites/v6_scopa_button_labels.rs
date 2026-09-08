//! SQ-0721: a scopa button's label must stay inside the button the game drew.
//!
//! Selecting a card in scopa relabels the confirm button from "Choose" to "OK", and
//! the player saw the OK label spread rightwards out of its rounded outline. What
//! spreads is the label's white FIELD, not its glyphs — the same defect as SQ-0706's
//! left-side bleed, one edge over, and it survived that fix.
//!
//! The game's own Inform source names the mechanism. Every button label is printed
//! into ONE scratch window (window 5), moved and resized for each draw:
//!
//! ```text
//! measure [ ...; @window_size 5 1000 1000; ... ],      ! left at the sentinel
//! draw    [ ...; @window_size 5 h w; @move_window 5 y x; ... print (string) self.text; ]
//! ```
//!
//! so by render time that window's box describes nothing: `measure` leaves it at a
//! 1000×1000 sentinel that the interpreter clamps to the screen. SQ-0519 floods the
//! full WINDOW WIDTH of every explicit-background text row so a status band printed
//! as separate runs reads as one solid bar; SQ-0706 gated that on the runs being
//! CONTAINED by the window box, which is what stopped "abort" (runs 567..607, box
//! 579..640 — outside on the left) from smearing. "OK" is contained: its run is
//! 579..595 inside that same 579..640 box, starting exactly ON the left edge. So it
//! flooded white from the glyphs out to the screen edge, 15 px past the outline's
//! right edge at 625. "Choose" never showed it — its run, 563..611, sits in a box
//! that draw() had just sized to it.
//!
//! Containment was the wrong question. A window is the bar only when its runs REACH
//! BOTH of its edges; the fix asks that instead.
//!
//! The story is gitignored (CLAUDE.md), so these skip cleanly when it is absent.


use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::GameSession;
use image::Rgba;

use crate::fixture_paths::fixture_path;


/// scopa's baize, which is what must show beside a button: ZMSD §8.3.1 green
/// through the pixel path.
const BAIZE: Rgba<u8> = Rgba([0, 132, 0, 255]);

/// The two buttons, as the game's own `Button.clear` lays them out. `xc` = 590 for
/// both (`GAMEWID-50`), `wid` 60, `hei` 30, `graphics.xoffs`/`yoffs` = 1, so each
/// outline's widest band runs `xc - wid/2 - 5 + 1 - 1` = 555 to 625 (0-based, end
/// exclusive) and the label sits on a `FONT_H` row inside it.
const OUTLINE_L: u32 = 555;
const OUTLINE_R: u32 = 625;
/// (name, label row top) — "abort" at `yc` = 20, the confirm button at `yc` = 380;
/// each label run starts one row below the outline's top rounded band.
const BUTTONS: [(&str, u32); 2] = [("abort", 10), ("confirm", 370)];
const FONT_H: u32 = 16;

fn boot() -> Option<GameSession> {
    let path = fixture_path("scopa.z6");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(
        bytes, true, false, None, false, dims, picts.std_window(), None, None,
    )
    .expect("scopa is a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    Some(s)
}

fn click(s: &mut GameSession, x: u16, y: u16) {
    Engine::set_mouse(s, y, x);
    let _ = s.submit_char(254);
    let _ = s.take_transcript();
}

/// Every pixel-positioned run on the screen, with the box of the window carrying it.
fn runs(s: &GameSession) -> Vec<(String, u32, u32, u32, u32)> {
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let mut out = Vec::new();
    for it in items {
        if let WinNode::Grid(g) = &it.node {
            for t in &g.px_texts {
                let x0 = u32::from(t.x.max(1)) - 1;
                out.push((
                    t.text.clone(),
                    x0,
                    x0 + t.text.chars().count() as u32 * 8,
                    u32::from(it.x_px),
                    u32::from(it.x_px) + u32::from(it.w_px),
                ));
            }
        }
    }
    out
}

fn label(s: &GameSession, text: &str) -> Option<(String, u32, u32, u32, u32)> {
    runs(s).into_iter().find(|r| r.0 == text)
}

/// Boot, pick the Milanese deck off the menu, and play on until the confirm button
/// reads "Choose" — the game asking the player to select a card.
fn scopa_at_the_players_turn() -> Option<GameSession> {
    let mut s = boot()?;
    click(&mut s, 250, 350); // the sample Milanese card on the opening menu
    for _ in 0..40 {
        if label(&s, "Choose").is_some() {
            return Some(s);
        }
        click(&mut s, 590, 380); // the confirm button, to walk past the computer's turn
    }
    panic!("scopa never reached the player's turn — no \"Choose\" label appeared");
}

/// …and then click a card in hand, which is what relabels the button to "OK".
fn select_a_card(s: &mut GameSession) {
    // The three hand cards are centred at x = 320 + (idx-1)*44 on the bottom row
    // (`ButtonCard.measure`); only cards with a legal move respond.
    for x in [276u16, 320, 364] {
        click(s, x, 365);
        if label(s, "OK").is_some() {
            return;
        }
    }
    panic!("no hand card could be selected — the button never became \"OK\"");
}

/// The composite the player sees, in a given colour mode.
fn composite(s: &GameSession, honor: bool) -> image::RgbaImage {
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.honor_game_colours = honor;
    *state.v6_paint.borrow_mut() = s.paint_surface();

    let (canvas, _metrics) =
        app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    canvas
}

/// Nothing but baize either side of a button, on the rows its label occupies.
fn assert_labels_stay_inside(canvas: &image::RgbaImage, honor: bool, when: &str) {
    for (name, top) in BUTTONS {
        for y in top..top + FONT_H {
            for (side, x) in (0..OUTLINE_L)
                .rev()
                .take(20)
                .map(|x| ("left of", x))
                .chain((OUTLINE_R..canvas.width()).map(|x| ("right of", x)))
            {
                assert_eq!(
                    *canvas.get_pixel(x, y),
                    BAIZE,
                    "{when}, honor={honor}: the {name} button's label must stay inside the \
                     outline the game drew (555..{OUTLINE_R}) — it painted over the baize \
                     {side} it at ({x},{y})"
                );
            }
        }
    }
}

/// The premise, in the game's own numbers: the scratch window's box is not the
/// button, and after a card is selected the "OK" run sits inside it anyway.
///
/// This is the whole bug in one assertion — a containment test cannot tell these
/// two apart, so it cannot be the discriminator.
#[test]
fn the_label_window_box_describes_nothing() {
    let Some(mut s) = scopa_at_the_players_turn() else { return };

    let (_, lo, hi, box_l, box_r) = label(&s, "Choose").expect("the player's turn shows \"Choose\"");
    assert_eq!((lo, hi), (563, 611), "\"Choose\" is printed centred in the button");
    assert!(
        lo >= box_l && hi <= box_r,
        "\"Choose\" is drawn into a window draw() had just sized to it: {lo}..{hi} in {box_l}..{box_r}"
    );

    select_a_card(&mut s);

    let (_, lo, hi, box_l, box_r) = label(&s, "OK").expect("selecting a card relabels the button");
    assert_eq!((lo, hi), (579, 595), "\"OK\" is printed centred in the same button");
    assert!(
        hi + 8 < box_r,
        "the premise: by render time the scratch window has been left at its size \
         sentinel, so the \"OK\" run ends {} px short of the box's right edge \
         ({lo}..{hi} in {box_l}..{box_r}) — flooding that box is what smeared the label",
        box_r - hi
    );
    assert!(lo >= box_l && hi <= box_r, "…yet it IS contained by the box, which is why SQ-0706's test passed it");
}

/// The label stays inside its outline once a card is selected — the reported bug.
///
/// Both colour modes: the label's background is a colour the game named, so
/// declining game colours must not change where it lands either.
///
/// Falsified by restoring SQ-0706's containment test
/// (`let spans_window = lo >= ox && hi <= ox + win_w;`):
/// "a card is selected, honor=true: the confirm button's label must stay inside the
///  outline the game drew (555..625) — it painted over the baize right of it at
///  (625,370)".
#[test]
fn a_selected_card_does_not_spread_the_ok_label() {
    let Some(mut s) = scopa_at_the_players_turn() else { return };

    for honor in [true, false] {
        assert_labels_stay_inside(&composite(&s, honor), honor, "the player's turn");
    }

    select_a_card(&mut s);

    for honor in [true, false] {
        let canvas = composite(&s, honor);
        assert_labels_stay_inside(&canvas, honor, "a card is selected");

        // …and the button is still a button: its white field and the OK glyphs on it.
        // Without this the test would pass on a screen that had lost the button.
        let ink = (370..386)
            .flat_map(|y| (579..595).map(move |x| (x, y)))
            .filter(|&(x, y)| canvas.get_pixel(x, y)[1] < 64)
            .count();
        assert!(
            ink > 40,
            "honor={honor}: the \"OK\" glyphs must still be drawn on the button; {ink} dark pixels"
        );
        assert_eq!(
            *canvas.get_pixel(OUTLINE_R - 1, 376),
            Rgba([255, 255, 255, 255]),
            "honor={honor}: the button's own white field still reaches its outline"
        );
    }
}
