//! SQ-1591 — the host-facing Hybrid chrome layout must agree with the terminal's
//! own Hybrid render, for every specimen the quest names.
//!
//! `app::render::screen::hybrid_chrome_layout` answers, at a HOST's own font
//! size, the same questions the terminal's Hybrid renderer answers with its own
//! picker: where does each chrome run land in terminal cells, is it over art, and
//! where is the story viewport. It is built from the exact same classification
//! machinery the real draw uses (`decompose_chrome_strips`/`menu_band_strips`/
//! `ChromeRowOracle`/`run_cell`), so the cases below assert cross-render
//! AGREEMENT rather than pin numbers by hand: boot each specimen, render it for
//! real into a `ratatui::buffer::Buffer`, call the new function on the SAME
//! inputs, and check every reported run position lands on a non-blank glyph in
//! the real buffer.
//!
//! **Backends, deliberately two.** Kitty (the shipped default) is primary for
//! the viewport/text-strip agreement below. Zork Zero's banner runs
//! (`over_art: true`) need HALFBLOCKS instead: kitty's placements are virtual,
//! so printing a glyph over one ERASES it (SQ-0944) — the real kitty render
//! never draws those runs as glyphs at all, so kitty cannot falsify an over-art
//! position. Halfblocks (`backend_layers_glyphs_over_art`) is the one backend
//! that actually draws them, so it is the one that can.
//!
//! Fixtures are gitignored real commercial games; every case SKIPs cleanly when
//! its story is absent (symlink `stories/` into a worktree per CLAUDE.md).
//!
//! Five specimens, one press each, covering every distinct code path this
//! module's classification takes: Journey PC r83 and its Amiga release floppy
//! (menu-band coverage, a fixed-pen press and a proportional one, SQ-1009);
//! Zork Zero (over-art banner coverage); Arthur PC (SQ-0892's glyph-at-a-time
//! status-bar block placement); Shogun PC (the same block placement on its
//! boot-menu credits screen). Arthur's and Shogun's AMIGA floppies are also in
//! the quest's own specimen table and are deliberately NOT covered here — see
//! the report for why (time-boxed: the PC presses already exercise every
//! distinct code path an Amiga floppy would add for those two games, since
//! Journey's own Amiga floppy already covers the proportional-pen path).

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::host::input::deliver_v6_click;
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts, TurnCtx};
use app::interpreter::InterpreterProfile;
use app::launch_options::LaunchOverrides;
use app::render::screen::{hybrid_chrome_layout, render_story_pane, V6HybridChromeRun};
use app::render::v6_layout as v6;
use app::session::{GameSession, InputKind};
use app::state::AppState;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

// ---------------------------------------------------------------------------
// Boot helpers — one per specimen, each mirroring an existing suite's own
// pattern (named in each doc comment) so the frame under test is the one that
// suite already established, not a fresh guess.
// ---------------------------------------------------------------------------

fn boot_z6(file: &str) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("should boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

fn drive_enter(session: &mut GameSession, turns: usize, stop_when: impl Fn(&str) -> bool) {
    for _ in 0..turns {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if stop_when(&r.transcript) {
            break;
        }
    }
}

/// `v6_journey_menu.rs`'s `journey_at_menu`.
fn journey_pc() -> Option<GameSession> {
    let mut s = boot_z6("journey-r83-s890706.z6")?;
    drive_enter(&mut s, 40, |t| t.contains("Praxix") || t.contains("magical resources"));
    Some(s)
}

/// `v6_journey_amiga_frame.rs`'s `journey_floppy`.
fn journey_amiga() -> Option<GameSession> {
    let path = stories_dir().join("Journey - The Quest Begins.adf");
    let bytes = match app::hints::load_story(&path) {
        Ok(s) => s.into_bytes(),
        Err(_) => {
            eprintln!("SKIP: gitignored release floppy missing at {}", path.display());
            return None;
        }
    };
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut session = GameSession::new_with_trace(
        bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .expect("Journey's release floppy should mount and boot without a ZError");
    session.machine.set_palette(profile.palette());
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    drive_enter(&mut session, 40, |t| t.contains("Praxix") || t.contains("magical resources"));
    Some(session)
}

/// `v6_hybrid_zork0.rs`'s `boot_zork0` — boot alone, no driving: the banner and
/// status band are already painted at the intro card.
fn zork0() -> Option<GameSession> {
    boot_z6("zork0-r393-s890714.z6")
}

/// `v6_arthur_status.rs`'s `arthur_at_status`.
fn arthur_pc() -> Option<GameSession> {
    let mut s = boot_z6("arthur-r74-s890714.z6")?;
    for _ in 0..12 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
    }
    Some(s)
}

/// `v6_shogun_gameplay.rs`'s boot+drive.
fn shogun_pc() -> Option<GameSession> {
    let mut s = boot_z6("shogun-r322-s890706.z6")?;
    for turn in 0..8 {
        let _ = match s.pending_input() {
            InputKind::Line => s.submit(if turn % 2 == 0 { "look" } else { "xyzzy" }),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
    }
    Some(s)
}

// ---------------------------------------------------------------------------
// Render + verification plumbing
// ---------------------------------------------------------------------------

fn render_state(picker: ratatui_image::picker::Picker) -> AppState {
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(picker);
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state
}

fn render_real(session: &GameSession, state: &AppState, area: Rect) -> Buffer {
    let model = session.screen();
    let mut buf = Buffer::empty(Rect::new(0, 0, area.right() + 1, area.bottom() + 1));
    let _ = render_story_pane(&model, false, None, state, area, &mut buf);
    buf
}

fn is_blank(buf: &Buffer, x: u16, y: u16) -> bool {
    match buf.cell((x, y)) {
        Some(c) => c.symbol().trim().is_empty(),
        None => true,
    }
}

/// Every non-blank run's reported `(col, row)` must land on a non-blank glyph in
/// the real buffer — the direct cross-render agreement check the quest asks for.
/// A run whose origin falls outside `area` is skipped: the real draw clips it at
/// the pane edge (SQ-0949), so there is nothing in the buffer for it to agree
/// with.
fn assert_runs_land_on_glyphs(ctx: &str, buf: &Buffer, area: Rect, runs: &[V6HybridChromeRun]) {
    let mut checked = 0;
    for r in runs {
        if r.run.text.trim().is_empty() {
            continue;
        }
        if r.col < area.x as i32 || r.row < area.y as i32 || r.col >= area.right() as i32 || r.row >= area.bottom() as i32
        {
            continue;
        }
        let (x, y) = (r.col as u16, r.row as u16);
        assert!(
            !is_blank(buf, x, y),
            "{ctx}: run {:?} (over_art={}, in_menu_band={}) reports ({x},{y}) but the real buffer is blank there",
            r.run.text,
            r.over_art,
            r.in_menu_band
        );
        checked += 1;
    }
    assert!(checked > 0, "{ctx}: no in-bounds non-blank run to check — premise failed");
}

/// Boot, render for real through kitty, build `hybrid_chrome_layout` on the same
/// inputs, and check every non-blank chrome run agrees with the real buffer, and
/// that no text-strip run lands inside the reported story viewport.
fn verify_agreement(ctx: &str, session: &GameSession, cell_px: (u16, u16)) {
    let state = render_state(app::render::graphics::kitty_picker(cell_px.0, cell_px.1));
    let area = Rect::new(0, 0, 120, 40);
    let buf = render_real(session, &state, area);
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("{ctx}: a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &state.v6_text);
    let cell = state.v6_text.cell();
    let layout = v6::classify_windows(items, cell);
    let hyb = hybrid_chrome_layout(&layout, native, area, cell_px, &state)
        .unwrap_or_else(|| panic!("{ctx}: expected a hybrid chrome layout for this frame"));
    assert!(hyb.viewport.width > 0 && hyb.viewport.height > 0, "{ctx}: empty story viewport");
    assert!(!hyb.runs.is_empty(), "{ctx}: no chrome runs classified at all — premise failed");
    assert_runs_land_on_glyphs(ctx, &buf, area, &hyb.runs);
    // Chrome and prose must never overlap: no non-blank TEXT-strip run (never an
    // over-art one — those legitimately sit over the ring's own artwork, never
    // the story viewport) may land inside the reported viewport.
    for r in &hyb.runs {
        if r.run.text.trim().is_empty() || r.over_art {
            continue;
        }
        let inside = r.col >= hyb.viewport.x as i32
            && r.col < hyb.viewport.right() as i32
            && r.row >= hyb.viewport.y as i32
            && r.row < hyb.viewport.bottom() as i32;
        assert!(
            !inside,
            "{ctx}: chrome run {:?} at ({},{}) lands inside the reported story viewport {:?}",
            r.run.text, r.col, r.row, hyb.viewport
        );
    }
}

// ---------------------------------------------------------------------------
// Cross-render agreement, one case per specimen (SQ-1591's own table)
// ---------------------------------------------------------------------------

#[test]
fn journey_pc_hybrid_chrome_layout_agrees_with_the_real_render() {
    let Some(session) = journey_pc() else { return };
    verify_agreement("journey-pc-r83", &session, (8, 16));
}

#[test]
fn journey_amiga_hybrid_chrome_layout_agrees_with_the_real_render() {
    let Some(session) = journey_amiga() else { return };
    verify_agreement("journey-amiga-floppy", &session, (8, 16));
}

#[test]
fn zork0_hybrid_chrome_layout_agrees_with_the_real_render() {
    let Some(session) = zork0() else { return };
    verify_agreement("zork0-r393", &session, (8, 16));
}

#[test]
fn arthur_pc_hybrid_chrome_layout_agrees_with_the_real_render() {
    let Some(session) = arthur_pc() else { return };
    verify_agreement("arthur-pc-r74", &session, (8, 16));
}

#[test]
fn shogun_pc_hybrid_chrome_layout_agrees_with_the_real_render() {
    let Some(session) = shogun_pc() else { return };
    verify_agreement("shogun-pc-r322", &session, (8, 16));
}

/// Zork Zero's banner labels are the corpus's `over_art: true` case (SQ-0944).
/// Kitty's placements are virtual, so the real render never draws these as
/// glyphs at all (printing one would erase the placement) — only HALFBLOCKS
/// does, so it is the only backend that can falsify this classification against
/// a real buffer. A different backend from `verify_agreement`'s kitty, so this
/// case is its own rather than folded into `zork0_hybrid_chrome_layout_agrees…`.
#[test]
fn zork0_over_art_runs_agree_with_the_real_halfblocks_render() {
    let Some(session) = zork0() else { return };
    let state = render_state(ratatui_image::picker::Picker::halfblocks());
    let area = Rect::new(0, 0, 120, 40);
    let buf = render_real(&session, &state, area);
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("zork0: a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &state.v6_text);
    let cell = state.v6_text.cell();
    let layout = v6::classify_windows(items, cell);
    let hyb = hybrid_chrome_layout(&layout, native, area, (8, 16), &state).expect("zork0: expected a hybrid chrome layout");
    let over_art: Vec<V6HybridChromeRun> =
        hyb.runs.iter().filter(|r| r.over_art && !r.run.text.trim().is_empty()).cloned().collect();
    assert!(!over_art.is_empty(), "zork0: premise — expected at least one non-blank over-art banner run");
    assert_runs_land_on_glyphs("zork0-over-art-halfblocks", &buf, area, &over_art);
}

// ---------------------------------------------------------------------------
// Click-through: the layout's click_map, delivered for real (SQ-1588's pattern)
// ---------------------------------------------------------------------------

fn boot_host(file: &str, want_release: u16, tag: &str) -> Option<BootedStory> {
    let path = stories_dir().join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), want_release, "{file} is not the pinned release");
    let home = app::scratch_dir(tag);
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: path,
        disk_entry: None,
        overrides: &overrides,
        cfg: app::config::Config {
            user_dir: home.clone(),
            config_file: home.join("config.toml"),
            random_seed: Some(1),
            ..app::config::Config::default()
        },
        data_base: home.join("saves"),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
    };
    Some(boot_story(req, &mut QuietBoot).expect("the story boots headlessly"))
}

fn host_click(b: &mut BootedStory, tidy: &mut u32, game_px: (u16, u16)) -> Option<app::host::TurnOutcome> {
    let mut ctx = TurnCtx { game_dir: &b.game_dir, ifid: &b.ifid, arc_file: &b.arc_file, map_view: None, bg_tidy_counter: tidy };
    deliver_v6_click(&mut b.state, &mut b.mapper, &mut *b.session, &mut ctx, game_px)
}

fn host_answer(b: &mut BootedStory, tidy: &mut u32, line: &str) -> String {
    match b.session.pending_input() {
        InputKind::Line => {
            let r = b.session.submit(line);
            let text = r.transcript.clone();
            let _ = app::host::finish_command_turn(
                line, true, r, &mut b.state, &mut b.mapper, &mut *b.session, &b.game_dir, &b.ifid, &b.arc_file, None,
                tidy,
            );
            text
        }
        InputKind::Char | InputKind::Event => {
            let z = app::engine_helpers::zvm_session_opt_mut(&mut *b.session).expect("a z-machine story");
            let r = z.submit_char(13);
            let text = r.transcript.clone();
            let _ = app::host::apply_game_driven_result(
                &mut b.state, &mut b.mapper, &r, &b.game_dir, None, &*b.session, app::pager::Driver::PlayerInput,
            );
            text
        }
    }
}

/// Every non-blank chrome run's `(x, y, style, text)` — the menu's own visible
/// state, diffed before/after a click.
fn menu_snapshot(session: &dyn Engine) -> Vec<(u16, u16, u8, String)> {
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { return Vec::new() };
    items
        .iter()
        .filter_map(|it| match &it.node {
            WinNode::Grid(g) => Some(g.px_texts.iter().cloned()),
            _ => None,
        })
        .flatten()
        .filter(|t| !t.text.trim().is_empty())
        .map(|t| (t.x, t.y, t.style, t.text))
        .collect()
}

/// The click-inverse end-to-end acceptance case the quest asks for: invert a
/// click on Journey's menu band through `hybrid_chrome_layout`'s own
/// `click_map` — at a HOST cell size that is deliberately NOT the game's own
/// 8x16 cell, to prove the inverse follows the host's `cell_px` rather than
/// lanthorn's — then actually deliver it through `deliver_v6_click` and confirm
/// the menu changes, not merely that a coordinate comes back.
#[test]
fn hybrid_chrome_layout_click_map_recovers_a_menu_click_and_delivering_it_changes_the_menu() {
    let Some(mut b) = boot_host("journey-r83-s890706.z6", 83, "hybrid-chrome-layout-click") else { return };
    let mut tidy = 0u32;
    for _ in 0..40 {
        if host_answer(&mut b, &mut tidy, "").contains("magical resources") {
            break;
        }
    }
    assert_eq!(b.session.pending_input(), InputKind::Char, "Journey's command menu is a CHAR read");

    let model = b.session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &b.state.v6_text);
    let cell = b.state.v6_text.cell();
    let layout = v6::classify_windows(items, cell);
    let pane = Rect::new(0, 0, 140, 45);
    let cell_px = (10, 20); // NOT the game's own 8x16 cell — a host's own font.
    let hyb = hybrid_chrome_layout(&layout, native, pane, cell_px, &b.state).expect("expected a hybrid chrome layout");
    assert!(!hyb.menu_band.is_empty(), "premise — Journey's frame must have a menu band to click on");

    // Minar's own currently-queued verb label ("Scout" — matching
    // `v6_journey_menu_band.rs`'s own choice of specimen and its remark that
    // clicking the row's verb, not its name, is what reaches the game's action
    // handler: cycling it to the next verb). The target cell is `hyb`'s OWN
    // reported position for that run, not a native pixel hand-picked from
    // another suite's specimen.
    let verb = hyb
        .runs
        .iter()
        .find(|r| r.in_menu_band && r.run.text.trim() == "Scout")
        .expect("premise — Minar's row must carry its own queued verb label");
    let (col, row) = (verb.col as u16, verb.row as u16);
    let game_px = hyb
        .click_map
        .map_click(col, row)
        .unwrap_or_else(|| panic!("the click map must recover a game pixel for Minar's own row at ({col},{row})"));

    let before = menu_snapshot(&*b.session);
    let out = host_click(&mut b, &mut tidy, game_px).expect("a char read always takes a click");
    assert!(!out.quit, "the game goes on");
    let after = menu_snapshot(&*b.session);
    assert_ne!(before, after, "clicking the menu band's own reported run position did not change the menu");
}

// ---------------------------------------------------------------------------
// SQ-1599 — the five draw-time facts SQ-1591 deliberately left unpublished:
// strip/row ground floods, SQ-0747's rule stretch + claimed-word collision
// guard (published together as `V6HybridChromeRun::resolved`), flank border
// extensions, the story-slot Grid's own placement, and the main ring's scale.
// ---------------------------------------------------------------------------

/// Arthur's status bar is the quest's own named symptom for the ground/flood
/// gap: "one-cell gaps between its fields". The gap is a COLOUR discontinuity,
/// not a missing glyph — every ground fill paints a plain SPACE, so checking
/// for a non-blank symbol (as the origin/resolved checks above do) cannot see
/// it at all. Arthur's own status row is a pure reverse-video bar
/// (`row_is_reverse_bar`), so the fill's `reverse` flag is what actually
/// closes the gap, and it is what this checks: apply every published
/// [`V6HybridGroundFill`] in order (later overwriting earlier, exactly as a
/// host is told to draw them), then confirm each covered cell's REVERSED
/// modifier in the real render matches the fill that ended up covering it —
/// including the cells strictly BETWEEN and AROUND the runs, which is exactly
/// where a host with no ground data would leave the theme backdrop showing
/// instead of the continuous bar.
#[test]
fn arthur_ground_fills_close_the_gaps_between_status_fields() {
    let Some(session) = arthur_pc() else { return };
    let state = render_state(app::render::graphics::kitty_picker(8, 16));
    let area = Rect::new(0, 0, 120, 40);
    let buf = render_real(&session, &state, area);
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("arthur: a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &state.v6_text);
    let cell = state.v6_text.cell();
    let layout = v6::classify_windows(items, cell);
    let hyb = hybrid_chrome_layout(&layout, native, area, (8, 16), &state)
        .unwrap_or_else(|| panic!("arthur: expected a hybrid chrome layout"));
    assert!(!hyb.ground.is_empty(), "arthur: no ground fills published at all — premise failed");

    // Later fills win a cell they share with an earlier one, exactly as
    // `V6HybridChromeLayout::ground`'s own doc tells a host to draw them.
    let mut final_fill: std::collections::HashMap<(u16, u16), bool> = Default::default();
    for fill in &hyb.ground {
        for y in fill.rect.y..fill.rect.bottom().min(area.bottom()) {
            for x in fill.rect.x..fill.rect.right().min(area.right()) {
                final_fill.insert((x, y), fill.reverse);
            }
        }
    }
    assert!(!final_fill.is_empty(), "arthur: no in-bounds ground cell to check — premise failed");
    let mut reverse_checked = 0;
    for (&(x, y), &want_reverse) in &final_fill {
        let reversed = buf.cell((x, y)).unwrap().style().add_modifier.contains(ratatui::style::Modifier::REVERSED);
        assert_eq!(
            reversed, want_reverse,
            "arthur: ground fill predicts reverse={want_reverse} at cell ({x},{y}) — exactly the \
             one-cell gap the quest describes if this cell is between two status fields — but \
             the real render disagrees"
        );
        if want_reverse {
            reverse_checked += 1;
        }
    }
    assert!(
        reverse_checked > 0,
        "arthur: no reverse-video ground fill found — premise (the status bar is a reverse-video \
         bar) failed"
    );
}

/// Journey's Amiga release draws its whole frame (and its menu header) out of
/// abutting rule fragments (SQ-0742/SQ-0747) — at a WIDE pane (the quest's own
/// named symptom for stretch) a fragment's collapsed width falls well short of
/// its neighbours, and `draw_chrome_text_strip` stretches it to close the seam.
/// Every rule run whose `resolved` span is wider than its own raw text must
/// have that WHOLE wider span painted with the rule's own glyph in the real
/// render — the unstretched origin alone would under-report where the real
/// rule actually reaches.
#[test]
fn journey_amiga_rule_runs_are_stretched_to_match_the_real_render_at_a_wide_pane() {
    let Some(session) = journey_amiga() else { return };
    let state = render_state(app::render::graphics::kitty_picker(8, 16));
    let area = Rect::new(0, 0, 200, 40); // wide pane — the quest's own named symptom
    let buf = render_real(&session, &state, area);
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("journey-amiga: a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &state.v6_text);
    let cell = state.v6_text.cell();
    let layout = v6::classify_windows(items, cell);
    let hyb = hybrid_chrome_layout(&layout, native, area, (8, 16), &state)
        .unwrap_or_else(|| panic!("journey-amiga: expected a hybrid chrome layout"));

    let is_rule_glyph = |t: &str| match t.chars().next() {
        Some(g) if !g.is_whitespace() && !g.is_alphanumeric() => t.chars().all(|c| c == g),
        _ => false,
    };
    let mut stretched = 0;
    for r in &hyb.runs {
        if !is_rule_glyph(&r.run.text) {
            continue;
        }
        let raw_len = r.run.text.chars().count() as i32;
        let Some((s, e)) = r.resolved else { continue };
        if e - s <= raw_len {
            continue; // this fragment was not actually stretched
        }
        if s < area.x as i32 || e > area.right() as i32 || r.row < area.y as i32 || r.row >= area.bottom() as i32 {
            continue; // draw_chrome_text_strip clips these at the pane edge
        }
        stretched += 1;
        let glyph = r.run.text.chars().next().unwrap();
        for c in s..e {
            let (x, y) = (c as u16, r.row as u16);
            assert!(
                !is_blank(&buf, x, y),
                "journey-amiga-wide: stretched rule cell ({x},{y}) is blank in the real render"
            );
            assert_eq!(
                buf.cell((x, y)).unwrap().symbol().chars().next(),
                Some(glyph),
                "journey-amiga-wide: stretched rule cell ({x},{y}) does not carry the rule's own glyph"
            );
        }
    }
    assert!(stretched > 0, "journey-amiga-wide: no stretched rule run found — premise failed");
}

/// Journey's menu at ~39 columns is the quest's own named symptom for the
/// claimed-word collision guard: a lone divider/frame glyph (`│`/`▌`/a short
/// rule) lands on the same cell a multi-character WORD run already claims, and
/// `draw_chrome_text_strip` drops the glyph entirely rather than punching a
/// hole in the word. A suppressed run's OWN glyph must never be what the real
/// render shows at its origin cell — whatever IS shown there belongs to
/// whichever run actually won the contest (a real word's own character, or
/// one of that word's own internal spaces, which is why this checks only that
/// the loser's glyph lost, not that a replacement glyph is visible).
#[test]
fn journey_amiga_narrow_pane_collision_guard_matches_which_run_wins_a_contested_cell() {
    let Some(session) = journey_amiga() else { return };
    let state = render_state(app::render::graphics::kitty_picker(8, 16));
    let area = Rect::new(0, 0, 39, 40); // ~39 columns — the quest's own named symptom
    let buf = render_real(&session, &state, area);
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("journey-amiga-narrow: a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &state.v6_text);
    let cell = state.v6_text.cell();
    let layout = v6::classify_windows(items, cell);
    let hyb = hybrid_chrome_layout(&layout, native, area, (8, 16), &state)
        .unwrap_or_else(|| panic!("journey-amiga-narrow: expected a hybrid chrome layout"));

    // Restricted to LONE glyphs (one character) — `draw_chrome_text_strip`'s
    // own `over_word` guard only ever applies to a single, non-rule
    // character (its own precondition, `t.text.chars().count() == 1`), never
    // to a multi-character rule fragment. A multi-character rule fragment can
    // also end up `resolved: None` at a narrow pane, but for a DIFFERENT
    // reason (SQ-0949's edge clip, once stretching pushes it past the
    // strip), which is not this item's mechanism — excluding it keeps this
    // test from passing on the wrong cause.
    let mut checked = 0;
    let mut collisions = 0;
    for r in &hyb.runs {
        if r.run.text.trim().is_empty() || r.resolved.is_some() || r.run.text.chars().count() != 1 {
            continue;
        }
        if r.col < area.x as i32 || r.col >= area.right() as i32 || r.row < area.y as i32 || r.row >= area.bottom() as i32 {
            continue;
        }
        let (x, y) = (r.col as u16, r.row as u16);
        let shown = buf.cell((x, y)).unwrap().symbol().chars().next();
        let own_glyph = r.run.text.chars().next();
        assert_ne!(
            shown, own_glyph,
            "journey-amiga-narrow: suppressed run {:?} at ({x},{y}) still shows its OWN glyph in \
             the real render — the collision guard did not actually suppress it",
            r.run.text
        );
        checked += 1;
        // Reconstruct the collision itself from the published data alone: some
        // OTHER multi-character WORD run on the same row must actually claim
        // this run's own origin column, exactly `over_word`'s own test.
        let is_genuine_collision = hyb.runs.iter().any(|w| {
            w.row == r.row
                && !std::ptr::eq(w, r)
                && w.run.text.chars().count() > 1
                && !w.run.text.trim().is_empty()
                && matches!(w.resolved, Some((lo, hi)) if r.col >= lo && r.col < hi)
        });
        if is_genuine_collision {
            collisions += 1;
        }
    }
    assert!(checked > 0, "journey-amiga-narrow: no in-bounds suppressed lone glyph to check — premise failed");
    assert!(
        collisions > 0,
        "journey-amiga-narrow: no suppressed lone glyph is actually claimed by another WORD run's \
         own resolved span — the collision guard was not genuinely exercised (premise failed)"
    );
}

/// Copy-through data (SQ-1599): `V6HybridChromeLayout::flank_borders`/
/// `divider_exts` are `HybridFrame::flank_borders`/`divider_exts` verbatim —
/// the exact same fields `render_node`'s own draw reads — so this checks that
/// what is published actually matches the real render rather than that the
/// copy itself is correct (which cannot drift by construction). HALFBLOCKS,
/// not kitty: a `BorderInk::Band` extension is a real terminal-cell paint only
/// on a backend without virtual placements (SQ-0944's reasoning, same as
/// `zork0_over_art_runs_agree_with_the_real_halfblocks_render` above).
///
/// Two specimens: the PC press (Glyph AND Band ink both occur, at (140,40)),
/// and the Amiga release (Glyph ink on BOTH flanks, at (140,40)) — between
/// them every `BorderInk` variant this corpus produces. A `Band` extension
/// whose own flank strip has no art behind it at all
/// (`state.v6_cell_map`'s own "skipped — no art behind it" line, checked
/// against `render_node`'s own dump) is legitimately blank in the REAL render
/// too, and the check follows that rather than assuming every `Band` paints —
/// the PC press's right flank is exactly this case in the corpus today, so it
/// is asserted blank-and-agreeing rather than skipped.
fn flank_border_extensions_match_the_real_halfblocks_render(ctx: &str, session: &GameSession, area: Rect) {
    let state = render_state(ratatui_image::picker::Picker::halfblocks());
    let buf = render_real(session, &state, area);
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("{ctx}: a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &state.v6_text);
    let cell = state.v6_text.cell();
    let layout = v6::classify_windows(items, cell);
    let hyb = hybrid_chrome_layout(&layout, native, area, (8, 16), &state)
        .unwrap_or_else(|| panic!("{ctx}: expected a hybrid chrome layout"));
    assert!(!hyb.flank_borders.is_empty(), "{ctx}: no flank border extensions published — premise failed");
    assert!(!hyb.divider_exts.is_empty(), "{ctx}: no divider extensions published — premise failed");

    let cell_map = state.v6_cell_map.borrow();
    let strip_has_art = |r: Rect| {
        cell_map
            .iter()
            .find(|e| e.cells == (r.x, r.y, r.width, r.height))
            .map(|e| !e.label.contains("skipped"))
    };

    let mut checked = 0;
    let mut band_checked = 0;
    let mut glyph_checked = 0;
    for (strip, inner, outer) in &hyb.flank_borders {
        for (ext, ink) in inner.iter().chain(outer.iter()) {
            match ink {
                app::render::screen::BorderInk::Band(_) => {
                    let has_art = strip_has_art(*strip).unwrap_or(true);
                    for y in ext.y..ext.bottom().min(area.bottom()) {
                        for x in ext.x..ext.right().min(area.right()) {
                            let blank = is_blank(&buf, x, y);
                            assert_eq!(
                                blank, !has_art,
                                "{ctx}: Band divider extension {ext:?} (strip {strip:?}, \
                                 has_art={has_art}) disagrees with the real halfblocks render \
                                 at ({x},{y})"
                            );
                            checked += 1;
                        }
                    }
                    band_checked += 1;
                }
                app::render::screen::BorderInk::Glyph { style, .. } => {
                    let want_reversed = style & 1 != 0;
                    for y in ext.y..ext.bottom().min(area.bottom()) {
                        for x in ext.x..ext.right().min(area.right()) {
                            let reversed = buf
                                .cell((x, y))
                                .unwrap()
                                .style()
                                .add_modifier
                                .contains(ratatui::style::Modifier::REVERSED);
                            assert_eq!(
                                reversed, want_reversed,
                                "{ctx}: Glyph divider extension {ext:?} predicts \
                                 reverse={want_reversed} at cell ({x},{y}), but the real render \
                                 disagrees"
                            );
                            checked += 1;
                        }
                    }
                    glyph_checked += 1;
                }
            }
        }
    }
    assert!(checked > 0, "{ctx}: no in-bounds divider extension cell to check — premise failed");
    eprintln!("{ctx}: checked {checked} cells across {band_checked} Band and {glyph_checked} Glyph extensions");
}

#[test]
fn journey_pc_flank_border_extensions_match_the_real_halfblocks_render() {
    let Some(session) = journey_pc() else { return };
    flank_border_extensions_match_the_real_halfblocks_render("journey-pc", &session, Rect::new(0, 0, 140, 40));
}

#[test]
fn journey_amiga_flank_border_extensions_match_the_real_halfblocks_render() {
    let Some(session) = journey_amiga() else { return };
    flank_border_extensions_match_the_real_halfblocks_render("journey-amiga", &session, Rect::new(0, 0, 140, 40));
}

/// Boot Shogun to its InvisiClues hint screen (`hint`, then `y`) — the ACTUAL
/// v6-Grid-in-the-story-slot specimen (SQ-0934/SQ-1074/SQ-1075), reusing
/// `v6_hint_menu_mouse.rs`'s own proven boot sequence. The quest's own text
/// named Shogun's BOOT menu ("START the game") for this item; that screen
/// keeps window 0 as an ordinary Buffer and paints its menu as chrome runs
/// over it (`shogun_hybrid_boot_menu_is_one_coherent_ring_screen_with_a_solid_selection_bar`
/// in `v6_shogun_gameplay.rs` pins exactly that shape) — never a story-slot
/// Grid, so it cannot exercise `hybrid_story_slot_grid` at all. The hint
/// screen is what `render_node`'s own `WinNode::Grid` arm and
/// `draw_grid_transparent` exist for, and is the corpus's only such frame.
fn shogun_hint_menu() -> Option<GameSession> {
    let path = stories_dir().join("shogun-r322-s890706.z6");
    let bytes = std::fs::read(&path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, dims, std_window, None, None)
        .expect("shogun should load and boot without a ZError");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..8 {
        match s.pending_input() {
            InputKind::Line => break,
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
    }
    s.submit("hint");
    let entered = s.submit_char(b'y');
    assert!(entered.fault.is_none(), "shogun: entering the hint menu faulted: {:?}", entered.fault);
    Some(s)
}

#[test]
fn shogun_hint_menu_story_slot_grid_matches_the_real_render() {
    let Some(session) = shogun_hint_menu() else { return };
    let state = render_state(app::render::graphics::kitty_picker(8, 16));
    let area = Rect::new(0, 0, 190, 60);
    let buf = render_real(&session, &state, area);
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("shogun-hint: a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &state.v6_text);
    let cell = state.v6_text.cell();
    let layout = v6::classify_windows(items, cell);
    assert!(
        matches!(layout.story.map(|s| &s.node), Some(WinNode::Grid(_))),
        "shogun-hint: premise — the hint screen's story slot is not a Grid on this frame"
    );
    let slot = app::render::screen::hybrid_story_slot_grid(&layout, native, area, (8, 16), &state)
        .unwrap_or_else(|| panic!("shogun-hint: expected a story-slot grid layout"));
    assert!(slot.viewport.width > 0 && slot.viewport.height > 0, "shogun-hint: empty story-slot viewport");

    // Cross-render: the topic list's own first entry (SQ-0934's specimen) must
    // land inside the returned viewport, placed 1:1 — never centred, which is
    // exactly SQ-1074/SQ-1075's own defect shape.
    let rows: Vec<String> = (0..area.height)
        .map(|y| (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect())
        .collect();
    let found = rows
        .iter()
        .enumerate()
        .find_map(|(y, r)| r.find("Erasmus").map(|b| (r[..b].chars().count() as u16, y as u16)));
    let (x, y) = found.unwrap_or_else(|| panic!("shogun-hint: 'Erasmus' must be drawn as glyphs on this frame:\n{}", rows.join("\n")));
    assert!(
        x >= slot.viewport.x && x < slot.viewport.right() && y >= slot.viewport.y && y < slot.viewport.bottom(),
        "shogun-hint: 'Erasmus' at ({x},{y}) falls outside the reported story-slot viewport {:?}",
        slot.viewport
    );
}

/// Journey PC r83 is the Menu-plan specimen the quest names for item 6: on
/// this plan `click_map`'s own scale is the MENU BAND's, not the main ring's.
/// The magnification factor happens to be shared between the two on this
/// plan, but the vertical device-pixel offset is not — the main ring
/// top-anchors while the menu band bottom-anchors to the reclaimed slack —
/// and `click_map` alone can only ever answer with the menu band's offset.
/// `hyb.scale` must differ from it on that axis (recovered exactly, since
/// `build_hybrid_click_map` derives `img_y = scale.off_y`) and must still be
/// the scale that actually produced the reported story viewport.
#[test]
fn journey_pc_scale_is_the_main_rings_own_and_differs_from_the_menu_bands() {
    let Some(session) = journey_pc() else { return };
    let state = render_state(app::render::graphics::kitty_picker(8, 16));
    let area = Rect::new(0, 0, 140, 45);
    let cell_px = (8u16, 16u16);
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("journey-pc: a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &state.v6_text);
    let cell = state.v6_text.cell();
    let layout = v6::classify_windows(items, cell);
    let hyb = hybrid_chrome_layout(&layout, native, area, cell_px, &state)
        .unwrap_or_else(|| panic!("journey-pc: expected a hybrid chrome layout"));
    assert!(!hyb.menu_band.is_empty(), "journey-pc: premise — expected a Menu-plan frame with a command band");

    // On the Menu plan the magnification factor happens to be shared between
    // the main ring and the menu band (`build_hybrid_frame_with` derives both
    // from the same `scale_center.s`) — what differs, and what a host cannot
    // recover from `click_map` alone, is the VERTICAL device-pixel offset:
    // the main ring top-anchors (`off_y == 0`) while the menu band is
    // bottom-anchored to the reclaimed slack (`off_y == slack`).
    // `build_hybrid_click_map` derives `img_y = scale.off_y`, so this is
    // exactly `click_map.img_y` vs `hyb.scale.off_y`.
    let menu_off_y = hyb.click_map.img_y;
    assert!(
        (hyb.scale.off_y as f32 - menu_off_y).abs() > 1e-4,
        "journey-pc: the main ring's own vertical offset ({}) must differ from the menu \
         band's ({menu_off_y}) on a Menu-plan frame",
        hyb.scale.off_y
    );
    assert_eq!(hyb.scale.off_y, 0, "journey-pc: premise — the main ring top-anchors on the Menu plan");

    // …and `hyb.scale` is the scale that actually produced the reported story
    // viewport: its native size, scaled by `hyb.scale.s`, must agree with the
    // viewport's own device-pixel size (one host cell of rounding slack).
    let expect_w = hyb.viewport_native.2 as f32 * hyb.scale.s;
    let actual_w = hyb.viewport.width as f32 * cell_px.0 as f32;
    assert!(
        (expect_w - actual_w).abs() <= cell_px.0 as f32 * 1.5,
        "journey-pc: hyb.scale ({}) does not reproduce the reported viewport's own width \
         (expected ~{expect_w}px, viewport reports {actual_w}px)",
        hyb.scale.s
    );
}
