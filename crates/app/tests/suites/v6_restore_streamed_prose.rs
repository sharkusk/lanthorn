//! SQ-0820: the two pixel-run layers a v6 PROSE window carries — the prose it has
//! streamed, and the prose a move or resize froze behind it — must travel in the
//! archive and come back with a restore.
//!
//! `ZWindow` holds a v6 window's text as three layers in the same screen-absolute
//! pixel space. `texts` is the window's own painted layer and has always been
//! archived; `streamed` is where the prose the window sent to the host transcript is
//! currently SITTING (SQ-0697/SQ-0729), and `retired` is the prose a `move_window` or
//! `window_size` left frozen at coordinates the window no longer covers (ZMSD §15:
//! "window_size does not change the current display"). Only the first of the three
//! was in `screen.json`.
//!
//! What that cost, measured on the real corpus: fmvpoker.z6 two steps from boot is
//! holding `[PxText { y: 247, x: 76, text: "Current Bet:" }, PxText { y: 265, x: 76,
//! text: "10" }, …]` in `streamed` and NOWHERE else that a save carried — so a
//! resumed game came back with its bet and winnings legends missing from the pixel
//! raster. Shogun one step in is holding nine `retired` runs (its whole title header,
//! frozen when it moved window 0 down beside the menu) and lost the lot. Nothing
//! repaints them, because nothing knows a restore happened: Quetzal saves no screen
//! state by design (the standard assumes the STORY repaints), and a host Save State
//! swaps VM memory under a game that never learns it.
//!
//! It hid in cell mode because the CELL grid (`cells`) was archived all along — the
//! defect only shows in the raster/hybrid composite, which is what these tests read.
//!
//! Both layers travel as a RECIPE, not a result: they are the game's own runs in
//! zvm's native pixel space, exactly as `texts` travels, so the archive stays
//! terminal- and backend-neutral — pinned below by restoring one archive into two
//! very different terminals and rendering the result on two graphics backends.
//!
//! Per CLAUDE.md every test here PERTURBS before asserting: the frame immediately
//! after a restore is exactly when everything still looks correct. Both
//! `honor_game_colours` modes are pinned, because a run carries a colour pair.
//!
//! The stories are gitignored (CLAUDE.md), so every test skips cleanly without them.


use app::engine::{Engine, PxText, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;


/// Boot `story` with `honor_game_colours` set and the session believing it is on
/// `host_screen` — varied by the different-terminal test, and irrelevant to a v6
/// story's own 640x400 native screen, which is part of what is being pinned.
fn boot(story: &str, honor: bool, host_screen: Option<(u16, u16)>) -> Option<GameSession> {
    let path = fixture_path(story);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut pic = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = pic.all_pict_dims();
    let mut s = GameSession::new_with_trace(
        bytes, honor, false, None, false, dims, pic.std_window(), None, host_screen,
    )
    .expect("a valid v6 story");
    s.set_pict_source(Some(pic));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    Some(s)
}

/// One step of a keyboard-driven story, answering whichever input it is waiting on.
fn step(s: &mut GameSession) {
    match s.pending_input() {
        InputKind::Char => {
            let _ = s.submit_char(13);
        }
        _ => {
            let _ = s.submit("look");
        }
    }
    let _ = s.take_transcript();
}

fn boot_to(story: &str, steps: usize, honor: bool, host: Option<(u16, u16)>) -> Option<GameSession> {
    let mut s = boot(story, honor, host)?;
    for _ in 0..steps {
        step(&mut s);
    }
    Some(s)
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
/// State does — screen (which now carries all three pixel-run layers), display list,
/// fallback canvases and the painted ground.
fn round_trip(session: &mut GameSession) -> app::archive::ArchiveContents {
    let es = Engine::save_state(session);
    let (dto, fallback, _diags) = session.display_list();
    let pics = session.pictures_png_for(&fallback);
    let ground_png = session.paint_ground_png();
    let path = app::scratch_dir("streamed").join("save.lanthorn");
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
/// `pub(crate)` in the binary crate and so unreachable from an integration test.
fn restore_into(fresh: &mut GameSession, ac: &app::archive::ArchiveContents) {
    Engine::restore_state(fresh, &ac.engine_save()).expect("restore");
    app::session::restore_screen(fresh, ac.screen.clone().expect("screen"));
    match &ac.display {
        Some(d) => fresh.load_display_list(d, &ac.pictures),
        None => fresh.load_pictures_png(&ac.pictures),
    }
    fresh.load_paint_ground(ac.ground.as_deref());
    fresh.load_v6_screen_layers(ac.display.as_ref().map(|d| &d.layers));
}

/// Every pixel-positioned text run the SCREEN MODEL is publishing, in z-order — the
/// fields the raster renderer consumes (`BufferWindow::px_runs` from `streamed`,
/// `GridWindow::px_texts` from `texts` and from the retired-prose layer), so this is
/// the layers' user-visible face rather than their storage.
fn px_layers(s: &GameSession) -> Vec<(usize, Vec<PxText>)> {
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    items
        .iter()
        .enumerate()
        .filter_map(|(i, pw)| match &pw.node {
            WinNode::Grid(g) if !g.px_texts.is_empty() => Some((i, g.px_texts.clone())),
            WinNode::Buffer(b) if !b.px_runs.is_empty() => Some((i, b.px_runs.clone())),
            _ => None,
        })
        .collect()
}

/// The corpus: `(story, steps_to_the_save, steps_to_the_screen_it_is_restored_onto)`.
/// Every pair was chosen by measuring the whole `stories/` v6 set — the saved screen's
/// pixel runs DIFFER from those of the screen being restored onto, which is what makes
/// each case able to fail.
const CASES: &[(&str, usize, usize)] = &[
    // fmvpoker, the quest's fixture: window 0 streams the table legends and holds four
    // runs at every step, so the save and the screen differ by their TEXT ("Total
    // Winnings: 1000" against "990") rather than by their count — the case a
    // length check would pass and a content check catches.
    ("fmvpoker.z6", 2, 1),
    // …and against the title screen, which streams an entirely different four runs.
    ("fmvpoker.z6", 1, 0),
    // Shogun one step in has nine RETIRED runs — its whole title header, frozen when it
    // moved window 0 down beside the menu — plus a live streamed run. A fresh boot has
    // neither, so this is the `retired` layer's case.
    ("shogun-r322-s890706.z6", 1, 0),
    // Journey at three steps streams eight runs; at two it streams none. One ordering
    // exercises "the save's runs must arrive", the other "the screen's runs must go".
    ("journey-r83-s890706.z6", 3, 2),
    ("journey-r83-s890706.z6", 2, 3),
    // advent streams through window 7, a SECONDARY prose window rather than window 0 —
    // a different publishing path in the screen model, and empty one step in.
    ("advent.z6", 2, 1),
];

/// The defect itself, across the corpus and both colour modes.
///
/// Falsified by reverting `ZWindowDto`'s `streamed`/`retired` to nothing (the
/// pre-quest state, where the archive carried only `texts`):
///
/// ```text
/// fmvpoker.z6 (honor=true), saved at 2 steps and restored onto the screen at 1: a
/// move after the restore leaves the screen showing pixel runs that are not the saved
/// game's.
///   saved:    [(1, [PxText { y: 247, x: 76, text: "Current Bet:", .. }, …])]
///   restored: []
/// ```
///
/// …and reverting `retired` ALONE, which the perturbed assertion cannot see (Shogun's
/// frozen runs are gone from both sides a move later) but the instant frame does:
///
/// ```text
/// shogun-r322-s890706.z6 (honor=true), restored onto the screen at 0 steps: the pixel
/// runs on the glass the instant the restore lands are the saved game's.
///   saved:    [(1, [PxText { y: 49, x: 297, text: "SHOGUN", .. }, …9 runs…]), …]
///   restored: [(2, [PxText { y: 337, x: 47, text: "You may choose to: ", .. }]), …]
/// ```
fn a_restore_brings_back_the_streamed_prose(honor: bool) {
    for &(story, saved_steps, fresh_steps) in CASES {
        let Some(mut saved) = boot_to(story, saved_steps, honor, None) else { return };
        let Some(mut fresh) = boot_to(story, fresh_steps, honor, None) else { return };

        assert_ne!(
            px_layers(&saved),
            px_layers(&fresh),
            "premise: {story} at {saved_steps} steps and at {fresh_steps} steps must show \
             DIFFERENT pixel runs, or this case cannot tell a restore from a no-op"
        );

        let ac = round_trip(&mut saved);
        restore_into(&mut fresh, &ac);

        // The frame the instant the restore lands. A RETIRED layer is loudest here and
        // quietest a move later: it is prose the hardware froze, and the story's next
        // repaint erases the window it was frozen from — measured across the whole v6
        // corpus, Shogun one step in is the only session that holds any, and it holds
        // none a step later. So it is checked here as WELL as after the perturb, not
        // instead of it (the same shape SQ-0814's erase fills have).
        assert_eq!(
            px_layers(&fresh),
            px_layers(&saved),
            "{story} (honor={honor}), restored onto the screen at {fresh_steps} steps: the \
             pixel runs on the glass the instant the restore lands are the saved game's. \
             Anything else is the PREVIOUS screen's — Shogun's nine frozen title runs, in \
             its case, either lost or left standing.\n  saved:    {:?}\n  restored: {:?}",
            px_layers(&saved),
            px_layers(&fresh),
        );

        // PERTURB, then assert (CLAUDE.md). One more move on each side: the two are the
        // same VM state, so they must go on reaching the same screen.
        step(&mut saved);
        step(&mut fresh);
        assert_eq!(
            px_layers(&fresh),
            px_layers(&saved),
            "{story} (honor={honor}), saved at {saved_steps} steps and restored onto the \
             screen at {fresh_steps}: a move after the restore leaves the screen showing \
             pixel runs that are not the saved game's.\n  saved:    {:?}\n  restored: {:?}",
            px_layers(&saved),
            px_layers(&fresh),
        );
    }
}

#[test]
fn a_restore_brings_back_the_streamed_prose_honouring_game_colours() {
    a_restore_brings_back_the_streamed_prose(true);
}

#[test]
fn a_restore_brings_back_the_streamed_prose_with_theme_colours() {
    a_restore_brings_back_the_streamed_prose(false);
}

/// Render the restored model the way the terminal does, on one backend at one size.
fn render(session: &GameSession, honor: bool, kitty: bool, w: u16, h: u16) -> Buffer {
    #[allow(deprecated)] // no terminal to query in a headless test
    let picker = if kitty {
        ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18))
    } else {
        ratatui_image::picker::Picker::halfblocks()
    };
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(picker);
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    *state.v6_paint.borrow_mut() = Engine::paint_surface(session);
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&session.screen(), false, None, &state, area, &mut buf);
    buf
}

/// The archive is backend- and terminal-neutral (CLAUDE.md), so pin that for the two
/// layers this quest adds rather than assume it.
///
/// A run is stored as native pixels plus a `ZColour` pair — no cell coordinates, no
/// font metrics, no picker state — so ONE archive restored into two terminals that
/// could not be less alike must reach the same screen, and that screen must draw the
/// same whichever backend is picking cells. The whole corpus goes into an 80x24
/// session and a 200x80 one, neither of them the terminal that wrote the archive, and
/// both are rendered through half-blocks and a kitty-sized picker.
///
/// Falsified with the same reverted `ZWindowDto`:
///
/// ```text
/// fmvpoker.z6: the pixel runs restored into a Some((80, 24)) session are the ones
/// that were saved — a v6 story lays out on its own native screen, so the terminal it
/// is restored on cannot change them
///   left: []
///  right: [(1, [PxText { y: 247, x: 76, text: "Current Bet:", .. }, …])]
/// ```
fn the_restored_runs_carry_no_terminal(honor: bool) {
    for &(story, saved_steps, fresh_steps) in CASES {
        // The archive is written by a session on the DEFAULT terminal and restored
        // below onto two others. The writer, played one move on, is the oracle — it is
        // the same VM state, so it must agree exactly.
        let Some(mut writer) = boot_to(story, saved_steps, honor, None) else { return };
        let ac = round_trip(&mut writer);
        step(&mut writer);

        let mut restored = Vec::new();
        for host in [Some((80u16, 24u16)), Some((200, 80))] {
            let mut fresh = boot_to(story, fresh_steps, honor, host).expect("fresh boot");
            restore_into(&mut fresh, &ac);
            step(&mut fresh); // PERTURB, then assert (CLAUDE.md)
            assert_eq!(
                px_layers(&fresh),
                px_layers(&writer),
                "{story}: the pixel runs restored into a {host:?} session are the ones that \
                 were saved — a v6 story lays out on its own native screen, so the terminal \
                 it is restored on cannot change them"
            );
            restored.push(fresh);
        }

        // …and the pane the renderer builds is the same whichever terminal the restore
        // landed in, on either backend, at either size.
        for (kitty, w, h) in [(false, 80u16, 25u16), (true, 200, 60)] {
            assert_eq!(
                render(&restored[0], honor, kitty, w, h),
                render(&restored[1], honor, kitty, w, h),
                "{story} drawn at {w}x{h} on {}: the pane is the same cell for cell whether \
                 the archive was restored into an 80x24 session or a 200x80 one. A pixel run \
                 carries no cell geometry, font metrics or picker state, so nothing about the \
                 terminal a save lands on can change what it paints.",
                if kitty { "kitty" } else { "half-blocks" },
            );
        }
    }
}

#[test]
fn the_restored_runs_carry_no_terminal_honouring_game_colours() {
    the_restored_runs_carry_no_terminal(true);
}

#[test]
fn the_restored_runs_carry_no_terminal_with_theme_colours() {
    the_restored_runs_carry_no_terminal(false);
}

/// The whole-pane oracle SQ-0814 had to back away from, now that it can be met.
///
/// That quest's note reads: "the pane a restore reproduces is not the whole game (v6
/// window text runs do not survive `restore_screen` today, measured on fmvpoker), so a
/// whole-pane comparison against a natively-played session would be measuring that
/// instead of this" — and this quest IS that. So compare the strongest way there is:
/// restore an archive into a fresh boot, play a move, and render the result against a
/// session that reached the same state entirely under its own steam.
///
/// fmvpoker.z6 is the fixture because it is deterministic across two independent boots
/// and across declared host sizes, which the oracle requires and Journey and advent do
/// not offer. Rendered on both backends, and both colour modes are pinned by the two
/// callers below.
fn a_restored_pane_matches_a_natively_played_one(honor: bool) {
    let Some(mut saved) = boot_to("fmvpoker.z6", 2, honor, None) else { return };
    let ac = round_trip(&mut saved);
    let Some(mut fresh) = boot_to("fmvpoker.z6", 0, honor, None) else { return };
    restore_into(&mut fresh, &ac);

    // PERTURB, then assert (CLAUDE.md).
    step(&mut saved);
    step(&mut fresh);

    for (kitty, w, h) in [(false, 100u16, 30u16), (true, 160, 50)] {
        assert_eq!(
            render(&fresh, honor, kitty, w, h),
            render(&saved, honor, kitty, w, h),
            "fmvpoker.z6 (honor={honor}) drawn at {w}x{h} on {}: a session restored from the \
             archive and played one move must render cell for cell like the session that \
             wrote it and played the same move. Before SQ-0820 the difference was the \
             prose window's streamed runs — its \"Current Bet:\" and \"Total Winnings:\" \
             legends — which the archive did not carry.",
            if kitty { "kitty" } else { "half-blocks" },
        );
    }
}

#[test]
fn a_restored_pane_matches_a_natively_played_one_honouring_game_colours() {
    a_restored_pane_matches_a_natively_played_one(true);
}

#[test]
fn a_restored_pane_matches_a_natively_played_one_with_theme_colours() {
    a_restored_pane_matches_a_natively_played_one(false);
}
