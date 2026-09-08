//! **In hybrid, never rasterise what the game printed as a character.** SQ-0750,
//! and the hole its position-based classification left at the frame's corner,
//! SQ-0747.
//!
//! Both quests are one mechanism, which is why they are one file. The hybrid ring
//! carves the chrome into strips and classified a SIDE band by where it sat — every
//! band narrower than the pane became one `Art` strip, and each of its border columns
//! was carried down the reclaimed gap as a one-native-row crop stretched into a
//! narrow band. Journey's frame is drawn as TEXT under both interpreter profiles
//! (box-drawing glyphs on the Amiga, reverse-video spaces on the IBM PC), so its four
//! vertical rules were uploaded as their own RGBA bitmaps — measured off the release
//! floppy at a 117x64 terminal: 8x900px each for the columns at 1, 47 and 114, 16x900
//! for the one at 115, about 192 KB of raw RGBA per frame that redraws them, to render
//! what is 200 `│` characters.
//!
//! The decisive observation was that the SAME rule changed medium partway down the
//! screen: column 1 was a kitty placeholder for rows 3..52 and a font glyph for rows
//! 53..59, where it crosses the menu strip. So this was never "Journey draws verticals
//! as pictures" — the text path already drew this exact rule correctly, in the same
//! frame, for seven rows. It needed to cover the other fifty.
//!
//! And where the frame's top rule should have met those flanks there was a one-row
//! hole: `story_viewport_box` quantizes the story's top edge OUTWARD to a whole cell
//! while the top band ends at that quantized row, so a sliver row fell between the two,
//! carried no runs and no art, classified Empty → Art → skipped, and was never written
//! at all (terminal row 2 of the same capture, across all 115 columns).
//!
//! ## What is asserted here, and why it is a CONTENT test
//!
//! The oracle is the game's own paint runs, never the pane geometry and never the
//! interpreter profile:
//!
//!   * (a) every border character the game printed beside the story box reaches the
//!     terminal AS that character, in the cell its own run maps to, on every row from
//!     the frame's top rule down to the menu — so the box is one medium all the way
//!     round instead of font glyphs on top and a resampled bitmap down the sides;
//!   * (b) no row between the frame's top rule and the story's first row is left
//!     unwritten — the flanks claim the gap the quantization opens;
//!   * (c) and the reserved case: a side column that is genuine ARTWORK — Zork Zero's,
//!     Shogun's and Arthur's frames — stays a bitmap, because the runs cannot account
//!     for its pixels. Getting that wrong is a worse regression than the bug.
//!
//! Swept across pane widths (the SQ-0742 lineage repeatedly turns up defects that
//! exist only at some widths), at two pane origins, and in BOTH `honor_game_colours`
//! modes per the project's colour-render convention.

use std::path::PathBuf;

use app::engine::{Engine, PxText, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

/// A rect as `/dump-windows` records it: `(x, y, w, h)`.
type Quad = (u16, u16, u16, u16);

/// Pane widths swept. 80 is the game's own column count, where the placement rates
/// coincide and a defect can hide; the rest do not coincide, and 115 is the story pane
/// of the 117x64 terminal the defect was captured at.
const WIDTHS: [u16; 6] = [80, 96, 110, 115, 138, 150];

/// SQ-0779: panes with NO letterbox slack.
///
/// The reclaim plans all need vertical slack to reclaim; a pane whose rows are at or
/// below the scaled native height has none. For a 640x400 native screen at an 8x18
/// cell that is `18·rows <= 5·cols` — an ASPECT threshold, not a width one, which is
/// why the report's 121x36 terminal (a 119x33 pane) showed the defect while its 117x64
/// control did not. Two panes per width: one right at the boundary and one well inside
/// it.
///
/// **SQ-0830 changed what plan these get, and not what they must look like.**
/// `hybrid_bottom_plan` used to return `Letterbox` here before it looked at anything
/// else, which is precisely the defect that quest fixed: a command menu is a fact
/// about the FRAME and slack is a fact about the PANE, so the menu is recognised at
/// every aspect now and slack gates only the reclaim. These panes are therefore `menu`
/// plans with nothing to reclaim. The requirements below are unchanged — a border the
/// game printed is a character on screen, and no artwork stands in its column — and
/// each case still asserts the plan it actually got, so the sweep cannot quietly drift
/// out of the regime it was written for.
const SHORT_PANES: [(u16, u16); 10] =
    [(96, 26), (96, 20), (115, 31), (115, 24), (119, 33), (119, 28), (138, 38), (138, 30), (150, 41), (150, 32)];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a v6 story exactly as `startup.rs` does — the profile comes from the medium
/// unless one is named — and drive `turns` "keep going" turns. `None` (with a SKIP
/// note) when the gitignored fixture is absent.
fn boot(file: &str, profile: Option<InterpreterProfile>, turns: usize) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let loaded = match app::hints::load_mounted_story(&path) {
        Ok((l, _)) => l,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            return None;
        }
    };
    let profile = profile.unwrap_or_else(|| InterpreterProfile::resolve(&path, None, None, None));
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut s = GameSession::new_with_trace(
        loaded.bytes().to_vec(),
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .unwrap_or_else(|e| panic!("{file}: should boot without a ZError: {e:?}"));
    // SQ-1393: the machine's own colour table. `new_with_trace` is the
    // no-machine door and presents §8.3.1's own, so a harness that boots a
    // press states the press's table here.
    s.machine.set_palette(profile.palette());
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..turns {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        // Arthur asks whether to restore a saved game before its story starts; a short
        // key list bounces off that prompt and looks like rejected input.
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
        if r.transcript.contains("Praxix") || r.transcript.contains("magical resources") {
            break;
        }
    }
    Some(s)
}

/// A hybrid render at real kitty-ish cell metrics (8x18) with the transcript fed in.
/// `Picker::halfblocks()` reports a 1x2 cell — a layout regime that reproduces no scale
/// defect at all — so the sweep runs at a plausible font cell (the SQ-0548 lesson).
#[allow(deprecated)]
fn render_pane(
    model: &app::engine::ScreenModel,
    honor: bool,
    pane: Quad,
    transcript: &str,
) -> (app::state::AppState, Rect, Buffer) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    for line in transcript.lines() {
        state.push_transcript(line);
    }
    let area = Rect::new(pane.0, pane.1, pane.2, pane.3);
    let mut buf = Buffer::empty(Rect::new(0, 0, area.right() + 1, area.bottom() + 1));
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    (state, area, buf)
}

/// Every `/dump-windows` record whose label starts with `prefix`: `(label, cells)`.
fn records(state: &app::state::AppState, prefix: &str) -> Vec<(String, Quad)> {
    state
        .v6_cell_map
        .borrow()
        .iter()
        .filter(|e| e.label.starts_with(prefix))
        .map(|e| (e.label.clone(), e.cells))
        .collect()
}

/// The same, keeping each record's NATIVE quad — for a stamped border that is the
/// character cell the extension stands for (SQ-0779).
fn records_native(state: &app::state::AppState, prefix: &str) -> Vec<(String, Quad, Quad)> {
    state
        .v6_cell_map
        .borrow()
        .iter()
        .filter(|e| e.label.starts_with(prefix))
        .map(|e| (e.label.clone(), e.cells, e.native))
        .collect()
}

/// Every stamped border on this frame: `(extension rect, native `[x0, x1)` of the
/// character cell it draws)`.
fn glyph_borders(state: &app::state::AppState) -> Vec<(Quad, (u32, u32))> {
    let mut v = records_native(state, "flank-divider");
    v.extend(records_native(state, "flank-border"));
    v.into_iter()
        .filter(|(label, _, _)| label.contains("glyph"))
        .map(|(_, cells, native)| (cells, (native.0 as u32, native.0 as u32 + native.2 as u32)))
        .collect()
}

fn viewport_of(state: &app::state::AppState) -> Quad {
    state
        .v6_cell_map
        .borrow()
        .iter()
        .find(|e| e.label == "viewport")
        .map(|e| e.cells)
        .expect("a hybrid ring frame records its story viewport")
}

/// The story window's native pixel box, from the model — the frame is what stands
/// beside it, so this is what "beside the story box" is measured against.
fn story_box(model: &app::engine::ScreenModel) -> (u32, u32, u32, u32) {
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let s = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT).story.expect("story window");
    (s.x_px as u32, s.y_px as u32, s.w_px as u32, s.h_px as u32)
}

/// Every paint run the game has on screen.
fn chrome_runs(model: &app::engine::ScreenModel) -> Vec<PxText> {
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    items
        .iter()
        .filter_map(|it| match &it.node {
            WinNode::Grid(g) => Some(g.px_texts.iter().cloned()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// The frame's SIDE-RULE runs: single-character runs the game printed beside the story
/// box (never above or below it) on the story's own middle row, one per rule. This is
/// the whole oracle — the characters the game itself put in those columns.
fn side_rule_runs(model: &app::engine::ScreenModel) -> Vec<PxText> {
    let (sx, sy, sw, sh) = story_box(model);
    let mid = sy + sh / 2;
    chrome_runs(model)
        .into_iter()
        .filter(|t| {
            let py = t.y.max(1) as u32 - 1;
            let px0 = t.x.max(1) as u32 - 1;
            let w = t.text.chars().count().max(1) as u32 * 8;
            (py..py + 16).contains(&mid) && (px0 + w <= sx || px0 >= sx + sw)
        })
        .collect()
}

/// Where a run's first character lands, through the ring's own letterbox scale.
/// Recomputed from `uniform_scale` rather than read off the `scale` dump record,
/// which rounds to two decimals and drifts a column at some pane widths.
fn run_col(t: &PxText, model: &app::engine::ScreenModel, area: Rect) -> u16 {
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let (cw, ch) = (8u32, 18u32);
    let s = app::render::v6_layout::uniform_scale(native, (area.width as u32 * cw, area.height as u32 * ch));
    let px = t.x.max(1) as f32 - 1.0;
    area.x + ((s.off_x as f32 + px * s.s) / cw as f32).round() as u16
}

/// Did this cell come out as the character `ch`, drawn as the game's own text? A
/// reverse-video SPACE is a solid block whose ink is the REVERSED modifier, so it is
/// the character AND the modifier that have to agree.
fn holds(buf: &Buffer, x: u16, y: u16, t: &PxText) -> bool {
    let Some(c) = buf.cell((x, y)) else { return false };
    let want = t.text.chars().next().unwrap_or(' ');
    let reversed = c.style().add_modifier.contains(Modifier::REVERSED);
    c.symbol().chars().next().unwrap_or(' ') == want && reversed == (t.style & 1 != 0)
}

/// SQ-0779: a stamped border's extension is the character's whole native text CELL,
/// which is more than one terminal column wherever the letterbox scale is more than one
/// column per native cell. What must be true of one of its rows:
///
///   * every cell of the span carries the run's own reverse state — the cell's ground,
///     which is what the picture beside it must not be standing on; and
///   * the character stands in exactly ONE of those columns. Stamping it across the
///     span would be SQ-0750's doubled rule, which is the regression this guards.
///
/// A border the game drew as a reverse-video SPACE has no visible glyph at all, so its
/// whole span is that solid ground and every column of it counts.
fn stamped_once(buf: &Buffer, ext: Quad, y: u16, t: &PxText) -> Result<(), String> {
    let want = t.text.chars().next().unwrap_or(' ');
    let cells: Vec<String> =
        (ext.0..ext.0 + ext.2).map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default()).collect();
    let hits = (ext.0..ext.0 + ext.2).filter(|&x| holds(buf, x, y, t)).count();
    let want_hits = if want.is_whitespace() { ext.2 as usize } else { 1 };
    if hits != want_hits {
        return Err(format!(
            "row {y} of the rule's span {ext:?} holds {want:?} in {hits} column(s), not {want_hits} \
             — the span is the character's own native cell, and the character stands in one column \
             of it. Cells: {cells:?}"
        ));
    }
    let rev = t.style & 1 != 0;
    for (i, x) in (ext.0..ext.0 + ext.2).enumerate() {
        let Some(c) = buf.cell((x, y)) else { return Err(format!("row {y} column {x} is off the buffer")) };
        if c.style().add_modifier.contains(Modifier::REVERSED) != rev {
            return Err(format!(
                "row {y} column {x} of the rule's span {ext:?} is not the run's own ground (reverse \
                 should be {rev}). Cells: {cells:?} (index {i})"
            ));
        }
    }
    Ok(())
}

// ── (a) + (b): the frame is one medium, all the way round, with no hole at its corner ──

/// SQ-0750 / SQ-0747 — Journey, on the release the report was captured off and on the
/// bare story file, under both interpreter profiles.
///
/// Every side rule the game PRINTED must be in the buffer as that character, in its own
/// column, on every row from the frame's top rule down to the menu. Before the fix the
/// rows below the top strip were an uploaded bitmap of the same character and the cell
/// buffer held nothing there at all; the row where the top rule meets the flanks was
/// written by nothing whatsoever.
///
/// Each of the ring's flank-border columns is checked against the game's own runs, in
/// both directions: a column the runs DO account for must have reached the screen as
/// that character, in every cell of its span; a column that stayed a bitmap must be one
/// no run covers (Journey's release 83 under the IBM PC profile prints one border run
/// and no right-hand rule at all, so its right flank is pixels the runs cannot explain
/// and correctly keeps the band).
///
/// FALSIFY by returning `None` from the `text_border` test in `flank_border_extension`
/// (so every column goes back to a stretched band): every case fails with
/// `the frame's side rule at column 47 came out as a BITMAP … a run the game printed
/// covers it`.
#[test]
fn journeys_frame_side_rules_are_the_characters_the_game_printed() {
    for (file, profile, want_glyphs) in [
        ("Journey - The Quest Begins.adf", None, 3),
        ("journey-r83-s890706.z6", Some(InterpreterProfile::Amiga), 3),
        ("journey-r83-s890706.z6", Some(InterpreterProfile::IbmPc), 1),
    ] {
        let Some(mut session) = boot(file, profile, 40) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        let rules = side_rule_runs(&model);
        assert!(
            !rules.is_empty(),
            "{file} {profile:?}: Journey prints its frame's side rules as characters — the whole \
             premise of this case"
        );
        for honor in [true, false] {
            for pane in WIDTHS.iter().flat_map(|&w| [(0, 0, w, 61), (1, 1, w, 61), (1, 1, w, 71)]) {
                let (state, area, buf) = render_pane(&model, honor, pane, &transcript);
                let ctx = format!("{file} {profile:?} honor={honor} pane {pane:?}");
                let mut borders = records(&state, "flank-divider");
                borders.extend(records(&state, "flank-border"));
                let mut glyphs = 0usize;
                for (label, r) in &borders {
                    // Which of the game's own side rules, if any, lands in this column?
                    // One column of slack past the SPAN, which is the character's whole
                    // native cell (SQ-0779): the ring places the rule by its run and the
                    // band by the cells its ink covers, and those can round apart.
                    let run = rules
                        .iter()
                        .find(|t| (r.0..r.0 + r.2).contains(&run_col(t, &model, area)))
                        .or_else(|| rules.iter().find(|t| run_col(t, &model, area).abs_diff(r.0) <= 1));
                    match run {
                        Some(t) => {
                            assert!(
                                label.contains("glyph"),
                                "{ctx}: the frame's side rule at column {} came out as a BITMAP \
                                 ({label}) — a run the game printed covers it ({:?} at native x \
                                 {}), so in hybrid it must be drawn as that character",
                                r.0,
                                t.text,
                                t.x
                            );
                            glyphs += 1;
                            for y in r.1..r.1 + r.3 {
                                if let Err(e) = stamped_once(&buf, *r, y, t) {
                                    panic!("{ctx}: the frame's side rule {:?}: {e}", t.text);
                                }
                            }
                        }
                        None => assert!(
                            !label.contains("glyph"),
                            "{ctx}: {label} at {r:?} was stamped as a character, but no run the \
                             game printed covers that column — the glyph path is reserved for \
                             pixels the runs account for.\nruns: {:?}",
                            rules.iter().map(|t| (t.x, t.text.clone())).collect::<Vec<_>>()
                        ),
                    }
                }
                assert_eq!(
                    glyphs, want_glyphs,
                    "{ctx}: this frame's side rules are {want_glyphs} characters the game \
                     printed, and every one of them must reach the screen as a character\n{borders:#?}"
                );
            }
        }
    }
}

/// SQ-0779 — and at a pane with NO letterbox slack, where the ring takes the
/// `Letterbox` plan and none of the reclaim machinery runs.
///
/// The user, sweeping widths over the SQ-0750 fix: *"journey breaks with a blank line
/// at top any time artwork width fills its entire allocated space (e.g. 121x36)"* and
/// *"l/r borders are broken with first scenario"*. Off the release floppy at 121x36
/// (a 119x33 pane) the picture's band was placed at columns 1..50 — spanning the
/// frame's own left rule AND the rule dividing the picture from the prose — while the
/// right-hand flank, one rule wide with no art in it, classified art-less and was
/// skipped outright. The frame had a `┌─┐` and a `└─┘` and nothing down either side.
///
/// The border extension was a Menu-plan privilege; every other plan drew the flank as
/// one band from edge to edge, borders included. The ruling that decides the fix is the
/// user's: **if a game draws a border, the artwork should not overlap it** — a LAYOUT
/// change, not a compositing one. So the two assertions are:
///
///   * no drawn ART strip may stand in a column the game printed a border rule in; and
///   * every one of those rules reaches the screen as the character the game printed,
///     on every row from the frame's top rule down to the story's last.
///
/// FALSIFY by restoring the `matches!(plan, BottomPlan::Menu)` gate on `flank_borders`
/// in `screen.rs`: every case fails with `the picture's band (1, 1, 50, 22) stands in
/// column 0, where the game printed its frame's rule "│"`.
///
/// SQ-0830 NOTE: that falsification no longer bites on **these** panes, because the
/// quest removed the route by which Journey reached them without a Menu plan — they
/// are menu plans now, so the gate it restores is satisfied and `flank_borders` is
/// produced either way. What the case still asserts is the RULING, which was never
/// about a plan: artwork does not stand in a column the game printed a rule in, and
/// every one of those rules reaches the screen as that character. Under the Menu plan
/// that is delivered by `menu_flank_panel`'s bounds (SQ-0747/0758) rather than by the
/// `glyph_borders_only` trim, and this is what proves the two agree.
#[test]
fn journeys_frame_side_rules_survive_a_pane_with_no_letterbox_slack() {
    for (file, profile, want_glyphs) in [
        ("Journey - The Quest Begins.adf", None, 3),
        ("journey-r83-s890706.z6", Some(InterpreterProfile::Amiga), 3),
        ("journey-r83-s890706.z6", Some(InterpreterProfile::IbmPc), 1),
    ] {
        let Some(mut session) = boot(file, profile, 40) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        let rules = side_rule_runs(&model);
        assert!(!rules.is_empty(), "{file} {profile:?}: Journey prints its frame's side rules as characters");
        for honor in [true, false] {
            for (w, h) in SHORT_PANES {
                let (state, area, buf) = render_pane(&model, honor, (0, 0, w, h), &transcript);
                let ctx = format!("{file} {profile:?} honor={honor} pane {w}x{h}");
                assert_eq!(
                    state.v6_ring_plan.get(),
                    "menu",
                    "{ctx}: this sweep exists to cover the no-slack regime, and since SQ-0830 a \
                     frame with a command menu is a `menu` plan at every aspect — with nothing to \
                     reclaim here, but with its flanks treated as the panels they are"
                );
                let vp = viewport_of(&state);
                // Where the picture is actually PUT, which is not the same record under
                // every plan (SQ-0830). Outside the Menu plan a flank strip IS its own
                // placement, and `glyph_borders_only` trims the strip rect off the border
                // columns — so `strip:art` is the span to test. Under the Menu plan the
                // strip is the whole flank column, the panel fill floods it, and the ART
                // goes to `menu_flank_panel`'s inset dest, recorded as `flank-art`; asking
                // `strip:art` there is asking about the panel's ground rather than the
                // picture standing on it, and the two rules bound the panel by
                // construction (SQ-0747/0758). The sibling case above makes the same
                // distinction for the same reason.
                let menu_plan = state.v6_ring_plan.get() == "menu";
                let drawn: Vec<Quad> = if menu_plan {
                    records(&state, "flank-art").into_iter().map(|(_, r)| r).filter(|r| r.2 < w).collect()
                } else {
                    records(&state, "strip:art")
                        .into_iter()
                        .filter(|(label, r)| label == "strip:art" && r.2 < w)
                        .map(|(_, r)| r)
                        .collect()
                };
                assert!(
                    !drawn.is_empty(),
                    "{ctx}: Journey's picture column is a side flank, so the frame must place it \
                     somewhere — an empty set here would pass the ruling below vacuously"
                );
                // THE RULING: artwork does not overlap a border the game draws.
                for t in &rules {
                    let col = run_col(t, &model, area);
                    for r in &drawn {
                        assert!(
                            !(r.0..r.0 + r.2).contains(&col),
                            "{ctx}: the picture's band {r:?} stands in column {col}, where the game \
                             printed its frame's rule {:?} (native x {}) — a border is not the \
                             artwork's ground to stand on, so the art's span must stop short of it",
                            t.text,
                            t.x
                        );
                    }
                }
                // …and each of those rules is on screen AS that character, for the whole
                // height of the flank: from the frame's top rule down to the story's
                // last row. (The row below that is the ring's own bottom band.)
                let mut borders = records(&state, "flank-divider");
                borders.extend(records(&state, "flank-border"));
                let top = records(&state, "strip:text").into_iter().map(|(_, r)| r.1 + r.3).min().unwrap_or(area.y);
                let mut glyphs = 0usize;
                for t in &rules {
                    let col = run_col(t, &model, area);
                    let Some((label, r)) = borders
                        .iter()
                        .find(|(_, r)| (r.0..r.0 + r.2).contains(&col))
                        .or_else(|| borders.iter().find(|(_, r)| r.0.abs_diff(col) <= 1))
                    else {
                        panic!(
                            "{ctx}: the frame's side rule {:?} (native x {}, column {col}) reaches \
                             the screen through nothing at all — no border column was resolved for \
                             it.\nstrips: {:#?}",
                            t.text,
                            t.x,
                            records(&state, "strip")
                        )
                    };
                    assert!(
                        label.contains("glyph"),
                        "{ctx}: the frame's side rule at column {} came out as a BITMAP ({label}) — \
                         a run the game printed covers it, so hybrid draws it as that character",
                        r.0
                    );
                    glyphs += 1;
                    for y in top..vp.1 + vp.3 {
                        if let Err(e) = stamped_once(&buf, *r, y, t) {
                            panic!("{ctx}: the frame's side rule {:?}: {e}", t.text);
                        }
                    }
                }
                assert_eq!(
                    glyphs, want_glyphs,
                    "{ctx}: this frame's side rules are {want_glyphs} characters the game printed, \
                     and every one of them must reach the screen as a character\n{borders:#?}"
                );
            }
        }
    }
}

/// SQ-0779, second pass — a stamped border must be out of the artwork's SOURCE CROP,
/// not merely out of its destination rect.
///
/// The first pass trimmed the picture band's destination to stop at the border's
/// column, and the user swept wider: *"there is an extra border on the left hand side
/// of the art … but not for all widths … recreate with 236x68. We end up with 3 border
/// lines on the left … the innermost (the one that shouldn't be there) is slightly
/// thicker than our standard border."*
///
/// [`app::render::graphics::GraphicsRender::draw_chrome_band`] derives a band's crop
/// from WHERE IT IS PLACED — the destination rect, mapped back through the letterbox
/// scale — so trimming the destination by whole terminal columns lands the crop's edge
/// somewhere INSIDE the border's own 8-pixel text cell. Journey release 30 inks its
/// `│` at native x 3 of the cell at x 0..8; at a 234-column pane (scale 2.925) the
/// trimmed band began at native x 2 and carried that stroke, so the game's own rule was
/// rasterised beside the font glyph we stamped for it — "slightly thicker" because a
/// native pixel column blown up 2.9x is fatter than a one-cell font stroke. At a
/// 119-column pane (scale 1.485) the band's first native column is 5, past the stroke,
/// and nothing shows: "not for all widths", exactly.
///
/// So the invariant is about NATIVE columns, and the sweep has to reach a scale where
/// one native text cell is more than two terminal columns wide.
///
/// FALSIFY by deleting the `clear_text_columns` call in `screen.rs` and restoring the
/// one-column extension rect (`Rect::new(col, ext.y, 1, ext.height)`): the wide panes
/// fail with `the picture's band (2, 2, 93, 45) samples native columns 2..257, which
/// runs into the frame's own rule at native 0..8`.
#[test]
fn journeys_picture_band_carries_no_pixel_of_the_frames_own_rules() {
    // Wide panes, where a native text cell covers more than one terminal column, in
    // both regimes: the first four are `letterbox` (18·rows <= 5·cols) and the rest
    // reclaim. 234x65 is the user's own 236x68 terminal.
    const WIDE: [(u16, u16); 8] =
        [(234, 65), (200, 55), (180, 50), (166, 46), (234, 80), (200, 70), (180, 62), (166, 58)];
    for (file, profile) in [
        ("Journey - The Quest Begins.adf", None),
        ("journey-r83-s890706.z6", Some(InterpreterProfile::Amiga)),
        ("journey-r83-s890706.z6", Some(InterpreterProfile::IbmPc)),
    ] {
        let Some(mut session) = boot(file, profile, 40) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let mut wide_seen = 0usize;
        for honor in [true, false] {
            for (w, h) in WIDE {
                let (state, area, _buf) = render_pane(&model, honor, (1, 1, w, h), &transcript);
                let ctx = format!("{file} {profile:?} honor={honor} pane {w}x{h}");
                let mut borders = glyph_borders(&state);
                borders.sort_by_key(|(_, n)| n.0); // report the left-hand rule first
                let scale = app::render::v6_layout::uniform_scale(native, (w as u32 * 8, h as u32 * 18));
                if scale.s >= 2.0 {
                    wide_seen += 1;
                }
                // A side band that is DRAWN and is its own placement — under a reclaim
                // plan the flank's art goes to `menu_flank_panel`'s dest with an explicit
                // native crop taken off the graphics-only canvas, which cannot contain a
                // border the game printed as text in the first place.
                let banded = state.v6_ring_plan.get() != "menu";
                for (label, r) in records(&state, "strip:art") {
                    if label != "strip:art" || r.2 >= w || !banded {
                        continue;
                    }
                    // Exactly the crop `draw_chrome_band` takes: the band's device span,
                    // less the letterbox offset, mapped back through the Nearest resize.
                    let sw = ((native.0 as f32 * scale.s).round() as u32).max(1);
                    let rel_x0 = (r.0 - area.x) as u32 * 8;
                    let sx_lo = (rel_x0 as i64 - scale.off_x as i64).clamp(0, sw as i64) as u32;
                    let sx_hi = (rel_x0 as i64 + r.2 as i64 * 8 - scale.off_x as i64).clamp(sx_lo as i64, sw as i64) as u32;
                    if sx_hi <= sx_lo {
                        continue;
                    }
                    let to_native =
                        |sp: u32| (((sp as f32 + 0.5) * native.0 as f32 / sw as f32).floor() as u32).min(native.0 as u32 - 1);
                    let (nx0, nx1) = (to_native(sx_lo), to_native(sx_hi - 1) + 1);
                    for (ext, (gx0, gx1)) in &borders {
                        assert!(
                            nx1 <= *gx0 || nx0 >= *gx1,
                            "{ctx}: the picture's band {r:?} samples native columns {nx0}..{nx1}, \
                             which runs into the frame's own rule at native {gx0}..{gx1} (stamped \
                             at {ext:?}) — a border is not the artwork's ground to stand on, and \
                             trimming the DESTINATION alone only moves the overlap one column in"
                        );
                    }
                }
                // …and the destination side of the same ruling: no drawn art may cover a
                // cell the stamped rule stands in. Under a reclaim plan that is the panel's
                // own art rect; under a letterbox plan it is the strip.
                let drawn: Vec<Quad> = records(&state, "strip:art")
                    .into_iter()
                    .filter(|(label, r)| label == "strip:art" && r.2 < w && banded)
                    .map(|(_, r)| r)
                    .chain(records(&state, "flank-art").into_iter().map(|(_, r)| r))
                    .collect();
                for (ext, _) in &borders {
                    for r in &drawn {
                        let clash = ext.0 < r.0 + r.2 && r.0 < ext.0 + ext.2;
                        assert!(
                            !clash,
                            "{ctx}: art at {r:?} stands in the columns of the frame's rule at {ext:?}"
                        );
                    }
                }
            }
        }
        assert!(
            wide_seen > 0,
            "{file} {profile:?}: this case exists for the scale where a native text cell covers \
             more than two terminal columns, and no pane in the sweep reached it"
        );
    }
}

/// SQ-0747 — no unwritten row between the frame's top rule and the story's first row.
///
/// The gap is one row of ceil-quantization, and it belonged to nothing: no runs map
/// into it, no art stands behind it, so it classified Empty → Art and the ring skipped
/// it. On the captured frame that left terminal row 2 never written across all 115
/// columns — a bare stripe through the picture panel and a one-row hole where the
/// frame's top rule should meet its two side rules.
///
/// FALSIFY by dropping the gap-absorption block in `screen.rs` (the `strips` fixup
/// after `strip_has_art`): the 115-column panes fail with `row 2 lies between the
/// frame's top rule (ends at row 2) and the story's first row (3) and is written by
/// nothing at all`.
#[test]
fn no_unwritten_row_stands_between_the_frames_top_rule_and_the_story() {
    for (file, profile) in [
        ("Journey - The Quest Begins.adf", None),
        ("journey-r83-s890706.z6", Some(InterpreterProfile::Amiga)),
    ] {
        let Some(mut session) = boot(file, profile, 40) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            for pane in WIDTHS.iter().flat_map(|&w| [(0, 0, w, 61), (1, 1, w, 61), (1, 1, w, 71)]) {
                let (state, area, buf) = render_pane(&model, honor, pane, &transcript);
                let ctx = format!("{file} {profile:?} honor={honor} pane {pane:?}");
                let vp = viewport_of(&state);
                let Some(top) = records(&state, "strip:text").into_iter().map(|(_, r)| r.1 + r.3).min() else {
                    continue;
                };
                // An untouched ratatui cell is a blank symbol with `Color::Reset` on both
                // channels and no modifier — precisely the `·` the pty harness prints for
                // "never written". Asking `Style::bg.is_some()` would not do it: that is
                // `Some(Reset)` on a cell nothing has touched.
                let untouched = |x: u16, y: u16| {
                    buf.cell((x, y)).is_some_and(|c| {
                        c.symbol().trim().is_empty()
                            && c.fg == ratatui::style::Color::Reset
                            && c.bg == ratatui::style::Color::Reset
                            && c.modifier.is_empty()
                    })
                };
                for y in top..vp.1 {
                    assert!(
                        (area.x..area.right()).any(|x| !untouched(x, y)),
                        "{ctx}: row {y} lies between the frame's top rule (ends at row {top}) and \
                         the story's first row ({}) and is written by nothing at all — the flanks \
                         must claim the gap the viewport's quantization opens",
                        vp.1
                    );
                }
                // …and the frame's own side rules run through it, so the box closes at its
                // corner instead of leaving a one-row hole where the top rule meets them.
                let mut borders = records(&state, "flank-divider");
                borders.extend(records(&state, "flank-border"));
                for (label, r) in &borders {
                    assert!(
                        r.1 <= top,
                        "{ctx}: {label} starts at row {} — the frame's side rule must reach the \
                         top rule (which ends at row {top}), not the quantized viewport top ({})",
                        r.1,
                        vp.1
                    );
                }
            }
        }
    }
}

/// SQ-0747, second pass — and no full-width band under the story either.
///
/// The story box has TWO quantized edges. The absorption above claims the half-cell
/// its TOP rounds away; the one its BOTTOM rounds away was left to the full-width band,
/// and that band spans the pane — so it paints straight across both of the frame's side
/// rules. Measured off `Journey - The Quest Begins.adf` (release 30 / serial 890322) at
/// a 121x36 terminal: the picture ran to row 23, the menu began at row 25, and row 24
/// was `strip:art (1, 24, 119, 1)`, placeholder cells right across, ending `▒▒▒▒▒▒│`
/// where the rows above it end `█│ │`. At a 236x68 terminal the same row has no art
/// behind it, classifies skipped, and reaches the screen unwritten instead.
///
/// So: no drawn full-width band may share a row with a side rule, and no row between
/// the frame's top rule and the menu may be written by nothing.
///
/// FALSIFY by dropping the downward walk in `screen.rs` (the `gap_bottom` loop): the
/// short panes fail with `the full-width band (1, 24, 119, 1) shares row 24 with the
/// frame's rule at (1, 2, 2, 23)`.
#[test]
fn no_full_width_band_paints_across_the_frames_side_rules() {
    for (file, profile) in [
        ("Journey - The Quest Begins.adf", None),
        ("journey-r83-s890706.z6", Some(InterpreterProfile::Amiga)),
        ("journey-r83-s890706.z6", Some(InterpreterProfile::IbmPc)),
    ] {
        let Some(mut session) = boot(file, profile, 40) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            // Short, tall and deliberately wide, in both plan regimes.
            let panes = SHORT_PANES
                .iter()
                .map(|&(w, h)| (w, h))
                .chain([(115, 62), (138, 68), (150, 71), (234, 65), (234, 80), (200, 55)]);
            for (w, h) in panes {
                let (state, _area, _buf) = render_pane(&model, honor, (1, 1, w, h), &transcript);
                let ctx = format!("{file} {profile:?} honor={honor} pane {w}x{h}");
                let mut borders = records(&state, "flank-divider");
                borders.extend(records(&state, "flank-border"));
                for (label, r) in records(&state, "strip:art") {
                    if label != "strip:art" || r.2 < w {
                        continue; // a skipped strip draws nothing; a side strip is not full width
                    }
                    for (blabel, b) in &borders {
                        let clash = r.1 < b.1 + b.3 && b.1 < r.1 + r.3;
                        assert!(
                            !clash,
                            "{ctx}: the full-width band {r:?} shares row {} with the frame's rule \
                             at {b:?} ({blabel}) — a band spans the pane, so it paints across both \
                             side rules; the remainder of the picture's own box belongs to the \
                             flanks, whichever side of the viewport it falls on",
                            r.1.max(b.1)
                        );
                    }
                }
                // …and the rules themselves run UNBROKEN from the top rule to the menu,
                // which is the same statement read the other way: the row the band used
                // to hold is a row the frame's own sides should be standing in. The menu
                // is a bottom-anchored strip under a reclaim plan and an ordinary ring
                // strip under `Letterbox`, so look for both.
                let vp = viewport_of(&state);
                let menu_top = records(&state, "strip:text")
                    .into_iter()
                    .chain(records(&state, "menu:text"))
                    .map(|(_, r)| r.1)
                    .filter(|&y| y >= vp.1 + vp.3)
                    .min();
                if let Some(menu_top) = menu_top {
                    for (label, r) in &borders {
                        assert!(
                            r.1 + r.3 >= menu_top,
                            "{ctx}: {label} at {r:?} stops at row {} while the menu begins at \
                             {menu_top} — the frame's side rule must reach it, and the rows \
                             between are exactly what the leftover full-width band used to paint \
                             across.\nstrips: {:#?}",
                            r.1 + r.3,
                            records(&state, "strip")
                        );
                    }
                }
            }
        }
    }
}

/// SQ-0783 — the frame reaches the pane's last column, at every width across the
/// transition where a native text cell stops fitting one terminal column.
///
/// A lone frame glyph is stamped in exactly ONE column by construction (SQ-0750 chose
/// that so a rule whose cell spans two columns is not drawn twice), and it was placed
/// by the column its cell's LEFT edge rounds to. Inside the screen that is invisible;
/// at the screen's own right edge it is a blank column between the frame and the pane
/// border, on every row. The user: *"starting at 159 width a blank space is added
/// after"* the frame — and the same gap is column 119 at 121x36. At 157 pane columns
/// the last native cell (632..640) covers 1.96 terminal columns, so one is always left
/// over; which one showed depended on where the two roundings fell, which is why a
/// single width proves nothing and this sweeps the whole transition.
///
/// The invariant is the game's own geometry: the frame's right-hand rule, its `┐` and
/// its `┘` all stand in the LAST column the screen's own last text cell covers. Both
/// mediums have to agree — the corner is a chrome TEXT strip and the rule below it a
/// stamped border extension — because a corner one column off its own shaft is the
/// other half of this defect.
///
/// FALSIFY by returning `None` unconditionally from `edge_glyph_col`: the widths past
/// the transition fail with `the frame's `┐` stands in column 156, but the game's last
/// text cell (native 632..640) covers columns 155..157 and the frame must reach the
/// last of them`.
#[test]
fn the_frames_edge_reaches_the_panes_last_column() {
    let Some(mut session) = boot("Journey - The Quest Begins.adf", None, 40) else { return };
    let transcript = session.take_transcript();
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let mut spans_two = 0usize;
    for honor in [true, false] {
        // The whole transition (SQ-0780's sweep found its own defect appearing at 155,
        // vanishing at 156 and returning from 157), plus the tall and short regimes and
        // a deliberately wide pane where the cell covers nearly three columns.
        let panes = (148u16..=168).map(|w| (w, 62u16)).chain([(119, 33), (115, 31), (150, 41), (234, 65), (234, 80)]);
        for (w, h) in panes {
            let (state, area, buf) = render_pane(&model, honor, (1, 1, w, h), &transcript);
            let ctx = format!("honor={honor} pane {w}x{h}");
            let scale = app::render::v6_layout::uniform_scale(native, (w as u32 * 8, h as u32 * 18));
            // The terminal columns the screen's LAST native text cell covers.
            let dev = |nx: u32| (scale.off_x as f32 + nx as f32 * scale.s) / 8.0;
            let first = area.x + dev(native.0 as u32 - 8).floor() as u16;
            let last = area.x + dev(native.0 as u32).ceil() as u16 - 1;
            if last > first {
                spans_two += 1;
            }
            // (1) the stamped rule down that side.
            let right = glyph_borders(&state)
                .into_iter()
                .find(|(_, n)| n.1 as u16 >= native.0)
                .map(|(ext, _)| ext);
            if let Some(ext) = right {
                let row = ext.1 + ext.3 / 2;
                let stood = (ext.0..ext.0 + ext.2).find(|&x| {
                    buf.cell((x, row)).is_some_and(|c| c.symbol() == "│" || c.style().add_modifier.contains(Modifier::REVERSED))
                });
                assert_eq!(
                    stood,
                    Some(last),
                    "{ctx}: the frame's side rule stands in column {stood:?} of its span {ext:?}, \
                     but the game's last text cell (native {}..{}) covers columns {first}..{last} \
                     and the frame must reach the last of them",
                    native.0 - 8,
                    native.0
                );
            }
            // (2) …and the corners above and below it, which are chrome TEXT.
            for corner in ['┐', '┘'] {
                let Some(y) = (area.y..area.bottom())
                    .find(|&y| (area.x..area.right()).any(|x| buf.cell((x, y)).is_some_and(|c| c.symbol() == corner.to_string())))
                else {
                    continue; // this frame does not draw that corner at this pane
                };
                let col = (area.x..area.right())
                    .rev()
                    .find(|&x| buf.cell((x, y)).is_some_and(|c| c.symbol() == corner.to_string()))
                    .expect("just found on this row");
                assert_eq!(
                    col, last,
                    "{ctx}: the frame's {corner:?} stands in column {col}, but the game's last text \
                     cell (native {}..{}) covers columns {first}..{last} and the frame must reach \
                     the last of them",
                    native.0 - 8,
                    native.0
                );
            }
        }
    }
    assert!(
        spans_two > 0,
        "this case exists for the scale where the screen's last text cell covers more than one \
         terminal column, and no pane in the sweep reached it"
    );
}

// ── (c) the reserved case: genuine artwork stays a bitmap ──

/// The corpus this rule must NOT reach: a side column that is a picture.
///
/// Zork Zero's, Shogun's and Arthur's frames are artwork down both sides — the runs
/// cannot account for those pixels, so they stay bitmaps, placed exactly where they
/// were. Getting this wrong is a worse regression than the defect being fixed, and it
/// is the reason SQ-0750 sat open through five passes.
///
/// FALSIFY by classifying a side band as text unconditionally (rather than on the
/// graphics canvas being clear): every case fails with `Zork Zero's left flank is no
/// longer drawn as art`.
#[test]
fn a_side_column_that_is_artwork_stays_a_bitmap() {
    for file in [
        "zork0-r393-s890714.z6",
        "Zork Zero - The Revenge of Megaboz.adf",
        "shogun-r322-s890706.z6",
        "arthur-r74-s890714.z6",
        "Arthur - The Quest for Excalibur.adf",
    ] {
        let Some(mut session) = boot(file, None, 12) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            // SQ-0779: and at panes with no letterbox slack, where the ring takes the
            // `Letterbox` plan. The border extension now runs under every plan, not just
            // Menu — reserved to the glyph ink, precisely so these frames keep the bands
            // they have always had. This regime was swept by nothing before.
            for pane in WIDTHS.iter().flat_map(|&w| [(0, 0, w, 61), (1, 1, w, 64), (0, 0, w, w * 5 / 18)]) {
                let (state, _, _) = render_pane(&model, honor, pane, &transcript);
                let ctx = format!("{file} honor={honor} pane {pane:?}");
                let vp = viewport_of(&state);
                // Side ART strips: narrower than the pane, and DRAWN (a strip the ring
                // skipped says so in its own label).
                let sides: Vec<Quad> = records(&state, "strip:art")
                    .into_iter()
                    .filter(|(label, r)| label == "strip:art" && r.2 < pane.2)
                    .map(|(_, r)| r)
                    .collect();
                assert!(
                    !sides.is_empty(),
                    "{ctx}: this game's side columns are ARTWORK and must still be drawn as \
                     bitmaps — no side art strip survives.\n{:#?}",
                    records(&state, "strip")
                );
                for r in &sides {
                    // A flank never starts BELOW the story's first row: it either begins
                    // with it or reaches up into the quantization remainder above it
                    // (SQ-0747). Its bottom is the ring's business — the Extend plan
                    // clips a flank to the art's own last row, which is the point of it.
                    assert!(
                        r.1 <= vp.1,
                        "{ctx}: the side art strip {r:?} starts below the story viewport {vp:?}"
                    );
                }
                // …and none of them was reclassified into the game's own characters.
                let mut glyphs = records(&state, "flank-divider");
                glyphs.extend(records(&state, "flank-border"));
                assert!(
                    glyphs.iter().all(|(l, _)| !l.contains("glyph")),
                    "{ctx}: a border column of an ARTWORK flank was drawn as a character — the \
                     content test must reserve the glyph path for pixels the game's runs \
                     account for.\n{glyphs:#?}"
                );
            }
        }
    }
}
