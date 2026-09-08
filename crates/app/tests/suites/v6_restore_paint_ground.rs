//! SQ-0787: a restore must replace the painted ground it found, not draw on top of it.
//!
//! The player resumed scopa and got the game's MAIN MENU under the restored hand —
//! three white buttons, one wide button, three suit cards — with the restored game's
//! own text ("abort", "9 = 6+3", "OK") drawn over the top of them. Clicking where the
//! real cards *should* be drove the right card, so nothing was wrong with the model:
//! only the picture.
//!
//! The measurement that settled it (SQ-0787's own notes) ruled out everything in
//! front of the composite. The restored frame is ONE kitty image, uploaded after the
//! restore and placed once — zero stale placements, so the placement-lifetime family
//! (SQ-0753/0763) is not involved. The raster generation key DID move and
//! `v6_wants_build` DID rebuild — so SQ-0788's gate is not involved either. The stale
//! pixels were in the INPUT to that rebuild: the PAINTED GROUND (SQ-0706), which
//! `build_v6_raster_canvas` blits under everything and which no restore path touched.
//!
//! Why it was invisible until scopa: the ground rides BESIDE the window tree, so it
//! was the one v6 screen layer the archive never carried. `auto_load` (on by default)
//! restores after the story has already booted and painted its opening screen, and a
//! host Save State swaps VM memory under a game that never learns it happened — so no
//! repaint is ever issued and the pre-restore surface simply stays.
//!
//! The fix is on the restore path, in two halves, and both are pinned below:
//!
//! - `GameSession::paint_ground_png` puts the ground in the archive
//!   (`pictures/ground.png`);
//! - `GameSession::load_paint_ground` installs it — and RESETS the ground when the
//!   archive has none, which is the half that matters for every other v6 story.
//!
//! Per CLAUDE.md these tests PERTURB before asserting: the frame immediately after a
//! restore is exactly when everything still looks correct. They restore, play a move,
//! and only then compare — against the session that was saved, which is the only
//! honest oracle for "did the restore reproduce this screen".
//!
//! The story is gitignored (CLAUDE.md), so every test here skips cleanly without it.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::GameSession;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;


fn story_path() -> PathBuf {
    fixture_path("scopa.z6")
}

fn picts() -> PictSource {
    PictSource::new(blorb::resolve_resource_blorb(&story_path()).map(|(b, _)| b))
}

/// Boot scopa to its main menu. `host_screen` is the terminal the session believes it
/// is on — varied by the different-size test, and irrelevant to a v6 story's own
/// 640x400 native screen, which is exactly the property being pinned.
fn boot_on(host_screen: Option<(u16, u16)>) -> Option<GameSession> {
    let path = story_path();
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut pic = picts();
    let dims = pic.all_pict_dims();
    let mut s = GameSession::new_with_trace(
        bytes, true, false, None, false, dims, pic.std_window(), None, host_screen,
    )
    .expect("scopa is a valid v6 story");
    s.set_pict_source(Some(pic));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    Some(s)
}

fn boot() -> Option<GameSession> {
    boot_on(None)
}

/// `Engine::set_mouse` takes the coordinates Y FIRST; a click reaches `read_char` as
/// ZSCII 254 (ZMSD §3.8) with the coordinates already set.
fn click(s: &mut GameSession, x: u16, y: u16) {
    Engine::set_mouse(s, y, x);
    let _ = s.submit_char(254);
    let _ = s.take_transcript();
}

fn has_label(s: &GameSession, text: &str) -> bool {
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    items.iter().any(|it| match &it.node {
        WinNode::Grid(g) => g.px_texts.iter().any(|t| t.text == text),
        _ => false,
    })
}

/// Boot, pick the Milanese deck off the menu, and play on until the confirm button
/// reads "Choose" — the game asking the player to select a card. (Same route as
/// `v6_scopa_selection_repaint.rs`.)
fn scopa_at_the_players_turn() -> Option<GameSession> {
    let mut s = boot()?;
    click(&mut s, 250, 350); // the sample Milanese card on the opening menu
    for _ in 0..40 {
        if has_label(&s, "Choose") {
            return Some(s);
        }
        click(&mut s, 590, 380); // the confirm button, to walk past the computer's turn
    }
    panic!("scopa never reached the player's turn — no \"Choose\" label appeared");
}

/// The confirm button: the one click that is legal in every state this suite reaches,
/// so both the saved session and the restored one can be driven the same way.
fn play_on(s: &mut GameSession) {
    click(s, 590, 380);
}

/// Advance a keyboard-driven story by one step, answering whichever input it is
/// waiting on. Shogun's boot alternates line and char reads.
fn step(s: &mut GameSession) {
    match s.pending_input() {
        app::session::InputKind::Char => {
            let _ = s.submit_char(13);
        }
        _ => {
            let _ = s.submit("look");
        }
    }
    let _ = s.take_transcript();
}

fn ground(s: &GameSession) -> image::RgbaImage {
    (*Engine::paint_surface(s).expect("scopa paints a ground — it draws its whole table with erases"))
        .clone()
}

fn pixels_differing(a: &image::RgbaImage, b: &image::RgbaImage) -> usize {
    assert_eq!(a.dimensions(), b.dimensions(), "the two grounds are the same screen");
    a.pixels().zip(b.pixels()).filter(|(p, q)| p != q).count()
}

fn meta() -> app::archive::Meta {
    app::archive::Meta {
        format_version: app::archive::CURRENT_FORMAT_VERSION,
        ifid: None,
        name: None,
        turns: 0,
        saved_at: String::new(),
        location: None,
        score: None,
        trigger: app::archive::SaveTrigger::HostState,
    }
}

/// Write `session` to a real `.lanthorn` and read it back, exactly as a host Save
/// State does — display list, fallback canvases, and the painted ground.
fn round_trip(session: &mut GameSession) -> app::archive::ArchiveContents {
    let es = Engine::save_state(session);
    let (dto, fallback, _diags) = session.display_list();
    let pics = session.pictures_png_for(&fallback);
    let ground_png = session.paint_ground_png();
    let path = app::scratch_dir("scopa-ground").join("save.lanthorn");
    app::archive::save_archive_meta_pics(
        &path,
        &mapper::mapper::Mapper::default(),
        &es,
        Some(&session.machine.screen),
        &session.machine.aux_data,
        meta(),
        &app::archive::SessionRecord::empty(),
        &pics,
        Some(&dto),
        ground_png.as_deref(),
    )
    .expect("save archive");
    let ac = app::archive::load_archive(&path).expect("load archive");
    let _ = std::fs::remove_file(&path);
    ac
}

/// Mirrors the app's restore order (`engine_helpers::apply_v6_pictures`), which is
/// `pub(crate)` in the binary crate and so unreachable from an integration test. The
/// funnel itself — that every restore site really does call `load_paint_ground` — is
/// pinned by a unit test beside it in `crates/app/src/engine_helpers.rs`.
fn restore_into(fresh: &mut GameSession, ac: &app::archive::ArchiveContents) {
    Engine::restore_state(fresh, &ac.engine_save()).expect("restore");
    app::session::restore_screen(fresh, ac.screen.clone().expect("screen"));
    match &ac.display {
        Some(d) => fresh.load_display_list(d, &ac.pictures),
        None => fresh.load_pictures_png(&ac.pictures),
    }
    fresh.load_paint_ground(ac.ground.as_deref());
}

/// The reported defect, on the real story.
///
/// A fresh boot sits on the main menu with the menu's ground painted. Restoring a
/// dealt hand into it must leave the DEALT HAND's ground on the screen — and go on
/// doing so once the game plays another move, which is when a restore defect
/// normally surfaces.
///
/// Falsified by making `GameSession::load_paint_ground` a no-op (the pre-fix
/// behaviour, where nothing on the restore path touched the ground):
///
/// ```text
/// assertion failed: the restored screen is still standing on the ground the FRESH
/// BOOT painted — scopa's main menu (0 of 256000 pixels differ from the menu). This
/// is SQ-0787: the player resumed a dealt hand and saw the menu's cards underneath
/// it.
/// ```
#[test]
fn a_resumed_game_does_not_stand_on_the_previous_screens_ground() {
    let Some(menu) = boot() else { return };
    let menu_ground = ground(&menu);
    drop(menu);

    let Some(mut saved) = scopa_at_the_players_turn() else { return };
    let table_ground = ground(&saved);
    assert!(
        pixels_differing(&table_ground, &menu_ground) > 0,
        "premise: a dealt hand paints a different ground from the main menu, or there \
         is nothing here to tell apart"
    );
    let ac = round_trip(&mut saved);
    assert!(ac.ground.is_some(), "the archive carries the painted ground");

    let mut fresh = boot().expect("fresh boot");
    assert_eq!(
        pixels_differing(&ground(&fresh), &menu_ground),
        0,
        "premise: the session being restored INTO is showing the main menu"
    );
    restore_into(&mut fresh, &ac);

    // Immediately after the restore — the moment everything always looks correct.
    assert_eq!(
        pixels_differing(&ground(&fresh), &table_ground),
        0,
        "the restore brings back the saved ground byte for byte"
    );

    // PERTURB, then assert (CLAUDE.md). One more move on each side: the restored
    // session must go on painting the same screen the saved one does.
    play_on(&mut saved);
    play_on(&mut fresh);
    let after_restore = ground(&fresh);
    let stale = pixels_differing(&after_restore, &menu_ground);
    assert!(
        stale > 0,
        "the restored screen is still standing on the ground the FRESH BOOT painted — \
         scopa's main menu ({stale} of {} pixels differ from the menu). This is \
         SQ-0787: the player resumed a dealt hand and saw the menu's cards underneath it.",
        menu_ground.width() * menu_ground.height()
    );
    assert_eq!(
        pixels_differing(&after_restore, &ground(&saved)),
        0,
        "a move after the restore paints the same ground the saved session paints — \
         the two are the same VM state and must reach the same screen"
    );
}

/// The archive is backend- and terminal-neutral (CLAUDE.md), and the ground is the
/// newest thing in it, so pin that here rather than assume it.
///
/// The ground is stored in the story's own 640x400 native pixels, so a save taken on
/// one terminal restores unchanged onto another — and the composite built from it is
/// the same picture whichever graphics backend is picking cells. Restores into a
/// 200x80 session and an 80x24 one, and renders each through both half-blocks and a
/// kitty-sized picker.
#[test]
fn the_restored_ground_is_the_same_at_any_pane_size_and_on_any_backend() {
    let Some(mut saved) = scopa_at_the_players_turn() else { return };
    let ac = round_trip(&mut saved);

    // Two terminals that could not be less alike, neither of them the one that saved.
    for host in [Some((80u16, 24u16)), Some((200, 80))] {
        let mut fresh = boot_on(host).expect("fresh boot");
        restore_into(&mut fresh, &ac);
        play_on(&mut fresh);
        assert_eq!(
            pixels_differing(&ground(&fresh), &ground(&saved)),
            0,
            "the ground restored into a {host:?} session is the one that was saved — \
             a v6 story lays out on its own native screen, so the terminal it is \
             restored on cannot change it"
        );

        // …and the composite the renderer builds from it is the same picture on
        // either backend, at either pane size.
        let mut composites = Vec::new();
        for (kitty, pane) in [
            (false, Rect { x: 0, y: 0, width: 80, height: 24 }),
            (true, Rect { x: 0, y: 0, width: 200, height: 80 }),
        ] {
            #[allow(deprecated)] // no terminal to query in a headless test
            let picker = if kitty {
                ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18))
            } else {
                ratatui_image::picker::Picker::halfblocks()
            };
            let mut state = app::state::AppState::default();
            state.colors = app::colors::ColorScheme::terminal_default();
            state.game_picker = Some(picker);
            *state.v6_paint.borrow_mut() = Engine::paint_surface(&fresh);
            let model = fresh.screen();
            let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
            // The gate is asked per pane/backend; the canvas it guards is native.
            let _gen = app::render::screen::v6_raster_gen(
                items, &state, pane, state.game_picker.as_ref().expect("picker"),
            );
            let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
            let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
            let (canvas, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
            composites.push(canvas);
        }
        assert_eq!(
            pixels_differing(&composites[0], &composites[1]),
            0,
            "the restored frame is the same picture on half-blocks at 80x24 as it is on \
             kitty at 200x80 — the ground is native pixels and carries no backend or \
             cell geometry"
        );
    }
}

/// The half of the fix that reaches every OTHER v6 story: an archive with no ground
/// must CLEAR the one already on screen, not leave it standing.
///
/// Zork Zero, Arthur, Journey, advent.z6 and mysterious01 paint no ground at all —
/// only a fill naming a colour OUTRIGHT paints one (`explicit_pixel_rgba`) and their
/// erases inherit, measured across 25 turns from boot on each — so their archives
/// carry none, and the reset is the only thing standing between them and whatever
/// screen preceded the restore.
#[test]
fn an_archive_without_a_ground_resets_the_one_on_screen() {
    let Some(mut fresh) = scopa_at_the_players_turn() else { return };
    assert!(Engine::paint_surface(&fresh).is_some(), "premise: this session has a ground");

    fresh.load_paint_ground(None);
    assert!(
        Engine::paint_surface(&fresh).is_none(),
        "an archive with no painted ground restores an EMPTY ground — a Save State \
         swaps memory under a game that never learns it happened, so nothing repaints \
         and whatever was on screen would otherwise survive the restore (SQ-0787)"
    );
}

/// Not a scopa quirk: Shogun paints a ground too, and shows the OTHER face of the
/// same defect.
///
/// Shogun lays a 548x368 backdrop down one keypress into its boot and never repaints
/// it. A fresh boot has not reached that point yet, so `auto_load` used to restore a
/// Shogun save onto NO ground at all — the backdrop simply missing, which is how
/// SQ-0787 was first (mis)reported on scopa: "the blue card panels do not come back".
/// Missing and stale are the same bug seen from the two ends, and the same archive
/// entry fixes both.
///
/// Falsified with the same no-op `load_paint_ground` as the scopa test above:
///
/// ```text
/// called `Option::expect()` on a `None` value: Shogun's backdrop comes back with
/// the restore — it was painted before the save and no fresh boot has painted it
/// ```
#[test]
fn shogun_restores_its_backdrop_onto_a_boot_that_never_painted_one() {
    let path = fixture_path("shogun-r322-s890706.z6");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };
    let boot_shogun = || {
        let mut pic = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
        let dims = pic.all_pict_dims();
        let mut s = GameSession::new_with_trace(
            bytes.clone(), true, false, None, false, dims, pic.std_window(), None, None,
        )
        .expect("Shogun is a valid v6 story");
        s.set_pict_source(Some(pic));
        s.flush_boot_pictures();
        let _ = s.take_transcript();
        s
    };

    let mut saved = boot_shogun();
    assert!(
        Engine::paint_surface(&saved).is_none(),
        "premise: Shogun has painted no ground at the moment auto_load fires"
    );
    // Walk the boot sequence until the backdrop goes down (measured: two steps).
    for _ in 0..8 {
        if Engine::paint_surface(&saved).is_some() {
            break;
        }
        step(&mut saved);
    }
    let backdrop = Engine::paint_surface(&saved)
        .expect("Shogun paints its backdrop within a few steps of boot")
        .clone();

    let ac = round_trip(&mut saved);
    assert!(ac.ground.is_some(), "the archive carries Shogun's backdrop");

    let mut fresh = boot_shogun();
    assert!(Engine::paint_surface(&fresh).is_none(), "premise: the fresh boot has no ground");
    restore_into(&mut fresh, &ac);

    // PERTURB, then assert (CLAUDE.md).
    step(&mut saved);
    step(&mut fresh);
    let now = Engine::paint_surface(&fresh).expect(
        "Shogun's backdrop comes back with the restore — it was painted before the save \
         and no fresh boot has painted it",
    );
    assert_eq!(
        pixels_differing(&now, &backdrop),
        0,
        "and it is the backdrop that was saved, unchanged by the move after the restore"
    );
}

