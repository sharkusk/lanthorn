//! SQ-0729: fmvpoker drew NO graphics at all in hybrid, and its menu never
//! reached the pixel composite.
//!
//! **A ring around a full-screen window has nowhere to draw.** Hybrid shows
//! artwork as bands AROUND the story viewport, so a story window covering the
//! screen leaves `chrome_bands` empty and no art can be uploaded whatever its
//! shape. That is what `picture_takeover` exists to catch — such a frame goes to
//! the composite instead — but it asked whether the art FILLED the screen, sampling
//! an 8x8 grid and requiring every point painted. fmvpoker paints a 640x400 poker
//! table into full-screen window 0 and prints its whole title inside it, and that
//! table is a FRAME: 42830 of 256000 pixels opaque, the middle a hole. It missed at
//! every point that mattered, hybrid kept its empty ring, and the game drew not one
//! picture. The rule now also fires when the art ENCLOSES the screen — painted
//! pixels within a native text row of all four edges.
//!
//! `fmvpoker_is_the_only_title_this_moves` is the guard on that. Hybrid is the
//! shipped default, so a frame that stops taking the ring renders as a pixel image
//! instead of crisp cells — a mode change every v6 player would see.
//!
//! **The composite skipped a whole class of window.** Routing those frames here
//! exposed the second half: `build_chrome_canvas` draws Graphics and Grid windows
//! and never a secondary prose `Buffer` (SQ-0585). fmvpoker prints its bottom menu
//! and "Select an option with your mouse or by typing the first letter." into one,
//! so raster showed neither while both cell paths showed both — and hybrid would
//! have lost them on arrival. They are drawn now, one 16px row each from the
//! window's own origin, in ink the caller resolves against `honor_game_colours`
//! (painting fmvpoker's declared black regardless put black glyphs on the host's
//! black page), and `fill_story_page_under_chrome_text` spares them.
//!
//! **The menu labels kept their columns** (SQ-0729, second pass). They used to
//! arrive from the model already concatenated, as
//! `lines[0] == "PLAY CURRENT BETCHANGE CURRENT BETSAVERESTOREQUIT"`: the game
//! prints them at five separate columns, but a v6 window in flowing-prose mode
//! streams through `ZWindow::push_prose`, which kept no cursor x, and that window's
//! `texts` and `streamed` are both empty — so the positions were lost inside zvm
//! before any render code saw them. `push_prose` now takes the column its own
//! `set_cursor` declared and pads the line out to it, the same declaration the
//! streaming path already carried as an indent. `fmvpoker_menu_labels_keep_their
//! _columns` pins the result against the game's own `set_cursor` operands.
//!
//! **…and then could not be clicked** (SQ-0738). With the labels finally in the
//! right columns the player still could not press one. Input was never the
//! problem — the VM received every click — but the same conversion FLOORED the
//! declared row, `(80-1)/16 = 4`, and a diverted window's lines are stacked 16 px
//! apart from its origin, so the five labels were drawn on native rows 300..315
//! while the game's own hit test accepts 316..333 for them. Zero overlap: the
//! drawn label was dead and the blank row beneath it worked. The row rounds to the
//! nearest line now, which is what `row=80` means, and `MENU_LINE` moved 4 → 5
//! with it. `fmvpoker_menu_labels_are_clickable_where_drawn_*` is the guard, and
//! it is behavioural: it finds each label's own pixels on the composite, clicks
//! the terminal cell showing them through the app's own click map, and requires
//! the game to act.
//!
//! **The frame vanished for the duration of a bet** (SQ-0739). Choosing "CHANGE
//! CURRENT BET" makes the 594x156 bottom panel the window the game reads through, so
//! the panel becomes the primary `Buffer` and window 0 — still holding the poker
//! table drawn into it — stops being the story window. Nothing was redrawn: the
//! table did not move, the story window did. But `classify_windows` keeps window 0's
//! plate out of the chrome canvas on the grounds that it belongs to the story, and
//! that reasoning holds only while the plate is inside the window it belongs to.
//! With the story viewport reduced to one bottom panel, the escaping art was in
//! neither half — no band carried it and no viewport showed it — so hybrid built a
//! ring canvas of ZERO opaque pixels and the player's frame disappeared while they
//! typed. `fmvpoker_bet_entry_keeps_its_frame` is the guard.
//!
//! **…and the arm that fixed it is gone** (SQ-0897). SQ-0746 removed its premise:
//! reading through a panel the game declares is NOT its transcript never made that
//! panel the story window, so window 0 never stops being it and nothing escapes.
//! SQ-0896 removed its need: the ring's canvas and its art oracle now carry window
//! 0's plate at its own native origin wherever that is. MEASURED false on every
//! frame of all eight corpus titles before removal — its own corpus test had
//! asserted exactly that since SQ-0746. That test is now
//! `picture_takeover_arms_across_the_corpus`, which records which arm carries each
//! frame instead of which one does not, because a boolean cannot say why a frame is
//! where it is when four predicates are OR'd over it.
//!
//! `stories/fmvpoker.blb` is a byte-identical copy of `stories/Zork0.blb`: the
//! original release ships Zork Zero's picture file renamed to FMVPOKER.EG1, so the
//! table is a Zork Zero picture drawn deliberately. Both `honor_game_colours` modes
//! are pinned. Stories are gitignored (CLAUDE.md), so these skip cleanly.


use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;


fn boot(name: &str, honor: bool) -> Option<(GameSession, app::state::AppState)> {
    let path = fixture_path(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(
        bytes, honor, false, None, false, dims, picts.std_window(), None, None,
    )
    .expect("a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    app::state::apply_transcript_elems(&mut state, &Engine::take_transcript_elems(&mut s));
    Some((s, state))
}

/// Render one hybrid frame and report the path it took.
fn render_path(session: &GameSession, state: &app::state::AppState) -> String {
    let model = session.screen();
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, state, area, &mut buf);
    state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default()
}

/// fmvpoker one keypress in: the table is drawn and the title printed inside it.
fn fmvpoker_title(honor: bool) -> Option<(GameSession, app::state::AppState)> {
    let (mut session, mut state) = boot("fmvpoker.z6", honor)?;
    let r = match session.pending_input() {
        InputKind::Char => session.submit_char(13),
        _ => session.submit(""),
    };
    assert!(r.fault.is_none(), "fmvpoker faulted: {:?}", r.fault);
    app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
    // The game's own painted ground, as the app publishes it every frame
    // (`main.rs`): fmvpoker's `erase_window` fills live here and nowhere else.
    *state.v6_paint.borrow_mut() = Engine::paint_surface(&session);
    Some((session, state))
}

fn fmvpoker_hybrid_draws_its_frame(honor: bool) {
    let Some((session, state)) = fmvpoker_title(honor) else { return };

    // Premise: window 0 covers the screen — so hybrid's ring is empty — and the
    // backdrop behind it is mostly HOLE, the shape the fill test misses.
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let story = layout.story.expect("fmvpoker publishes a story window");
    assert_eq!(
        (story.x_px, story.y_px, story.w_px, story.h_px),
        (0, 0, 640, 400),
        "premise (honor={honor}): window 0 covers the whole screen, so there is no ring to carve"
    );
    let plate = layout.story_gfx.expect("fmvpoker draws its table into window 0 (SQ-0714)");
    let WinNode::Graphics(g) = &plate.node else { panic!("story_gfx is a Graphics leaf") };
    let (pw, ph) = g.canvas.dimensions();
    let opaque = g.canvas.pixels().filter(|p| p.0[3] != 0).count();
    assert_eq!((pw, ph), (640, 400), "premise (honor={honor}): a full-screen backdrop");
    assert!(
        opaque * 3 < (pw * ph) as usize,
        "premise (honor={honor}): the table is a hollow frame — {opaque} of {} pixels painted",
        pw * ph
    );

    assert_eq!(
        render_path(&session, &state),
        "raster",
        "honor={honor}: with window 0 covering the screen the hybrid ring has no band to draw in, \
         so this frame must go to the composite — otherwise fmvpoker draws NO graphics at all \
         (SQ-0729). Its table encloses the screen; it does not fill it."
    );
}

#[test]
fn fmvpoker_hybrid_draws_its_frame_honoring_game_colours() {
    fmvpoker_hybrid_draws_its_frame(true);
}

#[test]
fn fmvpoker_hybrid_draws_its_frame_theme_only() {
    fmvpoker_hybrid_draws_its_frame(false);
}

/// The corpus guard: the enclosure arm is meant to move fmvpoker and nothing else.
///
/// Zork Zero and Shogun keep window 0 inset inside their frames, advent and scopa
/// paint nothing behind it, and Arthur's intro plate and Journey's title already
/// filled the screen.
///
/// SQ-0725 added a third arm — a full-screen story window whose plate paints
/// anything must take the composite, because `blit_story_gfx` is reachable from the
/// raster path alone — and mysterious01 is the one title in the corpus it moves.
/// Its two stacked 512x192 title cards leave the right-hand quarter of the screen
/// bare, so they satisfy neither the fill grid nor the enclosure test, and hybrid
/// drew NOTHING for them: its ring is empty when the story window covers the
/// screen. `mysterious01.z6` reads `raster` below for exactly that reason, and the
/// rest of this table is the guard that nothing else came with it.
#[test]
fn fmvpoker_is_the_only_title_this_moves() {
    const RING: &str = "hybrid-ring";
    const MENU: &str = "cell — painted menu takeover routed here";
    let expected: &[(&str, &[&str])] = &[
        ("zork0-r393-s890714.z6", &[RING; 4]),
        ("arthur-r74-s890714.z6", &["raster", RING, RING, RING]),
        // SQ-0886: Shogun's boot menu is a painted takeover WITH the game's own
        // artwork behind it — the ornate side panels and the machine's ground — and
        // the cell path draws no art at all, so it does not take MENU. advent's popup
        // below is the control that kept the cell path: same takeover shape, no
        // artwork in the game anywhere.
        //
        // SQ-0892 moved that frame from the composite to the RING, which draws the
        // panels as art and the menu as glyphs; step 0 is the splash BEFORE the menu,
        // a full-screen picture with no story window, and stays `raster`.
        ("shogun-r322-s890706.z6", &["raster", RING, RING, RING]),
        ("journey-r83-s890706.z6", &[RING, "raster", RING, RING]),
        ("advent.z6", &[RING, MENU, RING, RING]),
        ("scopa.z6", &["painted (hint/menu takeover)"; 4]),
        // SQ-0725: its title cards have no ring to live in — see above.
        ("mysterious01.z6", &["raster"; 4]),
        // …and fmvpoker, which boots with no art at all and takes the ring for that
        // one frame, then draws its table and must go to the composite from there.
        ("fmvpoker.z6", &[RING, "raster", "raster", "raster"]),
    ];
    for &(game, want) in expected {
        let Some((mut session, mut state)) = boot(game, true) else { continue };
        let mut got = Vec::new();
        for step in 0..want.len() {
            got.push(render_path(&session, &state));
            if step + 1 == want.len() {
                break;
            }
            let r = match session.pending_input() {
                InputKind::Char => session.submit_char(13),
                _ => session.submit(""),
            };
            assert!(r.fault.is_none(), "{game} faulted at step {step}: {:?}", r.fault);
            app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
        }
        assert_eq!(
            got,
            want.to_vec(),
            "{game}: the hybrid render path per frame changed. The SQ-0729 enclosure arm of \
             `picture_takeover` is meant to move fmvpoker alone — a title that stops taking the \
             ring renders as a pixel image instead of crisp terminal cells."
        );
    }
}

/// SQ-0739: the frame must not disappear while the player types a bet.
///
/// Choosing "CHANGE CURRENT BET" hands the read to the bottom panel, and that alone
/// re-labels the screen: the panel becomes the primary `Buffer` and window 0 — with
/// the 640x400 poker table still drawn into it — becomes an ordinary window. The
/// game redrew nothing; the story window moved out from under its own art.
///
/// What that does to hybrid is the defect. The ring's bands are cropped from the
/// CHROME canvas, and `classify_windows` deliberately leaves the story window's own
/// plate out of it (`story_gfx` is blitted inside the story viewport instead). Once
/// the plate is no longer inside the window it is the plate OF, it is in neither
/// place: the premises below measure exactly that — a ring canvas with not one
/// opaque pixel, and art painting outside a story viewport that could never show it.
///
/// So the frame must keep taking the composite, and the assertion is the player's
/// own test: the screen above the bottom panel is byte-identical to the screen they
/// were looking at when they chose the option.
///
/// SQ-0746 REWROTE THE PREMISE, and the assertion is why it is still here. The story
/// window no longer moves at all: reading through a panel the game has declared is
/// NOT its transcript (attribute 2 clear, while window 0 sets it) never made that
/// panel the story window, and publishing it as one is what emptied the screen. So
/// the plate no longer escapes anything and `picture_takeover`'s SQ-0739 arm no
/// longer has to fire for this title — the enclosure arm keeps the frame here
/// exactly as it does at the menu. What the player sees is unchanged, which is what
/// these pixels check.
fn fmvpoker_bet_entry_keeps_its_frame(honor: bool) {
    let Some((mut session, mut state)) = fmvpoker_title(honor) else { return };
    // The frame as the player last saw it, at the menu.
    let menu_shot = {
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        app::render::screen::build_v6_raster_canvas(&layout, native, &state).0
    };
    assert_eq!(render_path(&session, &state), "raster", "premise (honor={honor}): the menu screen is a composite");

    // "c" — CHANGE CURRENT BET.
    let r = session.submit_char(b'c');
    assert!(r.fault.is_none(), "fmvpoker faulted choosing CHANGE CURRENT BET: {:?}", r.fault);
    app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
    *state.v6_paint.borrow_mut() = Engine::paint_surface(&session);

    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);

    // Premise: the story window did not move (SQ-0746) — it is window 0, the window
    // that declares itself the transcript, exactly as at the menu…
    let story = layout.story.expect("window 0 is still the primary Buffer");
    assert_eq!(
        (story.x_px, story.y_px, story.w_px, story.h_px),
        (0, 0, 640, 400),
        "premise (honor={honor}): the game reads the bet through a panel whose attribute 2 is \
         CLEAR, so that panel is a display panel and window 0 stays the story window"
    );
    // …and the table is exactly where it always was, on window 0.
    let plate = layout.story_gfx.expect("window 0 still holds the poker table");
    assert_eq!(
        (plate.x_px, plate.y_px, plate.w_px, plate.h_px),
        (0, 0, 640, 400),
        "premise (honor={honor}): the game redrew nothing — the plate has not moved"
    );
    // …so this frame is carried by an arm that asks about the PLATE, not about a
    // plate that escaped. SQ-0897 retired the escape arm outright: SQ-0746 left it
    // nothing to catch (the assertion here used to be `!story_plate_escapes_story_window`)
    // and SQ-0896 left it nothing to do, since the ring's canvas and art oracle now
    // carry window 0's plate at its own native origin wherever that is. Naming the arm
    // rather than negating a retired one means a future retirement that moves this
    // frame fails HERE, with the arm that moved it in the message.
    assert_eq!(
        app::render::screen::picture_takeover_reason(story, &layout.chrome, Some(plate), native),
        Some("art_paints_anything"),
        "premise (honor={honor}): the bet frame takes the composite because window 0's own \
         plate paints, which is the same arm that carries the menu"
    );

    assert_eq!(
        render_path(&session, &state),
        "raster",
        "honor={honor}: hybrid has no ring to draw on a frame whose art encloses the screen, so \
         this must stay a composite — the poker frame vanished for as long as the player took to \
         type a bet (SQ-0739)"
    );

    // And what ships is the same frame they were looking at.
    let bet_shot = app::render::screen::build_v6_raster_canvas(&layout, native, &state).0;
    for y in 0..PANEL.1 as u32 {
        for x in 0..native.0 as u32 {
            assert_eq!(
                menu_shot.get_pixel(x, y),
                bet_shot.get_pixel(x, y),
                "honor={honor}: at ({x},{y}) — above the bet panel — the screen changed when the \
                 player chose CHANGE CURRENT BET. The game drew nothing there; only which window \
                 it reads through moved (SQ-0739)."
            );
        }
    }
}

/// fmvpoker's bottom panel — window 2 — as the game declares it: 594x156 at native
/// (23,236), 1-based, so (22,235) on the 0-based composite.
const PANEL: (u16, u16, u16, u16) = (22, 235, 594, 156);

#[test]
fn fmvpoker_bet_entry_keeps_its_frame_honoring_game_colours() {
    fmvpoker_bet_entry_keeps_its_frame(true);
}

#[test]
fn fmvpoker_bet_entry_keeps_its_frame_theme_only() {
    fmvpoker_bet_entry_keeps_its_frame(false);
}

/// SQ-0897's central measurement: WHICH `picture_takeover` arm carries each corpus
/// frame — the census the hatches are retired against, one at a time.
///
/// Four OR'd predicates over one frame are not four independent facts. They are
/// tried in order and the first match wins, so a hatch can be perfectly correct and
/// never decide anything, and "the gate is closed" says nothing about which arm shut
/// it. SQ-0894 disabled an arm, passed all 5553 tests, and the arm was live — the
/// corpus simply had no fixture that reached it. This is the guard against reading
/// that shape backwards: it records the arms actually observed and fails when the
/// set changes, so a retirement that silently moves a title says so.
///
/// MEASURED with `ring_scout --all --no-tap --turns 8` at a 98x37 pane and pinned
/// here over the same eight frames, both `honor_game_colours` modes (routing does
/// not read colours, and pinning both is what stops that quietly becoming untrue):
///
/// | title | arms observed over 8 frames |
/// |---|---|
/// | zork0, shogun, scopa, advent | ring only |
/// | arthur | `art_paints_anything` on his intro plate, ring on the prose screens |
/// | journey | `art_paints_anything` on its title, ring from gameplay on |
/// | mysterious01 | `art_paints_anything` on every frame |
/// | fmvpoker | ring at boot, `art_paints_anything` from the table on |
///
/// `art_fills_screen` and `art_encloses_screen` decide NOTHING on this corpus: every
/// frame whose art fills or encloses the screen has that art on window 0's own
/// plate, which the first arm answers for. That is shadowing, not death — both ask
/// about CHROME art too, which the first arm never looks at — and it is why they are
/// kept; see `picture_takeover`'s own doc for the reasoning.
fn picture_takeover_arms_across_the_corpus(honor: bool) {
    use std::collections::BTreeSet;
    let expected: &[(&str, &[&str])] = &[
        ("zork0-r393-s890714.z6", &["ring"]),
        ("arthur-r74-s890714.z6", &["ring", "art_paints_anything"]),
        ("shogun-r322-s890706.z6", &["ring"]),
        ("journey-r83-s890706.z6", &["ring", "art_paints_anything"]),
        ("advent.z6", &["ring"]),
        ("scopa.z6", &["ring"]),
        ("mysterious01.z6", &["art_paints_anything"]),
        ("fmvpoker.z6", &["ring", "art_paints_anything"]),
    ];
    for (game, want) in expected {
        let Some((mut session, mut state)) = boot(game, honor) else { continue };
        let mut seen: BTreeSet<&'static str> = BTreeSet::new();
        for step in 0..8 {
            {
                let model = session.screen();
                let WinNode::Layered(items) = &model.root else { panic!("{game}: v6 Layered root") };
                let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
                let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
                match layout.story {
                    Some(story) => {
                        seen.insert(
                            app::render::screen::picture_takeover_reason(
                                story, &layout.chrome, layout.story_gfx, native,
                            )
                            .unwrap_or("ring"),
                        );
                    }
                    // No story window is not a ring frame either way — it is the
                    // no-story exit — but it is not a takeover, so it reads as ring.
                    None => {
                        seen.insert("ring");
                    }
                }
            }
            let r = match session.pending_input() {
                InputKind::Char => session.submit_char(13),
                _ => session.submit(""),
            };
            assert!(r.fault.is_none(), "{game} faulted at step {step}: {:?}", r.fault);
            app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
            *state.v6_paint.borrow_mut() = Engine::paint_surface(&session);
        }
        let want: BTreeSet<&str> = want.iter().copied().collect();
        let got: BTreeSet<&str> = seen.iter().copied().collect();
        assert_eq!(
            got, want,
            "{game} (honor={honor}): the arms carrying its first eight frames changed. A \
             retirement that moves a title from the composite to the ring is a mode change \
             every v6 player would see, so it belongs in a quest with a fixture, not in a \
             passing gate."
        );
    }
}

#[test]
fn picture_takeover_arms_across_the_corpus_honoring_game_colours() {
    picture_takeover_arms_across_the_corpus(true);
}

#[test]
fn picture_takeover_arms_across_the_corpus_theme_only() {
    picture_takeover_arms_across_the_corpus(false);
}

/// fmvpoker's bottom menu row, as the game lays it out.
///
/// Derived, not guessed: on the frame under test the story runs
/// `@set_window(win2)` and then `@set_cursor(row=80, col=C, window=2)` for
/// C = 0, 178, 372, 454, 557 — five 1-based pixel columns in an 8px fixed-cell
/// font, i.e. character columns 0, 22, 46, 56 and 69 (`(C-1)/8`, clamped at the
/// left margin). The labels are 16, 18, 4, 7 and 4 characters, so every one of
/// them ENDS short of the next column: the gaps are the game's, and a renderer
/// that concatenates the runs is losing information the story supplied.
const MENU_LABELS: &[(usize, &str)] = &[
    (0, "PLAY CURRENT BET"),
    (22, "CHANGE CURRENT BET"),
    (46, "SAVE"),
    (56, "RESTORE"),
    (69, "QUIT"),
];
const MENU_ROW: &str =
    "PLAY CURRENT BET      CHANGE CURRENT BET      SAVE      RESTORE      QUIT";
/// …and the LINE it lands on. Those same `set_cursor` calls declare `row=80` — a
/// 1-based pixel row down a 156px window in a 16px cell, i.e. 80 px down, which is
/// text line **5** counted from zero. The row is honoured for the same reason the
/// column is (SQ-0729, second pass): fmvpoker prints its running totals into
/// WINDOW 0 at absolute y = 247 and 265, which fall in this window's first two
/// lines, so a panel that stacked its own prose from its top edge collided with
/// the game's own layout.
///
/// This constant was 4 until SQ-0738, and moving it is the fix, not a re-baseline:
/// `(80-1)/16` FLOORS to 4, which drew the labels 15 px above the row the game
/// declared and clear of the y band 316..333 its own mouse handler accepts for
/// them, so clicking a label did nothing. The declared row is quantized to the
/// nearest line now (`v6_take_declared_indent`), and 80 px is one pixel under
/// line 5's exact start.
const MENU_LINE: usize = 5;

/// SQ-0729: a v6 prose window is still a pixel surface, and fmvpoker places every
/// menu label on one row with its own `set_cursor`. `ZWindow::push_prose` kept no
/// cursor x, so the five runs butted together into
/// `PLAY CURRENT BETCHANGE CURRENT BETSAVERESTOREQUIT` — inside the VM, before any
/// render code could see the columns. It now pads the line out TO the declared
/// column (not BY it: the run has to land where the game named, not that far past
/// wherever the previous run ended), and pads the BUFFER out to the declared line
/// the same way.
fn fmvpoker_menu_labels_keep_their_columns(honor: bool) {
    let Some((session, _state)) = fmvpoker_title(honor) else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let menu = layout
        .chrome
        .iter()
        .find(|it| matches!(&it.node, WinNode::Buffer(b) if !b.primary && !b.lines.is_empty()))
        .expect("fmvpoker publishes its menu as a secondary prose window (SQ-0585)");
    let WinNode::Buffer(b) = &menu.node else { unreachable!() };

    assert_eq!(
        b.lines.get(MENU_LINE).map(String::as_str),
        Some(MENU_ROW),
        "honor={honor}: fmvpoker's five menu labels must reach the model at the columns AND the \
         line its own set_cursor named, not run together and not stacked from the window's top \
         edge (SQ-0729). Lines: {:?}",
        b.lines
    );
    let row: Vec<char> = b.lines[MENU_LINE].chars().collect();
    for &(col, label) in MENU_LABELS {
        let got: String = row.iter().skip(col).take(label.chars().count()).collect();
        assert_eq!(
            got, label,
            "honor={honor}: {label:?} must start at column {col} — the column the game's \
             set_cursor declared for it. Row: {:?}",
            b.lines[MENU_LINE]
        );
    }
    // The window is 594px / 74 characters wide, so nothing is being bought with
    // padding the game did not ask for.
    assert!(
        row.len() <= (menu.w_px / 8) as usize,
        "honor={honor}: the menu row ({} chars) must still fit the window the game declared ({} \
         columns)",
        row.len(),
        menu.w_px / 8
    );
}

#[test]
fn fmvpoker_menu_labels_keep_their_columns_honoring_game_colours() {
    fmvpoker_menu_labels_keep_their_columns(true);
}

#[test]
fn fmvpoker_menu_labels_keep_their_columns_theme_only() {
    fmvpoker_menu_labels_keep_their_columns(false);
}

/// SQ-0738, the behavioural half: a click on a menu label WHERE IT IS DRAWN must
/// make the game act.
///
/// The user reported the menu as dead — PLAY, CHANGE CURRENT BET, SAVE, RESTORE
/// and QUIT all ignored the mouse. Nothing was wrong with input: the VM received
/// every click (`@read_mouse -> y=314 x=199 buttons=0x0001`), and the click map is
/// an exact inverse of the letterbox in both render modes. The game simply
/// decided the clicks hit nothing, because we were drawing the labels 15 px above
/// the row it placed them on — `v6_take_declared_indent` floored
/// `set_cursor(row=80)` to text line 4, and a diverted prose window's lines are
/// stacked 16 px apart from its origin, so the labels landed on native rows
/// 300..315 while the game's own hit test accepts y 316..333. Clicking the label
/// did nothing; clicking the blank row beneath it worked.
///
/// So this drives the real geometry rather than a model field: build the composite
/// the renderer builds, find the native rows the label's glyphs actually INK, pick
/// the terminal cell that shows most of them, and require that cell's click — the
/// same `V6ClickMap::map_click` the app's mouse handler uses — to reach the game
/// and change the screen. A label the player can see and cannot press is the bug;
/// the assertion is that the two coincide.
fn menu_lines(session: &GameSession) -> Option<Vec<String>> {
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { return None };
    items.iter().find_map(|it| match &it.node {
        WinNode::Buffer(b) if !b.primary && !b.lines.is_empty() => Some(b.lines.clone()),
        _ => None,
    })
}

fn fmvpoker_menu_labels_are_clickable_where_drawn(honor: bool, mode: app::config::V6RenderMode) {
    for &(col, label) in MENU_LABELS {
        // A fresh game per label: each of the five takes the game somewhere else.
        let Some((mut session, mut state)) = fmvpoker_title(honor) else { return };
        state.config.v6_render = mode;
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let menu = layout
            .chrome
            .iter()
            .find(|it| matches!(&it.node, WinNode::Buffer(b) if !b.primary && !b.lines.is_empty()))
            .expect("fmvpoker publishes its menu as a secondary prose window (SQ-0585)");
        let WinNode::Buffer(mb) = &menu.node else { unreachable!() };
        assert_eq!(
            mb.lines.get(MENU_LINE).map(String::as_str),
            Some(MENU_ROW),
            "premise (honor={honor} {mode:?}): the five labels reach the model on the line and at \
             the columns the game's own set_cursor named"
        );

        // WHERE IT IS DRAWN: the pixels this label and nothing else puts on the
        // composite — the frame the renderer builds, differenced against the same
        // frame with the label blanked out of the model. Read back from pixels
        // rather than recomputed from the model, and immune to the artwork behind
        // the panel (which is why a plain ink scan will not do: the poker table
        // shows through).
        let (img, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
        let mut blanked = session.screen();
        {
            let WinNode::Layered(items) = &mut blanked.root else { panic!("v6 Layered root") };
            let b = items
                .iter_mut()
                .find_map(|it| match &mut it.node {
                    WinNode::Buffer(b) if !b.primary && !b.lines.is_empty() => Some(b),
                    _ => None,
                })
                .expect("the menu window");
            let line: String = b.lines[MENU_LINE]
                .chars()
                .enumerate()
                .map(|(i, ch)| if (col..col + label.chars().count()).contains(&i) { ' ' } else { ch })
                .collect();
            b.lines[MENU_LINE] = line;
        }
        let WinNode::Layered(bitems) = &blanked.root else { panic!("v6 Layered root") };
        let blayout = app::render::v6_layout::classify_windows(bitems, zvm::screen::V6Cell::DEFAULT);
        let (bimg, _) = app::render::screen::build_v6_raster_canvas(&blayout, native, &state);
        let (mut x0, mut x1, mut top, mut bot) = (u32::MAX, 0u32, u32::MAX, 0u32);
        for y in 0..img.height().min(bimg.height()) {
            for x in 0..img.width().min(bimg.width()) {
                if img.get_pixel(x, y) != bimg.get_pixel(x, y) {
                    (x0, x1) = (x0.min(x), x1.max(x + 1));
                    (top, bot) = (top.min(y), bot.max(y));
                }
            }
        }
        assert!(
            x0 < x1 && top <= bot,
            "honor={honor} {mode:?}: {label:?} puts nothing on the composite at all"
        );

        // The terminal cell the player sees it on: the one whose device rect covers
        // most of that ink. This is the FORWARD letterbox — native pixel n is drawn
        // at device `img_origin + n * img_size/native` — so it is independent of the
        // inverse `map_click` performs below.
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
        let map = state
            .graphics_render
            .borrow()
            .last_v6_map
            .clone()
            .expect("a v6 frame records a click map for the mouse handler");
        let best = |lo: u32, hi: u32, origin: f32, size: f32, native: u16, cell: u16, pane: u16, n: u16| {
            let dev = |v: u32| origin + v as f32 * size / native as f32;
            (0..n)
                .max_by_key(|&c| {
                    let (c0, c1) = ((c as f32 * cell as f32), ((c + 1) as f32 * cell as f32));
                    let (l, h) = (dev(lo).max(c0), dev(hi + 1).min(c1));
                    ((h - l).max(0.0) * 256.0) as i64
                })
                .unwrap_or(0)
                + pane
        };
        let cy = best(top, bot, map.img_y, map.img_h, map.canvas.1, map.cell_h, map.pane_y, area.height);
        let cx = best(x0, x1 - 1, map.img_x, map.img_w, map.canvas.0, map.cell_w, map.pane_x, area.width);

        let (gx, gy) = map.map_click(cx, cy).unwrap_or_else(|| {
            panic!("honor={honor} {mode:?}: the cell showing {label:?} ({cx},{cy}) is off the image")
        });
        // Premise: that cell really does look at the label. A click that leaves the
        // drawn glyphs cannot be expected to press them.
        assert!(
            (top..=bot).contains(&(gy.max(1) as u32 - 1)) && (x0..x1).contains(&(gx.max(1) as u32 - 1)),
            "honor={honor} {mode:?}: the cell showing {label:?} ({cx},{cy}) maps to game pixel \
             ({gx},{gy}), outside the label's own drawn ink x {x0}..{x1} y {top}..={bot}"
        );

        // DID THE GAME ACT? Its own answer, measured: a click it rejects makes it
        // reprint the identical menu and wait again, while a click it accepts on any
        // of the five erases that panel first — so the menu window's own lines are
        // the game's verdict. (A whole-model comparison is NOT: the story canvas
        // carries a version counter that every repaint bumps, hit or miss.)
        let before = menu_lines(&session);
        app::engine::Engine::set_mouse(&mut session, gy, gx);
        let r = session.submit_char(254);
        assert!(r.fault.is_none(), "honor={honor} {mode:?}: {label:?} click faulted: {:?}", r.fault);
        app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
        assert!(
            r.quit || menu_lines(&session) != before,
            "honor={honor} {mode:?}: clicking {label:?} where it is DRAWN — terminal cell \
             ({cx},{cy}), game pixel ({gx},{gy}), inside its own ink y {top}..={bot} — left the \
             game exactly as it was. The label is painted somewhere the game does not accept a \
             click for it (SQ-0738)."
        );
    }
}

#[test]
fn fmvpoker_menu_labels_are_clickable_where_drawn_hybrid_honoring_game_colours() {
    fmvpoker_menu_labels_are_clickable_where_drawn(true, app::config::V6RenderMode::Hybrid);
}

#[test]
fn fmvpoker_menu_labels_are_clickable_where_drawn_hybrid_theme_only() {
    fmvpoker_menu_labels_are_clickable_where_drawn(false, app::config::V6RenderMode::Hybrid);
}

#[test]
fn fmvpoker_menu_labels_are_clickable_where_drawn_raster_honoring_game_colours() {
    fmvpoker_menu_labels_are_clickable_where_drawn(true, app::config::V6RenderMode::Raster);
}

#[test]
fn fmvpoker_menu_labels_are_clickable_where_drawn_raster_theme_only() {
    fmvpoker_menu_labels_are_clickable_where_drawn(false, app::config::V6RenderMode::Raster);
}

/// The composite carries a secondary prose window's lines. Asserted as ink on the
/// hint line's row: a row that reached the screen shows more than one colour.
fn fmvpoker_composite_shows_its_menu_window(honor: bool) {
    let Some((session, state)) = fmvpoker_title(honor) else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);

    // Premise: the menu is a NON-PRIMARY Buffer holding those lines…
    let menu = layout
        .chrome
        .iter()
        .find(|it| matches!(&it.node, WinNode::Buffer(b) if !b.primary && !b.lines.is_empty()))
        .expect("fmvpoker publishes its menu as a secondary prose window (SQ-0585)");
    let WinNode::Buffer(b) = &menu.node else { unreachable!() };
    let hint = b
        .lines
        .iter()
        .position(|l| l.contains("Select an option"))
        .unwrap_or_else(|| panic!("premise (honor={honor}): the hint line is in that window: {:?}", b.lines));
    // …and its labels arrive at the columns and line the game printed them at.
    assert_eq!(
        b.lines.get(MENU_LINE).map(String::as_str),
        Some(MENU_ROW),
        "premise (honor={honor}): the menu labels keep their declared columns and line"
    );

    let (img, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    // The hint line's row in native pixels, one 16px text line per buffer line.
    let (hx, hy) = (menu.x_px as u32 + menu.left_margin as u32, menu.y_px as u32 + hint as u32 * 16);
    let mut seen = std::collections::HashSet::new();
    for y in hy..(hy + 16).min(img.height()) {
        for x in hx..(hx + 63 * 8).min(img.width()) {
            seen.insert(img.get_pixel(x, y).0);
        }
    }
    assert!(
        seen.len() >= 2,
        "honor={honor}: the hint line's row is one flat colour ({} seen) — the composite draws \
         Graphics and Grid windows but not a secondary prose Buffer, so fmvpoker's menu bar and \
         its \"Select an option…\" hint never reach the screen (SQ-0729)",
        seen.len()
    );
}

#[test]
fn fmvpoker_composite_shows_its_menu_window_honoring_game_colours() {
    fmvpoker_composite_shows_its_menu_window(true);
}

#[test]
fn fmvpoker_composite_shows_its_menu_window_theme_only() {
    fmvpoker_composite_shows_its_menu_window(false);
}

/// The "Double Fanucci" banner, and what actually happens to it (SQ-0729).
///
/// The frame art is Zork Zero's — fmvpoker ships that picture file renamed to
/// FMVPOKER.EG1 — so its top-centre tab natively reads "Double Fanucci", a title
/// belonging to a different game. The reading under investigation was that fmvpoker
/// overprints it with a title of its own and we fail to draw that text. It does not:
/// traced from the game itself, the boot frame parks WINDOW 1 at (173,7) 289x34,
/// exactly over the banner, `erase_window`s it to the blue it declared for that
/// window, and never prints a single character into it for the rest of the session.
/// The banner is not overwritten, it is ERASED — which is how a game hides a title
/// that is not its own, and there is no fmvpoker title to draw.
///
/// What WAS wrong is the colour of the hole. The erase reached the host correctly as
/// a painted-ground fill (SQ-0706), but `fill_story_page_under_chrome_text` then
/// flooded window 0's whole box on top of it — window 0 is the entire 640x400 screen
/// here — so the tab rendered as a white gash across the top of an otherwise
/// complete blue frame. That is the "the top-centre tab is cut off at y=0" this quest
/// carried for three passes as a clipping question; the art is not clipped at all.
/// The page is now the OLDEST thing in the box, sparing the game's own fills exactly
/// as it already spares chrome text.
fn fmvpoker_erased_banner_keeps_the_colour_the_game_named(honor: bool) {
    let Some((session, state)) = fmvpoker_title(honor) else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);

    // Premise: window 1 is parked over the banner and the game printed NOTHING into
    // it. If a later fmvpoker really does put its own title there, this is the
    // assertion that notices — and the theory this test refutes becomes true.
    let tab = layout
        .chrome
        .iter()
        .find(|it| (it.x_px, it.y_px, it.w_px, it.h_px) == (172, 6, 289, 34))
        .expect("premise: fmvpoker parks window 1 at native (173,7) 289x34, over the banner");
    let WinNode::Grid(g) = &tab.node else { panic!("window 1 is published as a Grid") };
    assert!(
        g.px_texts.is_empty() && g.cells.iter().all(|c| c.ch == '\0' || c.ch == ' '),
        "premise (honor={honor}): fmvpoker prints nothing into the window it parks over the \
         \"Double Fanucci\" banner — it erases the title rather than overwriting it"
    );

    // Premise: the erase cut the banner out of the ARTWORK — the plate is a hole
    // there — and reached us as painted ground in the colour window 1 declared. So
    // what shows at those pixels comes from under the art, and is the game's fill or
    // nothing.
    let plate = layout.story_gfx.expect("fmvpoker draws its table into window 0 (SQ-0714)");
    let WinNode::Graphics(gfx) = &plate.node else { panic!("story_gfx is a Graphics leaf") };
    let paint = Engine::paint_surface(&session).expect("premise: the erase_window fill is recorded");
    let ground = *paint.get_pixel(300, 20);
    assert_ne!(ground[3], 0, "premise (honor={honor}): the banner's pixels were filled by the game");

    let (img, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    for (x, y) in [(300u32, 20u32), (180, 10), (450, 36)] {
        assert_eq!(
            gfx.canvas.get_pixel(x, y).0[3],
            0,
            "premise (honor={honor}): at ({x},{y}) the erase cut the \"Double Fanucci\" banner out \
             of the plate, so nothing of the artwork remains to be clipped"
        );
        assert_eq!(
            *paint.get_pixel(x, y),
            ground,
            "premise (honor={honor}): ({x},{y}) is inside the rectangle the game filled"
        );
        assert_eq!(
            *img.get_pixel(x, y),
            ground,
            "honor={honor}: at ({x},{y}) the banner must keep the colour fmvpoker's own \
             erase_window painted there, not window 0's page. The story page is the oldest thing \
             in its box; a fill the game issued afterwards is newer, and flooding over it renders \
             the frame's top-centre tab as a white gash (SQ-0729)."
        );
    }
}

#[test]
fn fmvpoker_erased_banner_keeps_the_colour_the_game_named_honoring_game_colours() {
    fmvpoker_erased_banner_keeps_the_colour_the_game_named(true);
}

#[test]
fn fmvpoker_erased_banner_keeps_the_colour_the_game_named_theme_only() {
    fmvpoker_erased_banner_keeps_the_colour_the_game_named(false);
}

/// Play a hand: past the boot screen, `p` to deal, then a key per card until the
/// game announces the draw. Stops on the frame where its bottom prose window is
/// carrying "You draw (a) …" — the line the player reads to decide what to hold.
fn fmvpoker_dealt_hand(honor: bool) -> Option<(GameSession, app::state::AppState)> {
    let (mut session, mut state) = fmvpoker_title(honor)?;
    for step in 0..24 {
        let r = session.submit_char(if step == 0 { b'p' } else { b' ' });
        assert!(r.fault.is_none(), "fmvpoker faulted dealing: {:?}", r.fault);
        app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
        *state.v6_paint.borrow_mut() = Engine::paint_surface(&session);
        let has_draw = {
            let model = session.screen();
            let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
            app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT).chrome.iter().any(|it| {
                matches!(&it.node, WinNode::Buffer(b) if b.lines.iter().any(|l| l.starts_with("You draw")))
            })
        };
        if has_draw {
            return Some((session, state));
        }
    }
    panic!("fmvpoker never announced a draw within 24 keypresses");
}

/// SQ-0729: the story transcript wrote its boot banner straight through
/// "You draw (a) …", the line the player needs in order to see their draw.
///
/// Nothing about the model is wrong. The announcement is window 2's prose and
/// window 2 is already the bottom window, exactly where it belongs. What moves is
/// window 0: it is the whole 640x400 screen here, and once the five dealt cards
/// fill the frame's interior its largest CLEAR rectangle drops onto the very box
/// the game gave window 2. `fill_story_page_under_chrome_text` spared those pixels
/// from the page FILL (SQ-0728) but nothing spared them from the GLYPHS, so
/// `draw_story_text` rasterized "FROBOZZ MAGIC VIDEOPOKER / by Zach Matley / …"
/// over the announcement.
///
/// The rule the fix states: a story transcript is the HOST's re-render of
/// everything window 0 ever printed, while a label another window is holding is on
/// the screen *now* — so where they collide the live label wins.
///
/// Asserted by difference, which is what makes it falsifiable: the rows window 2's
/// own lines occupy must be byte-identical to the same frame rendered with an
/// EMPTY host transcript, since with no transcript there is nothing to overdraw
/// with. The rows below must NOT be identical — the money lines still belong to
/// the transcript and must still reach the screen — so the test cannot pass by
/// suppressing the story text wholesale.
fn fmvpoker_the_draw_announcement_survives_the_transcript(honor: bool) {
    let Some((session, mut state)) = fmvpoker_dealt_hand(honor) else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);

    // Premise: the announcement is in the bottom prose window, not the transcript…
    let panel = layout
        .chrome
        .iter()
        .find(|it| matches!(&it.node, WinNode::Buffer(b) if b.lines.iter().any(|l| l.starts_with("You draw"))))
        .expect("fmvpoker announces the draw in its bottom prose window");
    let WinNode::Buffer(b) = &panel.node else { unreachable!() };
    let lines = b.lines.len().min((panel.h_px / 16) as usize);
    // …and the host transcript has boot prose of its own to collide with.
    assert!(
        state.transcript.iter().any(|l| l.contains("FROBOZZ MAGIC VIDEOPOKER")),
        "premise (honor={honor}): the transcript carries the boot banner"
    );

    let (img, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    // The same frame with nothing for `draw_story_text` to draw: every other layer
    // is unchanged, so any difference in these rows IS the transcript overdrawing.
    state.transcript.clear();
    state.transcript_styles.clear();
    state.transcript_runs.clear();
    state.transcript_para.clear();
    state.transcript_images.clear();
    let (bare, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);

    let (x0, x1) = (panel.x_px as u32, (panel.x_px as u32 + panel.w_px as u32).min(img.width()));
    let row_band = |row: usize| -> (u32, u32) {
        let y = panel.y_px as u32 + row as u32 * 16;
        (y, (y + 16).min(img.height()))
    };
    for row in 0..lines {
        let (y0, y1) = row_band(row);
        for y in y0..y1 {
            for x in x0..x1 {
                assert_eq!(
                    img.get_pixel(x, y),
                    bare.get_pixel(x, y),
                    "honor={honor}: at ({x},{y}) the story transcript is drawn over row {row} of \
                     fmvpoker's own bottom window — {:?}. Once the cards fill the frame, window 0's \
                     clear rectangle lands on window 2's box, and the transcript painted the boot \
                     banner through \"You draw (a) …\" (SQ-0729).",
                    b.lines[row]
                );
            }
        }
    }
    // …and sparing those rows must not silence what window 0 prints in the rest of
    // that box: "Current Bet: / 10 / Total Winnings: / 990" belong there too. (They
    // reach the screen as PAINT rather than as transcript — see
    // `fmvpoker_paints_the_runs_it_positions` — so this looks for ink, not for a
    // difference against the bare render.)
    let money = panel.y_px as u32 + 12;
    let mut seen = std::collections::HashSet::new();
    for y in money..(money + 16).min(img.height()) {
        for x in x0..x1 {
            seen.insert(img.get_pixel(x, y).0);
        }
    }
    assert!(
        seen.len() >= 2,
        "honor={honor}: the row carrying \"Current Bet:\" and \"Total Winnings:\" is one flat \
         colour ({} seen) — sparing the rows the game's own bottom window holds must not take \
         window 0's own prose off the screen with them",
        seen.len()
    );
}

#[test]
fn fmvpoker_the_draw_announcement_survives_the_transcript_honoring_game_colours() {
    fmvpoker_the_draw_announcement_survives_the_transcript(true);
}

#[test]
fn fmvpoker_the_draw_announcement_survives_the_transcript_theme_only() {
    fmvpoker_the_draw_announcement_survives_the_transcript(false);
}

/// SQ-0729, rule (d): **a story window whose own art ENCLOSES it is a canvas.**
///
/// fmvpoker positions essentially everything it shows. It prints "HOLD" under each
/// card it is holding at `set_cursor(row=203, col=70/183/296/409/522, window=0)`
/// and its running totals at (76,247), (76,265), (420,247) and (420,265) — and
/// every one of those landed in the story SCROLL instead, because window 0 is the
/// host transcript. The player's own reading of it was the right one: there is not
/// supposed to be a scroll window in this game at all.
///
/// What makes it decidable is refusing to ask what a RUN means. A `set_cursor`
/// before a run is genuinely ambiguous — Arthur positions every room headline in
/// window 0 one character at a time, Shogun and Journey centre each header line,
/// mysterious01 re-homes before its prompt, and all of them mean "resume the
/// transcript here" with the identical signal fmvpoker uses for "paint this under
/// that card". Measuring that rule broke Arthur's room names ("CHURCH" came out as
/// "C" painted and "HURCH" streamed). So the question asked instead is what kind of
/// SURFACE the window is: Arthur's window 0 is a transcript that happens to have
/// plates drawn on it, and fmvpoker's is a picture frame that happens to have text
/// positioned in it. Its own art encloses it on all four sides and it covers the
/// whole screen.
///
/// The window then renders as what is SITTING on it — zvm's streamed shadow, which
/// an `erase_window` empties exactly as it empties any painted layer — at the
/// coordinates the game named, and the frame carries no transcript at all.
///
/// Asserted the way that makes it unambiguous: the composite must be BYTE-IDENTICAL
/// with and without a host transcript. A page depends on its transcript; a canvas
/// does not.
fn fmvpoker_paints_the_runs_it_positions(honor: bool) {
    let Some((mut session, mut state)) = fmvpoker_dealt_hand(honor) else { return };
    // …then hold card (a), which is when the game paints its HOLD label.
    let r = session.submit_char(b'a');
    assert!(r.fault.is_none(), "fmvpoker faulted holding a card: {:?}", r.fault);
    app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
    *state.v6_paint.borrow_mut() = Engine::paint_surface(&session);
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);

    // Premise: this frame's story window is a canvas…
    assert!(
        app::render::screen::story_window_is_a_canvas(&layout, native),
        "premise (honor={honor}): fmvpoker's window 0 covers the screen and its own table art \
         encloses it"
    );
    // …and the runs the game positioned are on it, at the coordinates it named.
    let story = layout.story.expect("fmvpoker publishes a story window");
    let WinNode::Buffer(b) = &story.node else { panic!("window 0 is the primary prose Buffer") };
    let at = |x: u16, y: u16| b.px_runs.iter().find(|t| (t.x, t.y) == (x, y)).map(|t| t.text.clone());
    assert_eq!(
        at(70, 203).as_deref(),
        Some("HOLD"),
        "premise (honor={honor}): the HOLD label the game printed at set_cursor(row=203, col=70) \
         is recorded where it was printed. Runs: {:?}",
        b.px_runs.iter().map(|t| (t.x, t.y, t.text.clone())).collect::<Vec<_>>()
    );
    assert_eq!(
        at(76, 247).as_deref(),
        Some("Current Bet:"),
        "premise (honor={honor}): so is the running total, at set_cursor(row=247, col=76)"
    );
    // And they are a SCREEN, not a log: no two of them claim the same pixels. The
    // running total is reprinted every hand into a window the game erases first —
    // 1000 becomes 990 — and a shadow that kept the erased figure would leave "990"
    // standing on the tail of "1000", reading "9900". An erase covers pixels, so it
    // trims this layer exactly as it trims any other painted one.
    let rect = |t: &app::engine::PxText| {
        let (x, y) = (t.x.max(1) as u32 - 1, t.y.max(1) as u32 - 1);
        (x, y, x + t.text.chars().count() as u32 * 8, y + 16)
    };
    for (i, a) in b.px_runs.iter().enumerate() {
        for c in &b.px_runs[i + 1..] {
            let (ax0, ay0, ax1, ay1) = rect(a);
            let (cx0, cy0, cx1, cy1) = rect(c);
            assert!(
                ax0 >= cx1 || cx0 >= ax1 || ay0 >= cy1 || cy0 >= ay1,
                "honor={honor}: {:?} at ({},{}) and {:?} at ({},{}) are both recorded on the same \
                 pixels — the older one was erased off the screen and its record has to go with \
                 it (SQ-0729)",
                a.text,
                a.x,
                a.y,
                c.text,
                c.x,
                c.y
            );
        }
    }
    // Premise: the host transcript is not empty, so "identical without it" means
    // something.
    assert!(
        state.transcript.iter().any(|l| l.contains("FROBOZZ MAGIC VIDEOPOKER")),
        "premise (honor={honor}): the transcript carries the boot banner"
    );

    let (img, metrics) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    // The HOLD label is really on the screen: its cells are not one flat colour.
    let mut seen = std::collections::HashSet::new();
    for y in 202..218u32 {
        for x in 69..(69 + 4 * 8) {
            seen.insert(img.get_pixel(x, y).0);
        }
    }
    assert!(
        seen.len() >= 2,
        "honor={honor}: the cells under card (a) at (70,203) are one flat colour ({} seen) — the \
         HOLD label the game positioned there is not being painted (SQ-0729)",
        seen.len()
    );
    assert!(
        metrics.is_none(),
        "honor={honor}: a canvas has no scroll — reporting scroll metrics for it puts a pager on \
         a frame with no transcript to page through"
    );

    state.transcript.clear();
    state.transcript_styles.clear();
    state.transcript_runs.clear();
    state.transcript_para.clear();
    state.transcript_images.clear();
    let (bare, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    let differing = img.pixels().zip(bare.pixels()).filter(|(a, b)| a != b).count();
    assert_eq!(
        differing, 0,
        "honor={honor}: {differing} pixels of this frame come from the host transcript. A story \
         window the game has drawn a frame AROUND is a canvas, and what it shows is the runs \
         sitting on it — not a re-render of everything window 0 has ever printed (SQ-0729)"
    );

    // …and the game's two layers do not collide. Painting window 0's runs at the
    // coordinates it named only reads as a fix if the OTHER window's prose is at
    // the coordinates IT named too: fmvpoker prints its bottom panel's lines at
    // `set_cursor(row=80, …)`, five text lines down that panel, while these runs
    // land in its first two lines. Stacking the panel's prose from its top edge
    // instead put "Current Bet: / 10 / Total Winnings: / 990" straight through
    // "You draw (a) an Eight, …" — the same symptom, arrived at from the other side.
    let panel = layout
        .chrome
        .iter()
        .find(|it| matches!(&it.node, WinNode::Buffer(b) if b.lines.iter().any(|l| l.starts_with("You draw"))))
        .expect("fmvpoker announces the draw in its bottom prose window");
    let WinNode::Buffer(pb) = &panel.node else { unreachable!() };
    for (row, line) in pb.lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let (ly0, ly1) = (panel.y_px as u32 + row as u32 * 16, panel.y_px as u32 + row as u32 * 16 + 16);
        let (lx0, lx1) = (
            panel.x_px as u32 + panel.left_margin as u32,
            panel.x_px as u32 + panel.w_px as u32,
        );
        for t in &b.px_runs {
            if t.text.trim().is_empty() {
                continue;
            }
            let (ry0, ry1) = (t.y.max(1) as u32 - 1, t.y.max(1) as u32 - 1 + 16);
            let (rx0, rx1) = (t.x.max(1) as u32 - 1, t.x.max(1) as u32 - 1 + t.text.chars().count() as u32 * 8);
            assert!(
                ry0 >= ly1 || ly0 >= ry1 || rx0 >= lx1 || lx0 >= rx1,
                "honor={honor}: window 0's {:?} at ({},{}) lands on row {row} of the bottom \
                 panel, {line:?}. Both are the game's own text and it placed neither of them \
                 there by accident (SQ-0729).",
                t.text,
                t.x,
                t.y
            );
        }
    }
}

#[test]
fn fmvpoker_paints_the_runs_it_positions_honoring_game_colours() {
    fmvpoker_paints_the_runs_it_positions(true);
}

#[test]
fn fmvpoker_paints_the_runs_it_positions_theme_only() {
    fmvpoker_paints_the_runs_it_positions(false);
}

/// The corpus guard on rule (d), and the reason it was worth trying at all: the
/// enclosure test was measured when it was written and fires for ONE title.
///
/// Zork Zero and Shogun keep window 0 inset inside their frames; advent and scopa
/// paint nothing behind it; Arthur's intro plate and Journey's title are solid and
/// answer the FILL arm rather than the enclosure one; mysterious01's plate is a
/// band across the lower half that reaches neither the top edge nor the right one.
/// Every one of them keeps its transcript, which is the whole point — window 0 is
/// the story window of every v6 game.
#[test]
fn fmvpoker_is_the_only_canvas_story_window() {
    let expected: &[(&str, [bool; 4])] = &[
        ("zork0-r393-s890714.z6", [false; 4]),
        ("arthur-r74-s890714.z6", [false; 4]),
        ("shogun-r322-s890706.z6", [false; 4]),
        ("journey-r83-s890706.z6", [false; 4]),
        ("advent.z6", [false; 4]),
        ("scopa.z6", [false; 4]),
        ("mysterious01.z6", [false; 4]),
        // …and fmvpoker, which boots with no art at all and is an ordinary page for
        // that one frame, then draws its table and is a canvas from there.
        ("fmvpoker.z6", [false, true, true, true]),
    ];
    for &(game, want) in expected {
        let Some((mut session, mut state)) = boot(game, true) else { continue };
        let mut got = [false; 4];
        for (step, slot) in got.iter_mut().enumerate() {
            {
                let model = session.screen();
                let WinNode::Layered(items) = &model.root else { panic!("{game}: v6 Layered root") };
                let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
                let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
                *slot = app::render::screen::story_window_is_a_canvas(&layout, native);
            }
            if step == 3 {
                break;
            }
            let r = match session.pending_input() {
                InputKind::Char => session.submit_char(13),
                _ => session.submit(""),
            };
            assert!(r.fault.is_none(), "{game} faulted at step {step}: {:?}", r.fault);
            app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
            *state.v6_paint.borrow_mut() = Engine::paint_surface(&session);
        }
        assert_eq!(
            got, want,
            "{game}: which frames read as a CANVAS changed. Rule (d) turns off a v6 title's whole \
             transcript, so a game that starts qualifying loses its narration (SQ-0729)."
        );
    }
}

// ---------------------------------------------------------------------------
// SQ-0746: the bet screen's panel was BLANK — no prompt, no money lines, and no
// echo of the digits the player typed (though the bet still applied on Enter, so
// only the display was lost). The same on QUIT, which is the discriminator: the
// trigger is window 2 becoming the window the game reads through, not anything
// about the bet screen's content.
//
// One cause under all three. zvm diverts a flowing-prose window's text to the
// window's own buffer unless the game declared it the transcript (ZMSD §8.8.3.1
// attribute 2) — with the window the game READS through as a fallback for a game
// that declares nothing. fmvpoker declares: window 0 sets attribute 2, its bottom
// panel does not. Letting the read re-designate the panel anyway split one screen
// across two sinks:
//
//   * the PROMPT was printed into the panel's buffer (before the read), and the
//     panel was then published as the primary `Buffer`, whose `lines` are empty by
//     construction — a primary window's prose is the host transcript;
//   * the MONEY LINES were window 0's streamed shadow, published only while window
//     0 is the primary Buffer, and it had just stopped being one;
//   * the ECHO went with the story window. The host draws its live input line in
//     the story window, and the story window was now a panel with no text box:
//     `story_clear_native` measures the story box against the game's own painted
//     ground, and fmvpoker had just `erase_window`ed that very panel — 92664 of
//     92664 pixels — so the box insetted to height 0 and `build_v6_raster_canvas`
//     took its "the picture owns the screen" branch and drew no story text at all.
//
// The rule now: the game's own attribute 2 decides which window is the transcript,
// and reading through a panel does not re-designate it. Window 0 stays the story
// window (and its px_runs with it), the panel stays a secondary prose window with
// its prompt in it — and the host's input line follows the READ into that panel,
// which is where the player is typing.

/// Reach a prompt fmvpoker reads through its bottom panel: `key` from the menu.
fn fmvpoker_panel_prompt(honor: bool, key: u8) -> Option<(GameSession, app::state::AppState)> {
    let (mut session, mut state) = fmvpoker_title(honor)?;
    let r = session.submit_char(key);
    assert!(r.fault.is_none(), "fmvpoker faulted on {:?}: {:?}", key as char, r.fault);
    app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
    *state.v6_paint.borrow_mut() = Engine::paint_surface(&session);
    Some((session, state))
}

/// The composite for this frame, and the same composite with `edit` applied to the
/// model first — the difference is exactly what `edit` was responsible for drawing.
/// Read back from pixels rather than recomputed, and immune to the poker table
/// showing through behind the panel (which is why a plain ink scan will not do).
fn shot_and_shot_without(
    session: &GameSession,
    state: &app::state::AppState,
    edit: impl Fn(&mut app::engine::ScreenModel),
) -> (image::RgbaImage, image::RgbaImage) {
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let (img, _) = app::render::screen::build_v6_raster_canvas(&layout, native, state);
    let mut stripped = session.screen();
    edit(&mut stripped);
    let WinNode::Layered(sitems) = &stripped.root else { panic!("v6 Layered root") };
    let slayout = app::render::v6_layout::classify_windows(sitems, zvm::screen::V6Cell::DEFAULT);
    let (bare, _) = app::render::screen::build_v6_raster_canvas(&slayout, native, state);
    (img, bare)
}

/// The bounding box of the pixels two composites disagree on; `None` when identical.
fn diff_box(a: &image::RgbaImage, b: &image::RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in 0..a.height().min(b.height()) {
        for x in 0..a.width().min(b.width()) {
            if a.get_pixel(x, y) != b.get_pixel(x, y) {
                (x0, y0) = (x0.min(x), y0.min(y));
                (x1, y1) = (x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    (x0 < x1).then_some((x0, y0, x1, y1))
}

/// The bottom panel's lines, as the model publishes them.
///
/// Found by its BOX, not by its classification, so that a frame which publishes the
/// panel as the primary `Buffer` is reported by what the player sees — no lines —
/// rather than by the panel going missing. A primary buffer's `lines` are empty by
/// construction (the host transcript is its prose), so republishing the panel as one
/// discards every line the game printed into it: that is the blank panel of SQ-0746.
fn panel_lines(session: &GameSession) -> Vec<String> {
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let panel = items
        .iter()
        .find(|it| (it.x_px, it.y_px, it.w_px, it.h_px) == PANEL)
        .expect("fmvpoker publishes its bottom panel as a window of its own");
    let WinNode::Buffer(b) = &panel.node else { panic!("the panel is a prose Buffer") };
    b.lines.clone()
}

/// SQ-0746: the prompt the game prints into the panel it then reads through must
/// reach the model AND the screen. Both of fmvpoker's are checked, because the QUIT
/// one is what proved the trigger is the read and not the bet screen.
fn fmvpoker_a_panel_prompt_reaches_the_screen(honor: bool, key: u8, prompt: &str) {
    let Some((session, state)) = fmvpoker_panel_prompt(honor, key) else { return };
    let lines = panel_lines(&session);
    let row = lines
        .iter()
        .position(|l| l.starts_with(prompt))
        .unwrap_or_else(|| panic!("honor={honor}: {prompt:?} never reached the model. Panel lines: {lines:?}"));

    // …and it is DRAWN: the composite differs from the same frame with that line
    // blanked out of the model, on the line's own row.
    let (img, bare) = shot_and_shot_without(&session, &state, |m| {
        let WinNode::Layered(items) = &mut m.root else { panic!("v6 Layered root") };
        for it in items.iter_mut() {
            if let WinNode::Buffer(b) = &mut it.node {
                if !b.primary && b.lines.len() > row {
                    b.lines[row] = String::new();
                }
            }
        }
    });
    let (_, y0, _, y1) = diff_box(&img, &bare).unwrap_or_else(|| {
        panic!(
            "honor={honor}: {prompt:?} puts nothing on the composite. The panel is blank while the \
             player answers it — no prompt, no money lines, no echo (SQ-0746)."
        )
    });
    let top = PANEL.1 as u32 + row as u32 * 16;
    assert!(
        (top..top + 16).contains(&y0) && y1 <= top + 16,
        "honor={honor}: {prompt:?} is drawn at rows {y0}..{y1}, not on its own row {top}..{}",
        top + 16
    );
}

#[test]
fn fmvpoker_the_bet_prompt_reaches_the_screen_honoring_game_colours() {
    fmvpoker_a_panel_prompt_reaches_the_screen(true, b'c', "Enter the new bet:");
}

#[test]
fn fmvpoker_the_bet_prompt_reaches_the_screen_theme_only() {
    fmvpoker_a_panel_prompt_reaches_the_screen(false, b'c', "Enter the new bet:");
}

#[test]
fn fmvpoker_the_quit_prompt_reaches_the_screen_honoring_game_colours() {
    fmvpoker_a_panel_prompt_reaches_the_screen(true, b'q', "Are you sure you want to quit?");
}

#[test]
fn fmvpoker_the_quit_prompt_reaches_the_screen_theme_only() {
    fmvpoker_a_panel_prompt_reaches_the_screen(false, b'q', "Are you sure you want to quit?");
}

/// SQ-0746: the money lines are window 0's, and window 0 does not stop being the
/// story window because the game reads a bet through a panel.
///
/// "Current Bet: / 10 / Total Winnings: / 1000" are printed into window 0 at
/// (76,247), (76,265), (420,247) and (420,265) and reach the host as that window's
/// streamed shadow (`px_runs`, SQ-0729) — published only while window 0 is the
/// primary `Buffer`. They vanished off the bet screen without the game touching
/// them.
fn fmvpoker_the_money_lines_survive_a_panel_prompt(honor: bool) {
    let Some((session, state)) = fmvpoker_panel_prompt(honor, b'c') else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let story = layout.story.expect("window 0 is still the primary Buffer");
    let WinNode::Buffer(b) = &story.node else { panic!("the story window is a prose Buffer") };
    let at = |x: u16, y: u16| b.px_runs.iter().find(|t| (t.x, t.y) == (x, y)).map(|t| t.text.clone());
    for (x, y, text) in [(76u16, 247u16, "Current Bet:"), (420, 247, "Total Winnings:")] {
        assert_eq!(
            at(x, y).as_deref(),
            Some(text),
            "honor={honor}: {text:?} is still on window 0's glass at ({x},{y}) — the game printed \
             it and has not erased it. Runs: {:?}",
            b.px_runs.iter().map(|t| (t.x, t.y, t.text.clone())).collect::<Vec<_>>()
        );
    }

    // …and they are drawn: the composite differs from the same frame with the runs
    // taken out of the model, inside the rows the game printed them on.
    let (img, bare) = shot_and_shot_without(&session, &state, |m| {
        let WinNode::Layered(items) = &mut m.root else { panic!("v6 Layered root") };
        for it in items.iter_mut() {
            if let WinNode::Buffer(b) = &mut it.node {
                if b.primary {
                    b.px_runs.clear();
                }
            }
        }
    });
    let (_, y0, _, y1) = diff_box(&img, &bare).unwrap_or_else(|| {
        panic!("honor={honor}: window 0's running totals put nothing on the bet screen (SQ-0746)")
    });
    assert!(
        (246..=280).contains(&y0) && y1 <= 282,
        "honor={honor}: the money lines drew at rows {y0}..{y1}, not the 247/265 rows the game \
         named for them"
    );
}

#[test]
fn fmvpoker_the_money_lines_survive_a_panel_prompt_honoring_game_colours() {
    fmvpoker_the_money_lines_survive_a_panel_prompt(true);
}

#[test]
fn fmvpoker_the_money_lines_survive_a_panel_prompt_theme_only() {
    fmvpoker_the_money_lines_survive_a_panel_prompt(false);
}

/// SQ-0746: what the player types has to appear where they are typing it.
///
/// The digits are HOST-side — the app holds the in-progress line and the game never
/// sees it until Enter — so they must reach the screen whatever the game's own
/// windows carry. They reached nothing: the host draws its live input line in the
/// STORY window, the panel had been published as the story window, and that panel's
/// text box measured to nothing because the game had just erased it (a fill covers
/// every pixel, so `story_clear_native` insetted the box away to height 0). Typing a
/// bet and pressing Enter still applied it, which is what made the loss purely
/// visual — and unnerving.
///
/// The echo follows the READ now: the window the game is reading through carries it,
/// continuing that window's own prompt line.
fn fmvpoker_echoes_the_digits_the_player_types(honor: bool) {
    let Some((session, mut state)) = fmvpoker_panel_prompt(honor, b'c') else { return };
    let lines = panel_lines(&session);
    let row = lines.iter().position(|l| l.starts_with("Enter the new bet:")).expect("the bet prompt");
    let prompt_cols = lines[row].chars().count() as u32;

    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let (quiet, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    state.input.value = "25".into();
    let (typed, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);

    let (x0, y0, x1, y1) = diff_box(&quiet, &typed).unwrap_or_else(|| {
        panic!(
            "honor={honor}: typing \"25\" at the bet prompt changed NOTHING on the screen. The \
             digits are the host's own input line, and with them invisible the player has no \
             feedback at all that the game is listening (SQ-0746)."
        )
    });
    let top = PANEL.1 as u32 + row as u32 * 16;
    let after_prompt = PANEL.0 as u32 + prompt_cols * 8;
    assert!(
        (top..top + 16).contains(&y0) && y1 <= top + 16,
        "honor={honor}: the echo drew at rows {y0}..{y1}, not on the prompt's own row {top}..{}",
        top + 16
    );
    assert!(
        x0 >= after_prompt,
        "honor={honor}: the echo drew from x={x0}, over the prompt itself — it continues {:?}, \
         which ends at x={after_prompt}",
        lines[row]
    );
    assert!(
        x1 <= after_prompt + 3 * 8,
        "honor={honor}: the echo drew out to x={x1}; two digits and a caret reach {}",
        after_prompt + 3 * 8
    );
}

#[test]
fn fmvpoker_echoes_the_digits_the_player_types_honoring_game_colours() {
    fmvpoker_echoes_the_digits_the_player_types(true);
}

#[test]
fn fmvpoker_echoes_the_digits_the_player_types_theme_only() {
    fmvpoker_echoes_the_digits_the_player_types(false);
}

/// The corpus guard on SQ-0746's rule: which window is the transcript's must not
/// move for anybody else.
///
/// It is the guard `fmvpoker_is_the_only_title_this_moves` puts on the render path,
/// one level down — at the model, where a title that lost its story window would
/// lose its narration whatever the path did. Every corpus title reads input through
/// a window that DECLARES attribute 2 (advent through its window 7, which sets it;
/// everyone else through window 0), so the fallback this quest made conditional was
/// never what any of them relied on.
#[test]
fn no_corpus_title_reads_through_a_panel() {
    for game in [
        "zork0-r393-s890714.z6",
        "arthur-r74-s890714.z6",
        "shogun-r322-s890706.z6",
        "journey-r83-s890706.z6",
        "advent.z6",
        "scopa.z6",
        "mysterious01.z6",
        "fmvpoker.z6",
    ] {
        let Some((mut session, mut state)) = boot(game, true) else { continue };
        for step in 0..8 {
            let input = session.machine.screen.v6_input_window as usize;
            assert!(
                !session.machine.v6_diverts_prose(input),
                "{game} frame {step}: the game is reading input through window {input}, whose \
                 prose is DIVERTED to its own buffer — so its narration is a display panel and \
                 the host transcript is some other window's (SQ-0746)"
            );
            let r = match session.pending_input() {
                InputKind::Char => session.submit_char(13),
                _ => session.submit(""),
            };
            assert!(r.fault.is_none(), "{game} faulted at step {step}: {:?}", r.fault);
            app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
            *state.v6_paint.borrow_mut() = Engine::paint_surface(&session);
        }
    }
}
