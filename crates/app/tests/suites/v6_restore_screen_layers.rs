//! SQ-0814: the two v6 screen layers that ride BESIDE the window tree — the
//! `erase_window` fills and the canvas anchors — must travel in the archive and be
//! replaced by every restore.
//!
//! SQ-0787 carried the third one, the painted ground, and named these two as its
//! siblings. They sit in exactly the same place: `GameSession` fields that no
//! restore path touched, under a window table the archive rebuilds from scratch.
//! Nothing repaints them away, because nothing knows a restore happened — Quetzal
//! saves no screen state by design (the standard assumes the STORY repaints), and a
//! host Save State swaps VM memory under a game that never learns it.
//!
//! What each one costs when it is left standing, measured on the real corpus:
//!
//! * **fills** (`window_fills`, SQ-0584). ZMSD §8.8.5.3 makes erasing a window an
//!   opaque fill of its rect — that is what makes advent.z6's help panel a solid
//!   panel rather than text floating over the story. Journey two steps into its boot
//!   is covering three windows with fills; one step later it is covering none.
//!   Restore the later save onto the earlier screen and, before this quest, all
//!   three stale fills stayed — three opaque bands over a screen that had moved on.
//!   Restore it the other way and the three the save DID carry never arrived.
//! * **anchors** (`canvas_anchor`, SQ-0715). An anchor records where a window's
//!   canvas was painted, so a later `move_window` can strand those pixels onto the
//!   ground at the coordinates the hardware left them (ZMSD §8: "subsequent
//!   movements of the window do not move what was printed"). A restored session
//!   inherited the PREVIOUS game's anchors, so the first `move_window` after a
//!   restore stranded the restored canvas at coordinates belonging to a screen that
//!   no longer existed. Journey, Zork Zero, Shogun and fmvpoker all hold live
//!   anchors within three steps of boot.
//!
//! Both travel as a RECIPE (`archive::V6LayersDto` inside `display.json`), not as
//! pixels: unlike the ground, whose inputs are an unbounded stream of fills, these
//! are one small struct per window however long the session runs. Everything in them
//! is native pixels or a packed colour, so the archive stays terminal- and
//! backend-neutral — pinned below by restoring into two very different terminals and
//! rendering the result on two different graphics backends.
//!
//! Per CLAUDE.md every test here PERTURBS before asserting: the frame immediately
//! after a restore is exactly when everything still looks correct. And both
//! `honor_game_colours` modes are pinned, because a fill carries a colour.
//!
//! The stories are gitignored (CLAUDE.md), so every test skips cleanly without them.


use app::engine::{Engine, WinNode};
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
/// State does — display list (which now carries the screen layers), fallback
/// canvases, and the painted ground.
fn round_trip(session: &mut GameSession) -> app::archive::ArchiveContents {
    let es = Engine::save_state(session);
    let (dto, fallback, _diags) = session.display_list();
    let pics = session.pictures_png_for(&fallback);
    let ground_png = session.paint_ground_png();
    let path = app::scratch_dir("layers").join("save.lanthorn");
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
/// `pub(crate)` in the binary crate and so unreachable from an integration test. That
/// every restore site really does call `load_v6_screen_layers` is pinned by a unit
/// test beside it in `crates/app/src/engine_helpers.rs`.
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

fn ground(s: &GameSession) -> Option<image::RgbaImage> {
    Engine::paint_surface(s).map(|a| (*a).clone())
}

/// Which windows the SCREEN MODEL is covering with an erase fill, and with what
/// colour — the field the renderer consumes (`GridWindow::fill`), so this is the
/// layer's user-visible face rather than its storage.
fn covered_windows(s: &GameSession) -> Vec<(usize, u32)> {
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    items
        .iter()
        .enumerate()
        .filter_map(|(i, pw)| match &pw.node {
            WinNode::Grid(g) => g.fill.map(|f| (i, f.bg)),
            _ => None,
        })
        .collect()
}

/// The corpus: `(story, steps_to_the_save, steps_to_the_screen_it_is_restored_onto)`.
/// Chosen by measuring the whole `stories/` v6 set — every pair below has the saved
/// screen's layers DIFFERING from the screen being restored onto, which is what makes
/// the test able to fail.
const CASES: &[(&str, usize, usize)] = &[
    // Journey both ways round: at two steps it is covering three windows and holds no
    // anchor; at three it is covering none and holds an anchor on window 3. One
    // ordering exercises "the save's layers must arrive", the other "the screen's
    // layers must go".
    ("journey-r83-s890706.z6", 2, 3),
    ("journey-r83-s890706.z6", 3, 2),
    // Shogun: the save covers nothing, the fresh boot covers window 7 edge to edge.
    ("shogun-r322-s890706.z6", 2, 0),
    // advent: the save covers its top band (window 1), the fresh boot covers nothing.
    ("advent.z6", 2, 0),
    // fmvpoker: an anchor on window 0 that a fresh boot has not made yet.
    ("fmvpoker.z6", 2, 0),
];

/// The defect itself, across the corpus and both colour modes.
///
/// Falsified by reverting `load_v6_screen_layers` to a no-op — the pre-quest state,
/// where nothing on the restore path touched either layer:
///
/// ```text
/// journey-r83-s890706.z6 (honor=true), saved at 2 steps and restored onto the
/// screen at 3: a move after the restore leaves the screen wearing layers that are
/// not the saved game's.
///   saved:    V6LayersDto { fills: [V6FillDto { win: 0, .. }, V6FillDto { win: 1, .. },
///             V6FillDto { win: 3, .. }], anchors: [] }
///   restored: V6LayersDto { fills: [], anchors: [V6AnchorDto { win: 3, .. }] }
/// ```
fn a_restore_replaces_the_screen_layers_it_found(honor: bool) {
    for &(story, saved_steps, fresh_steps) in CASES {
        let Some(mut saved) = boot_to(story, saved_steps, honor, None) else { return };
        let Some(mut fresh) = boot_to(story, fresh_steps, honor, None) else { return };

        let before = fresh.v6_screen_layers();
        assert_ne!(
            saved.v6_screen_layers(),
            before,
            "premise: {story} at {saved_steps} steps and at {fresh_steps} steps must show \
             DIFFERENT screen layers, or this case cannot tell a restore from a no-op"
        );

        let ac = round_trip(&mut saved);
        assert_eq!(
            ac.display.as_ref().map(|d| &d.layers),
            Some(&saved.v6_screen_layers()),
            "{story}: the archive carries the saved screen's layers"
        );
        restore_into(&mut fresh, &ac);

        // The frame the instant the restore lands. A stale FILL is loudest here and
        // quietest a move later, because a fill stops covering the moment the story
        // prints its next character — so it is checked here as well as after the
        // perturb, not instead of it.
        assert_eq!(
            covered_windows(&fresh),
            covered_windows(&saved),
            "{story} (honor={honor}), restored onto the screen at {fresh_steps} steps: the \
             windows covered by an opaque erase fill the instant the restore lands are the \
             saved game's. Anything else is the PREVIOUS screen's fills over the restored \
             one — three opaque bands, in Journey's case."
        );

        // PERTURB, then assert (CLAUDE.md). One more move on each side: the two are the
        // same VM state, so they must go on reaching the same screen.
        step(&mut saved);
        step(&mut fresh);
        assert_eq!(
            fresh.v6_screen_layers(),
            saved.v6_screen_layers(),
            "{story} (honor={honor}), saved at {saved_steps} steps and restored onto the \
             screen at {fresh_steps}: a move after the restore leaves the screen wearing \
             layers that are not the saved game's.\n  saved:    {:?}\n  restored: {:?}",
            saved.v6_screen_layers(),
            fresh.v6_screen_layers(),
        );
        assert_eq!(
            covered_windows(&fresh),
            covered_windows(&saved),
            "{story} (honor={honor}): …and the windows the model actually covers with an \
             opaque fill are the saved game's, not the ones the pre-restore screen left"
        );
        assert_eq!(
            ground(&fresh),
            ground(&saved),
            "{story}: the painted ground under them is the saved game's too (SQ-0787), \
             which is also where a stale ANCHOR would show — a `move_window` after the \
             restore strands the canvas onto the ground at the anchor's coordinates"
        );
    }
}

#[test]
fn a_restore_replaces_the_screen_layers_it_found_honouring_game_colours() {
    a_restore_replaces_the_screen_layers_it_found(true);
}

#[test]
fn a_restore_replaces_the_screen_layers_it_found_with_theme_colours() {
    a_restore_replaces_the_screen_layers_it_found(false);
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

/// The archive is backend- and terminal-neutral (CLAUDE.md), and these layers are the
/// newest thing in it, so pin that rather than assume it.
///
/// A fill is stored as native pixels and a packed colour, an anchor as native pixels,
/// so ONE archive restored into two terminals that could not be less alike must reach
/// the same screen — and that screen must draw the same whichever backend is picking
/// cells. Runs the whole corpus into an 80x24 session and a 200x80 one, neither of
/// them the terminal that wrote the archive, and renders both through half-blocks and
/// a kitty-sized picker.
///
/// Two restored sessions are compared against each other rather than against a
/// separately-played oracle, because a same-state oracle is what makes THIS claim
/// falsifiable: two restores of the same archive can differ only in what the terminal
/// contributed, which is exactly the property under test and must be nothing. A
/// whole-pane comparison against a natively-played session is a different and stronger
/// claim, and it lives in `v6_restore_streamed_prose.rs` — it was unavailable when
/// this suite landed, because the window's streamed text runs did not survive
/// `restore_screen` at all (measured on fmvpoker; SQ-0820 carried them).
///
/// Falsified with the same no-op `load_v6_screen_layers` as the test above:
///
/// ```text
/// journey-r83-s890706.z6: the layers restored into a Some((80, 24)) session are the
/// ones that were saved — a v6 story lays out on its own native screen, so the
/// terminal it is restored on cannot change them
///   left: V6LayersDto { fills: [], anchors: [] }
///  right: V6LayersDto { fills: [], anchors: [V6AnchorDto { win: 3, .. }] }
/// ```
fn the_restored_layers_carry_no_terminal(honor: bool) {
    for &(story, saved_steps, fresh_steps) in CASES {
        // The archive is written by a session on the DEFAULT terminal, and restored
        // below onto two others. The writer, played one move on, is the oracle for the
        // layers themselves — it is the same VM state, so it must agree exactly.
        let Some(mut writer) = boot_to(story, saved_steps, honor, None) else { return };
        let ac = round_trip(&mut writer);
        step(&mut writer);

        let mut restored = Vec::new();
        for host in [Some((80u16, 24u16)), Some((200, 80))] {
            let mut fresh = boot_to(story, fresh_steps, honor, host).expect("fresh boot");
            restore_into(&mut fresh, &ac);
            step(&mut fresh); // PERTURB, then assert (CLAUDE.md)
            assert_eq!(
                fresh.v6_screen_layers(),
                writer.v6_screen_layers(),
                "{story}: the layers restored into a {host:?} session are the ones that were \
                 saved — a v6 story lays out on its own native screen, so the terminal it is \
                 restored on cannot change them"
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
                 the archive was restored into an 80x24 session or a 200x80 one. The layers \
                 carry no cell geometry, font metrics or picker state, so nothing about the \
                 terminal a save lands on can change what they paint.",
                if kitty { "kitty" } else { "half-blocks" },
            );
        }
    }
}

#[test]
fn the_restored_layers_carry_no_terminal_honouring_game_colours() {
    the_restored_layers_carry_no_terminal(true);
}

#[test]
fn the_restored_layers_carry_no_terminal_with_theme_colours() {
    the_restored_layers_carry_no_terminal(false);
}
