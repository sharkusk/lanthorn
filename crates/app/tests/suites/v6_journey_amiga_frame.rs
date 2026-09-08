//! Journey's line-drawing frame under the Amiga interpreter profile — SQ-0742.
//!
//! The report, playing Journey in Ghostty (hybrid, the shipped default) with the
//! Amiga profile selected: *"in hybrid mode a nice border frame is being drawn,
//! but it doesn't scale to our window width (raster mode looks great). also,
//! hybrid mouse click not accurate (if you click way above the menu you will get
//! a hit)"* — and then, decisively, *"if i resize the window to match everything,
//! the mouse click works!"* and *"ibmpc mouse clicks work properly."*
//!
//! ONE defect, seen twice. Journey draws its frame as TEXT: reverse-video spaces
//! under the IBM PC profile, line-drawing glyphs (`┌ ─ ┐ │ └ ┘`) under the Amiga
//! one. Those glyphs are non-blank runs that share every row of the story box, so
//! the hybrid "painted menu takeover" gate — which tested rows only — called an
//! ordinary gameplay screen a menu and routed the whole frame to the CELL path.
//! That path draws the game's 80 columns one-for-one into a pane of any width
//! while placing the transcript and the click map PROPORTIONALLY across the pane,
//! so at 138 columns the border stopped at column 79, the prose ran straight
//! through it, and a click landed three game rows below where it was aimed — you
//! had to click well above the menu to press it. At an 80-column pane the two
//! placements coincide, which is exactly why resizing "fixed" it. Under IBM PC the
//! same rules are reverse-video SPACES, which trim to empty and never tripped the
//! gate — hence "ibmpc mouse clicks work properly".
//!
//! Fixed in two coordinated places, both in `render/screen.rs`:
//!   1. the takeover gate now requires a run to lie inside the story box on BOTH
//!      axes, so a frame rule beside the story is chrome, not a menu; and
//!   2. a text strip's repeated-glyph RULE is drawn across its own SCALED span
//!      rather than one terminal cell per fragment, so the border reaches the pane
//!      edge the way the IBM PC profile's reverse-video bars already do.
//!
//! SECOND PASS, on the user's *"the bottom menu columns are not aligned at most
//! widths"*: a lone box-drawing/block glyph is a DIVIDER, and it is now kept out of
//! SQ-0509's fragment merge and stamped at its own scaled column. The merge exists
//! to re-assemble a WORD out of proportional-metric fragments, and then advances one
//! terminal cell per character; Journey abuts each party member's `-->` marker to
//! the `▌` dividing the party column from the commands (native px 246 and 248), so
//! that `▌` rode along on the marker's character advance and landed three columns
//! left of its own column — but only on the rows that carry a marker. The menu's
//! column dividers therefore zig-zagged, at 83% of the pane sizes swept.
//!
//! Swept across pane widths — including deliberately wider than the game's 80
//! column screen — because a fix that only holds where the numbers coincide is the
//! bug in another costume, and now across pane HEIGHTS too, since the first sweep
//! sampled exactly one and could not have seen a defect confined to another. Both
//! `honor_game_colours` modes, per the project's colour-render convention.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// Pane widths swept by every case: the game's own 80 columns, where the cell
/// path's 1:1 chrome and the proportional transcript happen to agree, and five
/// wider panes where they do not.
const WIDTHS: [u16; 6] = [80, 96, 110, 124, 138, 150];

/// Pane HEIGHTS swept by the divider case. 51 is what every case here used to run
/// at; 71 and 84 cover the tall-terminal regime a full-height window on a modern
/// display puts the story pane in, and 30 the short one, where the letterbox scale
/// stops being width-limited and the whole placement arithmetic changes. The first
/// sweep sampled one height and could not have seen a defect confined to another —
/// the SQ-0548 lesson, on the other axis.
const HEIGHTS: [u16; 4] = [30, 51, 71, 84];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Journey under `profile` and drive the intro to the Praxix command menu —
/// the frame the report is about: a full line-drawing border, a left picture
/// column, prose beside it and the command menu along the bottom.
///
/// The container is not the variable and never was: the user established that
/// `Journey.blb` with the Amiga interpreter selected behaves exactly as the Amiga
/// disk image does, so the ordinary in-repo fixture plus an explicit profile
/// reproduces the whole thing without an `.adf`.
fn journey_at_menu(profile: InterpreterProfile) -> Option<GameSession> {
    let story_path = stories_dir().join("journey-r83-s890706.z6");
    let story_bytes = match std::fs::read(&story_path) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return None;
        }
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut session = GameSession::new_with_trace(
        story_bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .expect("Journey (v6) should load and boot without a ZError");
    // SQ-1393: the machine's own colour table. `new_with_trace` is the
    // no-machine door and presents §8.3.1's own, so a harness that boots a
    // press states the press's table here.
    session.machine.set_palette(profile.palette());
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..40 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if r.transcript.contains("Praxix") || r.transcript.contains("magical resources") {
            break;
        }
    }
    Some(session)
}

/// A hybrid render at real kitty-ish cell metrics (8×18). `Picker::halfblocks()`
/// reports a 1×2 cell — a layout regime that never reproduces a scale defect at
/// all — so the sweep has to run at a plausible font cell (the same lesson
/// SQ-0548 recorded).
#[allow(deprecated)]
fn render_hybrid_at(
    model: &app::engine::ScreenModel,
    honor: bool,
    cols: u16,
    rows: u16,
) -> (app::state::AppState, Rect, Buffer) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    (state, area, buf)
}

/// The original sweep's pane height, kept so the cases that pin one height read
/// exactly as they did.
fn render_hybrid(
    model: &app::engine::ScreenModel,
    honor: bool,
    cols: u16,
) -> (app::state::AppState, Rect, Buffer) {
    render_hybrid_at(model, honor, cols, 51)
}

fn row_text(buf: &Buffer, area: Rect, y: u16) -> String {
    (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
}

/// Which render path drew this frame, as `/dump-windows` reports it.
fn path_label(state: &app::state::AppState) -> String {
    state
        .v6_cell_map
        .borrow()
        .iter()
        .find(|e| e.label.starts_with("path:"))
        .map(|e| e.label.clone())
        .unwrap_or_else(|| "<no path recorded>".into())
}

/// (a) Journey's gameplay screen is gameplay, not a painted menu takeover — under
/// BOTH profiles, so the line-drawing frame and the reverse-video one are drawn by
/// the same renderer.
///
/// FALSIFY by restoring the row-only takeover test in `screen.rs`: the Amiga cases
/// report `path:cell — painted menu takeover routed here`.
#[test]
fn journey_gameplay_takes_the_hybrid_ring_under_both_profiles() {
    for profile in [InterpreterProfile::IbmPc, InterpreterProfile::Amiga] {
        let Some(session) = journey_at_menu(profile) else { return };
        let model = session.screen();
        for honor in [true, false] {
            for cols in WIDTHS {
                let (state, _, _) = render_hybrid(&model, honor, cols);
                let path = path_label(&state);
                assert_eq!(
                    path, "path:hybrid-ring",
                    "{profile:?} honor={honor} w={cols}: an ordinary gameplay screen must take the \
                     chrome ring; a frame rule beside the story is chrome, not a menu takeover"
                );
            }
        }
    }
}

/// (b) The Amiga frame reaches the pane at EVERY width, and the prose stays inside
/// it. The user's two visible complaints in one assertion pair: the border runs
/// unbroken from the pane's left edge to its right edge, and the transcript stops
/// at the border instead of running through it.
///
/// FALSIFY by reverting `collapse_row_rules`: every width past 80 fails with the
/// closing corner stuck at column 79 — the reported "doesn't scale to our window
/// width".
#[test]
fn journey_amiga_border_reaches_the_pane_at_every_width() {
    let Some(session) = journey_at_menu(InterpreterProfile::Amiga) else { return };
    let model = session.screen();
    for honor in [true, false] {
        for cols in WIDTHS {
            let (state, area, buf) = render_hybrid(&model, honor, cols);
            let ctx = format!("honor={honor} w={cols}");
            let find_row = |open: char| (0..area.height).find(|&y| row_text(&buf, area, y).starts_with(open));
            // Both horizontal rules of the box: the titled top and the plain bottom.
            //
            // The bottom rule is exempt at exactly 80 columns, where the pane is one
            // terminal column per native text cell. There the letterbox scale is 1.0
            // and a 16px game row COMPRESSES into an 18px terminal cell, while a text
            // strip draws its game rows on consecutive terminal rows (SQ-0543, one
            // cell tall each) — so the menu's seventh row, the frame's bottom edge,
            // needs a row past the pane's last. That is the packing's own vertical
            // arithmetic, older than this fix and untouched by it; the rule is drawn at
            // every width where the strip has the room.
            for (open, close) in [('┌', '┐'), ('└', '┘')] {
                let Some(y) = find_row(open) else {
                    assert!(
                        open == '└' && cols == 80,
                        "{ctx}: no frame row opening with {open:?}"
                    );
                    continue;
                };
                let row = row_text(&buf, area, y);
                let end = row
                    .char_indices()
                    .filter(|&(_, c)| c == close)
                    .map(|(i, _)| row[..i].chars().count())
                    .next_back()
                    .unwrap_or_else(|| panic!("{ctx}: frame row {y} has no closing {close:?}: {row:?}"));
                // The rule reaches the pane's right edge — not the game's 80th column.
                assert!(
                    end + 2 >= area.width as usize,
                    "{ctx}: the {open:?} border closes at column {end} of a {}-column pane — the \
                     rule must span the pane, not the game's own column count\n{row:?}",
                    area.width
                );
                // …and it is one unbroken line getting there.
                let hole = row.chars().take(end).position(|c| c == ' ');
                assert!(
                    hole.is_none(),
                    "{ctx}: the {open:?} border has a gap at column {} — a rule must close the \
                     seams a scale opens around its corners and title\n{row:?}",
                    hole.unwrap()
                );
            }
            // The prose stays inside the frame: the story viewport stops at or before
            // the border's own column, so no line of text crosses the right-hand rule.
            let top = find_row('┌').expect("the top rule is on screen at every width");
            let right_col = row_text(&buf, area, top).chars().count()
                - row_text(&buf, area, top).chars().rev().position(|c| c == '┐').unwrap()
                - 1;
            let vp = state.transcript_geom.get().expect("hybrid publishes transcript geometry").area;
            assert!(
                vp.right() as usize <= right_col,
                "{ctx}: the transcript ({vp:?}) runs past the frame's right rule at column \
                 {right_col} — the prose must wrap inside the border, not through it"
            );
        }
    }
}

/// (c) BEHAVIOURAL, and the half the player actually felt: a click where a menu
/// item is DRAWN must make the game act, at every pane width and under both
/// profiles.
///
/// The oracle is the game's own verdict, not "did the model change" — Journey
/// repaints its menu on a rejected click too. Pressing a party member's "Cast"
/// opens that character's spell list, so the menu gains spell names it did not
/// carry before. A click the game rejects leaves the same commands on screen.
///
/// FALSIFY by restoring the row-only takeover test: under Amiga at every width
/// past 80 the spell list never appears, because the cell path's proportional
/// click map puts the click three game rows below the row it was drawn on — the
/// reported "if you click way above the menu you will get a hit".
///
/// SQ-0747: swept over BOTH plan regimes, because the menu is not the same strip in
/// both. Only a reclaim plan anchors one to the pane's bottom; a pane with no vertical
/// slack (`18·rows <= 5·cols`) puts the very same command menu in an ordinary TEXT
/// strip of the ring, drawn by the very same consecutive-row packing — and the click
/// map was handed the packed row mapping only when a bottom-anchored strip produced
/// it. Everywhere else it inverted the pane linearly, which is SQ-0550's defect one
/// plan over: the user's *"the mouse is off by one row"*. Measured before the fix off
/// the release floppy: `Cast` clicked exactly where it is drawn was ACCEPTED at 119x34
/// (menu plan) and MISSED at 115x31, 150x41 and 234x65 (letterbox).
///
/// FALSIFY that half by dropping the `strips` fallback for `text_rows` in `screen.rs`:
/// every letterbox pane fails with `clicking 'Cast' where it is DRAWN … must open
/// Praxix's spell list; "Tremor" never appeared`.
#[test]
fn journey_menu_click_where_drawn_reaches_the_game() {
    // Spells Praxix can cast — what the game puts on screen when it accepts the
    // click, and nothing it shows on the plain command menu.
    const SPELLS: [&str; 3] = ["Tremor", "Wind", "Elevation"];
    // The reclaim regime at the original height, then the no-slack one: short, and
    // deliberately wide, where the scale is largest.
    let panes: Vec<(u16, u16)> = WIDTHS
        .iter()
        .map(|&w| (w, 51u16))
        .chain([(115, 31), (119, 33), (138, 38), (150, 41), (200, 55), (234, 65)])
        .collect();
    for profile in [InterpreterProfile::IbmPc, InterpreterProfile::Amiga] {
        for honor in [true, false] {
            for (cols, rows) in panes.iter().copied() {
                // A fresh game per click: an accepted one takes Journey elsewhere.
                let Some(mut session) = journey_at_menu(profile) else { return };
                let model = session.screen();
                let ctx = format!("{profile:?} honor={honor} pane {cols}x{rows}");
                let (state, area, buf) = render_hybrid_at(&model, honor, cols, rows);
                let map = state
                    .graphics_render
                    .borrow()
                    .last_v6_map
                    .clone()
                    .expect("a v6 frame records a click map for the mouse handler");

                // WHERE IT IS DRAWN: the cell the player sees "Cast" on.
                let sy = (0..area.height)
                    .find(|&y| row_text(&buf, area, y).contains("Cast"))
                    .unwrap_or_else(|| panic!("{ctx}: the 'Cast' command is drawn somewhere"));
                let sx = row_text(&buf, area, sy).find("Cast").expect("column of the label") as u16;
                let (gx, gy) = map
                    .map_click(sx + 1, sy)
                    .unwrap_or_else(|| panic!("{ctx}: the cell showing 'Cast' ({sx},{sy}) is off the image"));

                // Delivered exactly as `main.rs` delivers a v6 click during a
                // `read_char`: the game pixel, then ZSCII 254 (§3.8).
                assert_eq!(session.pending_input(), InputKind::Char, "{ctx}: the menu sits in read_char");
                session.set_mouse(gy, gx);
                let _ = session.submit_char(254);

                // DID THE GAME ACT? Its own answer: the spell list is on screen.
                let after = session.screen();
                let WinNode::Layered(items) = &after.root else { panic!("{ctx}: v6 Layered root") };
                let painted: Vec<String> = items
                    .iter()
                    .filter_map(|it| match &it.node {
                        WinNode::Grid(g) => Some(g.px_texts.iter()),
                        _ => None,
                    })
                    .flatten()
                    .map(|t| t.text.trim().to_string())
                    .collect();
                for spell in SPELLS {
                    assert!(
                        painted.iter().any(|t| t == spell),
                        "{ctx}: clicking 'Cast' where it is DRAWN (cell {sx},{sy} → game pixel \
                         {gx},{gy}) must open Praxix's spell list; {spell:?} never appeared. \
                         Menu now: {painted:?}"
                    );
                }
            }
        }
    }
}

/// (d) SECOND PASS — the command menu's COLUMN DIVIDERS line up down the panel, at
/// every width. The user's *"the bottom menu columns are not aligned at most
/// widths"*.
///
/// Journey draws each party member's row as `… -->▌ <command>`: a `-->` marker whose
/// native pixels END at 246 and the `▌` column divider that STARTS at 248. Two
/// abutting fragments, so SQ-0509's word merge glued them into one run — and a run
/// is positioned by its scale-mapped pixel but then advances one terminal cell per
/// character. The `▌` therefore rode the marker's advance instead of its own column,
/// landing three columns left of where the same divider lands on the "Game" row,
/// which carries no marker. The panel's divider zig-zagged row to row.
///
/// The invariant asserted is the game's own: every body row's dividers stand in the
/// SAME columns, and those columns are where the native pixels map to.
///
/// FALSIFY by dropping the box-glyph branch from `collapse_row_rules`: every width
/// fails with two different `▌` columns in one panel.
#[test]
fn journey_amiga_menu_dividers_line_up_down_the_panel() {
    let Some(session) = journey_at_menu(InterpreterProfile::Amiga) else { return };
    let model = session.screen();
    for honor in [true, false] {
        for (cols, pane_h) in WIDTHS.iter().flat_map(|&c| HEIGHTS.iter().map(move |&h| (c, h))) {
            let (state, area, buf) = render_hybrid_at(&model, honor, cols, pane_h);
            let ctx = format!("honor={honor} w={cols} h={pane_h}");
            let rows: Vec<String> = (0..area.height).map(|y| row_text(&buf, area, y)).collect();
            // The menu's body rows: the ones carrying a party member or the trailing
            // "Game" row, which is the row whose missing `-->` exposed the drift.
            let body: Vec<(usize, &String)> = rows
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    ["Proceed", "Praxix", "Minar", "Tag", "Game"].iter().any(|w| r.contains(w))
                })
                .collect();
            assert!(body.len() >= 4, "{ctx}: the command menu is on screen (rows found: {})", body.len());
            let cols_of = |r: &str, g: char| -> Vec<usize> {
                r.char_indices().filter(|&(_, c)| c == g).map(|(i, _)| i).collect()
            };
            for glyph in ['▌', '│'] {
                let mut seen: Vec<usize> =
                    body.iter().flat_map(|(_, r)| cols_of(r, glyph)).collect();
                seen.sort_unstable();
                seen.dedup();
                let want = if glyph == '▌' { 2 } else { 4 };
                assert_eq!(
                    seen.len(),
                    want,
                    "{ctx}: the menu's {glyph:?} dividers stand in {} different columns ({seen:?}) — \
                     a divider is a POSITION, and every body row must put it in the same one\n{}",
                    seen.len(),
                    body.iter()
                        .map(|(y, r)| format!("{y:3}|{}|", r.trim_end()))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
            // …and each divider stands the same distance from the column it heads.
            // The game puts BOTH `▌` exactly 16 native pixels (two text cells) left
            // of their column's text — `▌`@121 before "Praxix"@137, `▌`@249 before
            // "Cast"@265 — so through any one scale the two gaps must come out the
            // same. Pre-fix the second was dragged onto the `-->` marker's character
            // advance and the gaps differed (4 and 6 columns at a 138-column pane).
            let member = rows
                .iter()
                .find(|r| r.contains("Praxix") && r.contains("Cast"))
                .unwrap_or_else(|| panic!("{ctx}: Praxix's row carries his 'Cast' command"));
            let bars = cols_of(member, '▌');
            let (a, b) = (bars[0], bars[1]);
            let (name, cmd) = (member.find("Praxix").unwrap(), member.find("Cast").unwrap());
            // One column of slack: two native pixel positions 16px apart can round to
            // cell distances differing by one, which is the sub-cell rounding this
            // renderer lives with everywhere. Pre-fix the gap differed by three.
            assert!(
                (name as i64 - a as i64 - (cmd as i64 - b as i64)).abs() <= 1,
                "{ctx}: the party divider sits {} columns before its column's text and the \
                 command divider {} — the game drew both 16 native pixels ahead\n{member:?}",
                name - a,
                cmd - b
            );
            let _ = &state;
        }
    }
}

/// (e) THE FRAME'S SIDES REACH THE MENU — SQ-0742's second half, the VERTICAL
/// extent. `c7bd0bd8` fixed the frame's width; nothing addressed its height.
///
/// The report: *"there is still a big chunk of the border missing from the right
/// side (at the bottom), and a gap at the top."* Measured at the user's own pane
/// (138x68 cells, 8x18), the right flank band is 55 rows and reaches the menu — the
/// band is not short. Its IMAGE is. The game's canvas is 400 native pixels tall and
/// the flank is drawn at the uniform scale, so the border's pixels run out at
/// terminal row 39 and the remaining eighteen rows down to the menu are transparent.
/// Carrying the border through that reclaimed gap is exactly what
/// `flank_divider_extension` exists for — and under the Amiga profile it produced
/// NOTHING, for either flank, at any pane size.
///
/// The cause is a one-pixel probe. The extension locates the border as the opaque
/// native column abutting the story box; Journey/Amiga draws its frame with `│`
/// glyphs, whose ink sits in the middle of an 8-pixel text cell, so the column
/// immediately outside the story box is that cell's blank padding. The IBM PC
/// profile draws the same border as reverse-video spaces, which ink that column —
/// which is why one profile framed the reclaimed gap and the other did not.
///
/// The A/B is the assertion: the two profiles draw the same frame, so both must
/// carry both flanks' borders down to the menu.
///
/// FALSIFY by restoring the single-column probe in `flank_divider_extension`: every
/// Amiga case fails with `0 flank dividers`.
#[test]
fn journey_frame_sides_reach_the_menu_under_both_profiles() {
    for profile in [InterpreterProfile::IbmPc, InterpreterProfile::Amiga] {
        let Some(session) = journey_at_menu(profile) else { return };
        let model = session.screen();
        for honor in [true, false] {
            // The user's pane first, then the widths this file already sweeps, at a
            // tall pane and a short one — the reclaimed gap only exists when the pane
            // is taller than the game's scaled canvas, and it is what this is about.
            for (cols, rows) in [(138, 68), (96, 51), (110, 51), (138, 51), (150, 84)] {
                let (state, area, _) = render_hybrid_at(&model, honor, cols, rows);
                let ctx = format!("{profile:?} honor={honor} pane {cols}x{rows}");
                let map = state.v6_cell_map.borrow();
                let viewport = map
                    .iter()
                    .find(|e| e.label == "viewport")
                    .unwrap_or_else(|| panic!("{ctx}: the ring records its viewport"))
                    .cells;
                let dividers: Vec<(u16, u16, u16, u16)> =
                    // Prefix, so a border STAMPED as the game's own character counts too
                    // (SQ-0750) — the frame reaching the menu is the invariant here,
                    // whichever medium carries it.
                    map.iter().filter(|e| e.label.starts_with("flank-divider")).map(|e| e.cells).collect();
                assert!(
                    dividers.len() >= 2,
                    "{ctx}: {} flank dividers — the frame's side borders are drawn from the \
                     game's own canvas, which ends well above the menu, so without an extension \
                     per flank the frame has no sides through the reclaimed gap.\n{:#?}",
                    dividers.len(),
                    map.iter().map(|e| (e.label.clone(), e.cells)).collect::<Vec<_>>()
                );
                let menu_top = viewport.1 + viewport.3;
                for d in &dividers {
                    assert_eq!(
                        d.1 + d.3,
                        menu_top,
                        "{ctx}: divider {d:?} stops before the menu at row {menu_top}"
                    );
                    assert!(
                        d.0 >= area.x && d.0 + d.2 <= area.right(),
                        "{ctx}: divider {d:?} falls outside the pane {area:?}"
                    );
                }
                // One per flank, on opposite sides of the story viewport.
                assert!(
                    dividers.iter().any(|d| d.0 < viewport.0) && dividers.iter().any(|d| d.0 >= viewport.0),
                    "{ctx}: both flanks carry a border — got {dividers:?} against viewport {viewport:?}"
                );
            }
        }
    }
}

// ── SQ-0747 / SQ-0750: the frame's SIDE borders, at the pane the user captured ──
//
// Everything above renders the pane at the buffer's origin and lets Journey's prose go
// undrawn. Neither is what the player has. `/dump-windows` on the real frame
// (2026-08-10) reports `pane 163x61 at (1,1)` — the story pane sits INSIDE the app's
// panel border — and Journey's prose never reaches the story window's `lines` at all:
// it arrives as the session TRANSCRIPT and is drawn from `AppState`. A harness that
// renders the model without feeding the transcript renders a frame with no story text
// in it, which is what four earlier sweeps of this defect were measuring.

/// A rect as `/dump-windows` records it: `(x, y, w, h)`, cells or native pixels.
type Quad = (u16, u16, u16, u16);

/// The pane from the user's own `/dump-windows`: 163x61 at (1,1), cell 8x18, hybrid.
const USER_PANE: Quad = (1, 1, 163, 61);

/// A hybrid render at an arbitrary pane ORIGIN with the session transcript fed in, so
/// the frame carries Journey's prose the way the player's does.
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

fn pane_row(buf: &Buffer, area: Rect, y: u16) -> String {
    (area.x..area.right()).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
}

/// The letterbox scale and cell metrics this frame resolved, out of the same record
/// `/dump-windows` prints them from.
fn scale_and_cell(state: &app::state::AppState) -> (f32, u16, u16) {
    let e = state
        .v6_cell_map
        .borrow()
        .iter()
        .find(|e| e.label == "scale")
        .map(|e| e.native)
        .expect("a hybrid ring frame records its scale");
    (e.0 as f32 / 100.0, e.2, e.3)
}

/// The flank border extensions this frame drew: `(cell rect, native crop)`.
fn flank_dividers(state: &app::state::AppState) -> Vec<(Quad, Quad)> {
    flank_records(state, "flank-divider")
}

/// Every `/dump-windows` record under one label: `(cell rect, native rect)`.
///
/// Matched by PREFIX. SQ-0750: a border the game printed as a character is now
/// stamped as that character rather than uploaded as a bitmap of it, and its record
/// says which (`flank-divider (glyph '│' style=00)`). Both media are collected here;
/// [`is_glyph`] tells them apart where the distinction matters.
fn flank_records(state: &app::state::AppState, label: &str) -> Vec<(Quad, Quad)> {
    state
        .v6_cell_map
        .borrow()
        .iter()
        .filter(|e| e.label == label || e.label.starts_with(&format!("{label} (")))
        .map(|e| (e.cells, e.native))
        .collect()
}

/// Was this border STAMPED as the game's own character (SQ-0750) rather than
/// uploaded as a bitmap of it? A glyph is not resampled, so there is nothing to
/// magnify — its record carries the character's native text CELL rather than a crop,
/// and a crop is always one native ROW deep (SQ-0779).
fn is_glyph(crop: Quad) -> bool {
    crop.3 == 0
}

/// The story viewport this frame resolved, in pane-absolute cells.
fn viewport_of(state: &app::state::AppState) -> Quad {
    state
        .v6_cell_map
        .borrow()
        .iter()
        .find(|e| e.label == "viewport")
        .map(|e| e.cells)
        .expect("a hybrid ring frame records its story viewport")
}

/// The panes the two side-border cases sweep: the user's own first, then sizes this
/// file already covers, so a fix that only holds at one pane fails here.
const SIDE_PANES: [Quad; 5] =
    [USER_PANE, (1, 1, 138, 68), (0, 0, 138, 68), (0, 0, 100, 71), (0, 0, 96, 51)];

/// (f) SQ-0750 — THE SIDE BORDER IS DRAWN AT THE LETTERBOX SCALE, like every other
/// pixel in the ring.
///
/// `flank_divider_extension` locates the frame's border column and
/// `draw_chrome_band_stretched` RESIZES the native crop it returns to fill the band. So
/// the crop's width against the band's width IS the extension's horizontal
/// magnification — and it cropped to the border's INK alone. For a border the game
/// printed as a reverse-video SPACE that is the same number, because the run inks its
/// whole 8-pixel text cell. For one printed with a box-drawing GLYPH it is not: a `│`'s
/// stroke is ONE pixel inside its 8-pixel cell, so the magnification came out sixteen
/// times the letterbox scale and a hairline was inflated into a solid filled bar.
///
/// The invariant is the ring's own: `crop_width · s == band_width · cell_w`.
///
/// FALSIFY by cropping to the ink alone again (`(dnx0, mid, dnx1 - dnx0, 1)`):
/// `Amiga honor=true pane 163x61: the left flank border is magnified 16.00x while the
/// ring's letterbox scale is 2.03x … (ext (66, 3, 2, 46), crop (259, 152, 1, 1))`.
#[test]
fn journey_flank_border_is_drawn_at_the_letterbox_scale() {
    for profile in [InterpreterProfile::Amiga, InterpreterProfile::IbmPc] {
        let Some(mut session) = journey_at_menu(profile) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            for pane in SIDE_PANES {
                let (state, _, _) = render_pane(&model, honor, pane, &transcript);
                let (s, cell_w, _) = scale_and_cell(&state);
                let dividers = flank_dividers(&state);
                let ctx = format!("{profile:?} honor={honor} pane {}x{}", pane.2, pane.3);
                assert_eq!(dividers.len(), 2, "{ctx}: both flanks carry a border ({dividers:?})");
                for (i, (ext, crop)) in dividers.iter().enumerate() {
                    let side = if i == 0 { "left" } else { "right" };
                    // SQ-0750: a border stamped as the game's own CHARACTER has no
                    // magnification to check — it is not resampled at all. What it must be
                    // is exactly one column wide, because a rule is one column: the band's
                    // cell span can be two (it is, at the user's 163-column pane) and
                    // stamping the glyph across both would draw a double rule.
                    if is_glyph(*crop) {
                        // SQ-0779: …and the extension is that character's own native text
                        // CELL, which is one terminal column only where a column is about
                        // one native cell. At a wide pane it is two or three, and those
                        // columns belong to the border rather than to the picture beside
                        // it — the whole of the user's ruling. The glyph still stands in
                        // exactly ONE of them, which is what keeps the double rule away;
                        // that half is pinned in `v6_frame_border_medium.rs`.
                        let cell = (crop.2.max(1) as f32 * s / cell_w as f32).ceil() as u16 + 1;
                        assert!(
                            ext.2 >= 1 && ext.2 <= cell,
                            "{ctx}: the {side} flank border is stamped as a character, so its \
                             span is that character's native cell ({} native px → at most \
                             {cell} columns at scale {s:.2}) — got {ext:?}",
                            crop.2
                        );
                        continue;
                    }
                    assert!(crop.2 > 0, "{ctx}: the {side} flank border has an empty crop {crop:?}");
                    let mag = (ext.2 as f32 * cell_w as f32) / crop.2 as f32;
                    assert!(
                        (mag - s).abs() <= s * 0.2,
                        "{ctx}: the {side} flank border is magnified {mag:.2}x while the ring's \
                         letterbox scale is {s:.2}x — the crop must be the native columns the \
                         band's CELLS cover, not the ink alone, or a glyph's thin stroke is \
                         inflated into a solid filled bar (ext {ext:?}, crop {crop:?})"
                    );
                }
            }
        }
    }
}

/// (g) SQ-0750, the same defect as the player sees it: *"we are mixing the reverse
/// space into the amiga line drawing."*
///
/// Under the Amiga profile Journey prints its whole frame with box-drawing glyphs and
/// emits no reverse-video run anywhere on this screen — measured: every run in the menu
/// band arrives with `style & 1 == 0`. So a solid filled block standing in that frame's
/// line is not the game's, it is ours. Under the IBM PC profile the same border IS a
/// reverse-video space and a solid block there is exactly right — which is what makes
/// this an A/B rather than "Amiga is special": the discriminator is what the band
/// CONTAINS, never which profile is loaded.
///
/// FALSIFY by cropping to the ink alone again: `Amiga honor=true: 138 filled cell(s) in
/// the frame's side borders … the left flank border column 66 is a solid filled block
/// (Rgb(220, 220, 220)) at row 3`.
#[test]
fn journey_amiga_flank_border_is_a_stroke_not_a_filled_block() {
    // A cell wholly covered by bright ink: `fg == bg` means the halfblock renderer found
    // both halves the same colour, i.e. the cell is filled edge to edge.
    let filled = |c: &ratatui::buffer::Cell| -> Option<ratatui::style::Color> {
        let (fg, bg) = (c.style().fg?, c.style().bg?);
        let ratatui::style::Color::Rgb(r, g, b) = fg else { return None };
        (fg == bg && r >= 128 && g >= 128 && b >= 128).then_some(fg)
    };
    for profile in [InterpreterProfile::Amiga, InterpreterProfile::IbmPc] {
        let Some(mut session) = journey_at_menu(profile) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            let (state, _, buf) = render_pane(&model, honor, USER_PANE, &transcript);
            let ctx = format!("{profile:?} honor={honor}");
            let dividers = flank_dividers(&state);
            let mut blocks = 0usize;
            let mut first: Option<String> = None;
            for (i, (ext, _)) in dividers.iter().enumerate() {
                let side = if i == 0 { "left" } else { "right" };
                for y in ext.1..ext.1 + ext.3 {
                    for x in ext.0..ext.0 + ext.2 {
                        if let Some(col) = buf.cell((x, y)).and_then(filled) {
                            blocks += 1;
                            first.get_or_insert_with(|| {
                                format!(
                                    "the {side} flank border column {x} is a solid filled block \
                                     ({col:?}) at row {y}"
                                )
                            });
                        }
                    }
                }
            }
            match profile {
                InterpreterProfile::Amiga => assert_eq!(
                    blocks, 0,
                    "{ctx}: {blocks} filled cell(s) in the frame's side borders — Journey prints \
                     this frame with box-drawing glyphs and no reverse-video run at all, so a \
                     solid block standing in its line is ours, not the game's. {}",
                    first.unwrap_or_default()
                ),
                // The A/B: the same code path, a border the game really did print as a
                // reverse-video space, and the block is right there.
                _ => assert!(
                    blocks > 0,
                    "{ctx}: the IBM PC frame's side border must stay a solid block — it is a \
                     reverse-video SPACE in the game's own output, and thinning it would be the \
                     same defect in the other direction"
                ),
            }
        }
    }
}

/// (h) SQ-0747 — the menu header's labels are WHOLE at the pane the user captured, and
/// no uploaded image covers the row they are on.
///
/// The report is a truncated `The Pa` in hybrid. Four investigations swept 18414, 960,
/// 22000 and 800 configurations for it without a reproduction; this pins the null result
/// at the exact frame the user's `/dump-windows` describes — pane origin, transcript and
/// all — so the search space never has to be re-swept from scratch, and so any future
/// change that DOES start eating the labels fails here.
///
/// Both halves matter. The cells carry the labels, and nothing the terminal composites
/// above the cells covers their row: an art strip is an uploaded image, and under kitty
/// an image draws over the text layer, which is the one mechanism a cell-level assertion
/// on its own cannot see.
#[test]
fn journey_menu_header_labels_are_whole_at_the_users_pane() {
    for profile in [InterpreterProfile::Amiga, InterpreterProfile::IbmPc] {
        let Some(mut session) = journey_at_menu(profile) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            // The user's own pane, and the two they have since reported from — 157
            // bad, 159 good. Every sighting of this label eaten (`─The─`, `The Par`,
            // `The Pa`, and finally "The Party" gone entirely) differs only in how
            // many of its leading columns are covered, so the case has to sweep the
            // widths rather than pin one.
            for pane in [USER_PANE, (1, 1, 157, 61), (1, 1, 159, 61)] {
                let (state, area, buf) = render_pane(&model, honor, pane, &transcript);
                let ctx = format!("{profile:?} honor={honor} pane {}x{}", pane.2, pane.3);
                let rows: Vec<String> = (area.y..area.bottom()).map(|y| pane_row(&buf, area, y)).collect();
                let (idx, header) = rows
                    .iter()
                    .enumerate()
                    .find(|(_, r)| r.contains("The Party"))
                    .unwrap_or_else(|| panic!("{ctx}: the menu header is on screen\n{}", rows.join("\n")));
                assert!(
                    header.contains("Individual Commands"),
                    "{ctx}: the header carries BOTH labels whole — got {header:?}"
                );
                let header_row = area.y + idx as u16;
                // …and nothing the terminal composites above the cells covers that
                // row. Asked of the PLACEMENTS, not of the strip records: a Menu-plan
                // flank's picture is drawn at a rect the panel derives, not at the
                // strip's own, so a strip-level test could not see the one band whose
                // rows the user reports over the menu (SQ-0747).
                let placed: Vec<Quad> = {
                    let gr = state.graphics_render.borrow();
                    gr.ops()
                        .iter()
                        .filter_map(|o| match o {
                            app::render::graphics::GraphicsOp::Place { at, .. } => Some(*at),
                            _ => None,
                        })
                        .collect()
                };
                for p in &placed {
                    assert!(
                        header_row < p.1 || header_row >= p.1 + p.3,
                        "{ctx}: a band placed at {p:?} covers the menu header row {header_row} — \
                         an uploaded image draws ABOVE the terminal cells, so the labels would be \
                         eaten on screen while the buffer still holds them"
                    );
                }
            }
        }
    }
}

// ── SQ-0747 item (A) / SQ-0758: the flank's own extent ──
//
// The user, on the left picture column: *"maybe we should start by fixing the left
// side graphics overlap so it renders within it's alloted space?"* Two symptoms, one
// region, opposite directions — and one calculation behind both, which is why they are
// pinned together here: the panel was bounded by the BAND (everything between the pane
// edge and the story viewport) instead of by its own two border columns. So it ran past
// the inner rule at one end and buried the outer border at the other.

/// The panel colour Journey paints around its picture, which the flank flood samples.
const PANEL_BG: ratatui::style::Color = ratatui::style::Color::Rgb(34, 34, 34);

/// (i) SQ-0747(A) — no panel flood stands between the frame's inner rule and the story
/// text.
///
/// A band runs to the story VIEWPORT's edge, and the viewport is quantized INWARD to
/// whole cells while the rule's extension is quantized OUTWARD to the cells its ink
/// covers. Between them there can be a column belonging to neither, and flooding the
/// whole band put the picture column's ground in it — the panel painted past its own
/// rule and up against the prose. Width-dependent by construction: one column at the
/// user's 159- and 163-column panes, none at 138, which is why it came and went.
///
/// Asserted on the CELLS, not on the rects, so it reads as the symptom does. The IBM PC
/// profile rides along as the A/B: its rule is a reverse-video SPACE that inks its whole
/// 8-pixel text cell, so the rule's cells already reach the viewport and there was never
/// a gap there to flood — same code, different geometry, nothing to change.
///
/// FALSIFY by flooding the band again (`let fill = band;` in `menu_flank_panel`):
/// `Amiga honor=true pane 163x61: 46 cell(s) of the picture column's own ground
/// (Rgb(34, 34, 34)) stand between the frame's inner rule (ends at column 68) and the
/// story text (starts at column 69) — the panel is flooding the whole band instead of
/// its own extent. First at (68, 3).`
#[test]
fn journey_flank_panel_fill_stops_at_the_frames_inner_rule() {
    for profile in [InterpreterProfile::Amiga, InterpreterProfile::IbmPc] {
        let Some(mut session) = journey_at_menu(profile) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            for pane in SIDE_PANES {
                let (state, _, buf) = render_pane(&model, honor, pane, &transcript);
                let ctx = format!("{profile:?} honor={honor} pane {}x{}", pane.2, pane.3);
                let vp = viewport_of(&state);
                // The LEFT flank's rule: the divider extension that lies left of the story.
                let Some((rule, _)) = flank_dividers(&state).into_iter().find(|(c, _)| c.0 < vp.0)
                else {
                    panic!("{ctx}: the left flank carries a border rule");
                };
                let rule_end = rule.0 + rule.2;
                let mut stray = 0usize;
                let mut first = None;
                for x in rule_end..vp.0 {
                    for y in rule.1..rule.1 + rule.3 {
                        if buf.cell((x, y)).and_then(|c| c.style().bg) == Some(PANEL_BG) {
                            stray += 1;
                            first.get_or_insert((x, y));
                        }
                    }
                }
                assert_eq!(
                    stray, 0,
                    "{ctx}: {stray} cell(s) of the picture column's own ground ({PANEL_BG:?}) \
                     stand between the frame's inner rule (ends at column {rule_end}) and the \
                     story text (starts at column {}) — the panel is flooding the whole band \
                     instead of its own extent. First at {:?}",
                    vp.0,
                    first.unwrap_or((0, 0))
                );
            }
        }
    }
}

/// (j) SQ-0758 — the flank's OUTER border is drawn, and it is the game's own.
///
/// Under a Menu plan the flank band is never drawn as art: `menu_flank_panel` floods the
/// column and draws only the picture's bounding box, and the frame's outer edge lies
/// outside that box. So Journey's left border simply did not exist between the `┌` on
/// the top rule and the `└` on the bottom one — the honest half of *"unmatched lines on
/// the outside edges"*: one edge was font glyphs, one a bitmap, and one was nothing.
/// The inner rule survived only because its extension redraws it, and the fix is that
/// same extension, run from the other side.
///
/// The A/B that keeps this from being "Amiga is special": under the IBM PC profile there
/// IS no outer border — Journey's picture starts at native x 5 with nothing outside it —
/// so the outward probe finds the ILLUSTRATION, and carrying a one-pixel slice of that
/// down the whole column would be the same defect in the other direction. The graphics-
/// only canvas is what tells them apart, per SQ-0750's rule: a band is art only when it
/// is actually artwork.
///
/// FALSIFY by dropping the outer probe (return `None` for `FlankBorder::Outer`):
/// `Amiga honor=true pane 163x61: the flank's outer border is not drawn at all — no
/// flank-border record left of the story viewport (dividers [((66, 3, 2, 46), ...)])`.
#[test]
fn journey_flank_outer_border_is_drawn_when_the_game_drew_one() {
    for profile in [InterpreterProfile::Amiga, InterpreterProfile::IbmPc] {
        let Some(mut session) = journey_at_menu(profile) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            for pane in SIDE_PANES {
                let (state, _, buf) = render_pane(&model, honor, pane, &transcript);
                let (s, cell_w, _) = scale_and_cell(&state);
                let ctx = format!("{profile:?} honor={honor} pane {}x{}", pane.2, pane.3);
                let vp = viewport_of(&state);
                let outer: Vec<(Quad, Quad)> =
                    flank_records(&state, "flank-border").into_iter().filter(|(c, _)| c.0 < vp.0).collect();
                match profile {
                    InterpreterProfile::Amiga => {
                        let Some((ext, crop)) = outer.first().copied() else {
                            panic!(
                                "{ctx}: the flank's outer border is not drawn at all — no \
                                 flank-border record left of the story viewport (dividers {:?})",
                                flank_dividers(&state)
                            );
                        };
                        // It is the frame's edge, so it stands at the pane's own outer column —
                        // the one the `┌` and `└` on the rules above and below it stand in.
                        assert_eq!(
                            ext.0, pane.0,
                            "{ctx}: the outer border stands in the pane's outer column, with the \
                             frame's corners — got {ext:?}"
                        );
                        // …drawn at the ring's scale, like every other pixel in it — or,
                        // once SQ-0750 landed, not drawn as pixels at all: the game printed
                        // this border as a `│`, so it is stamped as one and there is no
                        // magnification to be wrong about.
                        if !is_glyph(crop) {
                            let mag = (ext.2 as f32 * cell_w as f32) / (crop.2.max(1) as f32);
                            assert!(
                                (mag - s).abs() <= s * 0.2,
                                "{ctx}: the outer border is magnified {mag:.2}x against a letterbox \
                                 scale of {s:.2}x (ext {ext:?}, crop {crop:?})"
                            );
                        }
                        // …and at the pane the user reported, something is visibly there: the
                        // column no longer reads as an unbroken run of the panel's own flood.
                        //
                        // Pinned at that pane only, and deliberately. The border is ONE native
                        // pixel of stroke; whether it survives being resampled into a terminal
                        // cell is the letterbox scale's business, not this fix's — at 100x71
                        // (s=1.25) the stroke averages into the ground exactly, which is
                        // SQ-0750's still-open glyph-versus-raster split and not a border that
                        // failed to draw. What this case owns is that the band is produced,
                        // placed and scaled; the geometry asserts above cover every pane.
                        if pane == USER_PANE {
                            let flooded = (ext.1..ext.1 + ext.3)
                                .filter(|&y| buf.cell((ext.0, y)).and_then(|c| c.style().bg) == Some(PANEL_BG))
                                .count();
                            assert!(
                                flooded < ext.3 as usize,
                                "{ctx}: all {flooded} rows of the outer border column read as the \
                                 panel flood ({PANEL_BG:?}) — the border is not being drawn over it"
                            );
                        }
                    }
                    // No border out there to draw, and the picture is not one.
                    _ => assert!(
                        outer.is_empty(),
                        "{ctx}: this frame has no outer border on the left flank — its picture \
                         runs to the screen edge — so anything drawn there is a slice of the \
                         ILLUSTRATION replicated down the column: {outer:?}"
                    ),
                }
            }
        }
    }
}

// ── SQ-0747, second pass: the fill stops SHORT of the borders, not level with them ──
//
// The user, on frames whose layout is otherwise right: *"the amiga build border lines
// around the art have the artwork's background color"* — and, correcting the obvious
// reading themselves, *"this background color is not part of the artwork pixels
// themselves (artwork fits). it is the fill color that matches the artwork."*
//
// So it is `menu_flank_panel`'s FILL, not the picture. The bound added above was
// inclusive at both ends: the fill began at the band's own left edge, which is the
// column the OUTER border stands in, and ran through the INNER rule's last column. A
// border reaches the screen as an image cropped to its whole text cell (SQ-0750), and a
// box glyph's padding is transparent, so the panel's ground showed through around both
// strokes and the frame's sides read in the picture's colour while its top and bottom
// read in the game's.

/// The panes the fill/border cases sweep. Wide, and one column apart around the sizes
/// the user has reported from, because the defect this file keeps finding is always the
/// one that comes and goes as the letterbox rounding falls — a fix pinned to 157 columns
/// would not be a fix.
const FILL_WIDTHS: [u16; 12] = [96, 110, 130, 138, 150, 155, 156, 157, 158, 159, 163, 170];
/// …at three pane heights, since the flank's rows come out of the same rounding.
const FILL_HEIGHTS: [u16; 3] = [51, 61, 68];

/// Which Journey this case is looking at — and they are DIFFERENT GAMES, not one game
/// under two profiles.
///
/// `journey-r83-s890706.z6` is release 83; `Journey - The Quest Begins.adf` is the Amiga
/// release FLOPPY, release 30, serial 890322. `InterpreterProfile::resolve` reads the
/// medium and picks Amiga off the disk, and the two releases narrate through different
/// windows and lay their menu out differently. **The user plays the floppy**, and every
/// investigation of this frame before 2026-08-10 drove the `.z6` — measuring a build
/// they never see. Both are pinned here; the floppy is the one that has to hold.
#[derive(Clone, Copy, Debug)]
enum JourneyBuild {
    /// The in-repo `.z6`, under an explicitly chosen profile.
    Z6(InterpreterProfile),
    /// The original Amiga release floppy, mounted the way the app mounts it.
    Floppy,
}

/// Every Journey build the flank cases sweep.
const BUILDS: [JourneyBuild; 3] =
    [JourneyBuild::Z6(InterpreterProfile::Amiga), JourneyBuild::Z6(InterpreterProfile::IbmPc), JourneyBuild::Floppy];

/// Boot one of them and drive it to the command menu. `None` (with a SKIP note) when
/// the gitignored fixture is absent.
fn journey_build(build: JourneyBuild) -> Option<GameSession> {
    match build {
        JourneyBuild::Z6(profile) => journey_at_menu(profile),
        JourneyBuild::Floppy => journey_floppy_at_menu(),
    }
}

/// Journey mounted straight off its Amiga release floppy — story, artwork and profile
/// all resolved from the medium, exactly as `startup` does it.
fn journey_floppy_at_menu() -> Option<GameSession> {
    journey_floppy(40)
}

/// …driven `steps` inputs from boot, or stopped early at the command menu. `steps = 0`
/// is the BOOT frame — the one the user's non-perturbing capture describes, and a
/// different band composition from the play frame (no `menu:art` at all at 115x61).
fn journey_floppy(steps: usize) -> Option<GameSession> {
    let path = stories_dir().join("Journey - The Quest Begins.adf");
    let story_bytes = match app::hints::load_story(&path) {
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
        story_bytes,
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
    // SQ-1393: the machine's own colour table. `new_with_trace` is the
    // no-machine door and presents §8.3.1's own, so a harness that boots a
    // press states the press's table here.
    session.machine.set_palette(profile.palette());
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..steps {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if r.transcript.contains("Praxix") || r.transcript.contains("magical resources") {
            break;
        }
    }
    Some(session)
}

/// A hybrid render through a real kitty picker, so a border column's cell keeps the
/// GROUND something painted under it: kitty places an image by writing placeholder
/// symbols and leaves the cell's background alone, which is exactly what the terminal
/// then shows through the glyph's transparent padding. Halfblocks draws the image INTO
/// the cell and would answer a colour question with the picker's own arithmetic.
fn render_pane_kitty(
    model: &app::engine::ScreenModel,
    honor: bool,
    pane: Quad,
    transcript: &str,
) -> (app::state::AppState, Rect, Buffer) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(app::render::graphics::kitty_picker(8, 18));
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

fn rects_overlap(a: Quad, b: Quad) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// Every border column this frame drew, either side of the story: the inner rules and
/// the outer edges together.
fn flank_border_columns(state: &app::state::AppState) -> Vec<Quad> {
    flank_records(state, "flank-divider")
        .into_iter()
        .chain(flank_records(state, "flank-border"))
        .map(|(c, _)| c)
        .collect()
}

/// (k) SQ-0747 — the panel's ground never reaches either of the flank's border columns.
///
/// The geometry half, swept across 48 panes per profile per honour mode, because a
/// bound that is right at one rounding and wrong at the next is what this whole quest
/// has been made of.
///
/// FALSIFY by restoring the inclusive bound in `menu_flank_panel`
/// (`lo = band.x`, `hi = inner.right()`):
/// `Amiga honor=true pane 96x51: the panel fill (1, 3, 39, 42) covers the flank's own
/// border column (39, 3, 1, 42) — a border is not part of the panel …`.
#[test]
fn journey_flank_panel_fill_stops_short_of_both_border_columns() {
    for build in BUILDS {
        let Some(mut session) = journey_build(build) else { continue };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            for h in FILL_HEIGHTS {
                for w in FILL_WIDTHS {
                    let (state, _, _) = render_pane(&model, honor, (1, 1, w, h), &transcript);
                    let ctx = format!("{build:?} honor={honor} pane {w}x{h}");
                    let fills = flank_records(&state, "flank-panel");
                    let borders = flank_border_columns(&state);
                    for (fill, _) in &fills {
                        for b in &borders {
                            assert!(
                                !rects_overlap(*fill, *b),
                                "{ctx}: the panel fill {fill:?} covers the flank's own border \
                                 column {b:?} — a border is not part of the panel, and the fill's \
                                 colour shows through the transparent padding of the glyph drawn \
                                 over it"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// (l) …and the colour half, which is what the user actually sees: no cell of either
/// border column carries the panel's own ground.
///
/// Read through a kitty picker, the shipped hybrid backend, so the cell under the
/// border image is the ground the terminal composites the glyph's transparent padding
/// against. The panel colour is sampled from the fill itself rather than hardcoded —
/// it is the picture's own, and the picture is the game's.
///
/// FALSIFY by restoring the inclusive bound: `Amiga honor=true pane 157x61: 47 of the
/// 47 cells in the flank's border column (64, 3, 1, 47) stand on the picture panel's
/// own ground (Rgb(34, 34, 34))`.
#[test]
fn journey_flank_border_columns_do_not_stand_on_the_panels_ground() {
    for build in BUILDS {
        let Some(mut session) = journey_build(build) else { continue };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            for pane in [(1, 1, 115, 61), (1, 1, 157, 61), (1, 1, 159, 61), USER_PANE] {
                let (state, _, buf) = render_pane_kitty(&model, honor, pane, &transcript);
                let ctx = format!("{build:?} honor={honor} pane {}x{}", pane.2, pane.3);
                let Some((fill, _)) = flank_records(&state, "flank-panel").first().copied() else {
                    continue; // no panel on this frame — nothing to bleed
                };
                // The panel's own colour, off its last filled row: below the picture, so
                // it is the flood and not the illustration.
                let ground = buf
                    .cell((fill.0, fill.1 + fill.3 - 1))
                    .and_then(|c| c.style().bg)
                    .expect("the flank panel floods its own extent, so its ground is a colour");
                for b in flank_border_columns(&state) {
                    let on_panel = (b.0..b.0 + b.2)
                        .flat_map(|x| (b.1..b.1 + b.3).map(move |y| (x, y)))
                        .filter(|&(x, y)| buf.cell((x, y)).and_then(|c| c.style().bg) == Some(ground))
                        .count();
                    assert_eq!(
                        on_panel, 0,
                        "{ctx}: {on_panel} of the {} cells in the flank's border column {b:?} \
                         stand on the picture panel's own ground ({ground:?})",
                        b.2 as usize * b.3 as usize
                    );
                }
            }
        }
    }
}

/// (n) SQ-0747 — `/dump-windows` names every band the pixel path places.
///
/// It did not, and that is what cost two passes on this quest. A Menu-plan flank's
/// picture is drawn by `draw_chrome_band_stretched`, at a dest rect the panel derives
/// rather than the strip's own — and only `draw_chrome_band` was writing to the band
/// log. So every capture the user sent listed the right-hand flank and the bottom strip
/// and NO left flank at all, and two investigations reasoned about the picture column
/// from the strip rect beside it: *"the left flank is not in the band list at all, at
/// this or any earlier capture"*. A diagnostic that cannot see the draw it exists to
/// diagnose is worse than none.
///
/// FALSIFY by dropping the `band_log.push` from `draw_chrome_band_stretched`:
/// `Amiga honor=true pane 157x61: a band was placed at (6, 12, 55, 28) that
/// /dump-windows' band list does not name …`.
#[test]
fn every_placed_band_is_named_in_the_window_dump() {
    for build in BUILDS {
        let Some(mut session) = journey_build(build) else { continue };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            for pane in [(1, 1, 115, 61), (1, 1, 157, 61), (1, 1, 159, 61), USER_PANE] {
                let (state, _, _) = render_pane(&model, honor, pane, &transcript);
                let ctx = format!("{build:?} honor={honor} pane {}x{}", pane.2, pane.3);
                let log = state.graphics_render.borrow().band_log.clone();
                let placed: Vec<Quad> = {
                    let gr = state.graphics_render.borrow();
                    gr.ops()
                        .iter()
                        .filter_map(|o| match o {
                            app::render::graphics::GraphicsOp::Place { at, .. } => Some(*at),
                            _ => None,
                        })
                        .collect()
                };
                assert!(!placed.is_empty(), "{ctx}: this frame places bands at all");
                for p in &placed {
                    let named = format!("band {}x{}@({},{})", p.2, p.3, p.0, p.1);
                    assert!(
                        log.iter().any(|l| l.starts_with(&named)),
                        "{ctx}: a band was placed at {p:?} that /dump-windows' band list does \
                         not name — the dump cannot see the draw it exists to diagnose\n{log:#?}"
                    );
                }
            }
        }
    }
}

/// (p) SQ-0747 — THE EATEN MENU LABELS, on the release the user actually plays.
///
/// Five passes chased `─The─`, `─Individual C─`, `The Par` and `The Pa` through
/// 18414, then 960, then 22000, then 800 configurations and every one came back clean,
/// because every one of them drove `journey-r83-s890706.z6`. **The user plays the Amiga
/// release FLOPPY, release 30**, and release 30 draws this row differently: it prints
/// the rule first and the title over it, so the row carries dozens of `─` fragments AND
/// one run per letter of the title at overlapping native columns. The letters split the
/// rule into groups too short to BE a rule, those fragments took the divider path and
/// were stamped at their own scaled columns, and the columns land inside a title that
/// advanced at the other rate. `The P` at 115 columns, `The Pa` at 157 — the count
/// varying with the pane, which is the signature every sighting had.
///
/// FALSIFY, either half of the fix in `draw_chrome_text_strip`, verbatim at the first
/// pane swept. The lone-glyph `over_word` guard, dropped — the rule fragments punch
/// through the titles:
/// `floppy honor=true pane 96x51: the menu header is missing a label — got
/// "│──────────────────────The─Party───────────────────────Individual C─mmands─────────────────────│"`.
/// The rule's own left edge, back to the immediately preceding run — the titles' tails
/// are painted over wholesale:
/// `floppy honor=true pane 96x51: the menu header is missing a label — got
/// "│──────────────────────The ────────────────────────────Individual Co───────────────────────────│"`.
#[test]
fn journey_release_30_menu_header_labels_are_whole() {
    let Some(mut session) = journey_floppy_at_menu() else { return };
    let transcript = session.take_transcript();
    let model = session.screen();
    for honor in [true, false] {
        for h in FILL_HEIGHTS {
            for w in FILL_WIDTHS {
                let (_, area, buf) = render_pane(&model, honor, (1, 1, w, h), &transcript);
                let ctx = format!("floppy honor={honor} pane {w}x{h}");
                let rows: Vec<String> = (area.y..area.bottom()).map(|y| pane_row(&buf, area, y)).collect();
                // Found by the tail of the right-hand label, which no sighting has ever
                // eaten — so the row is located even when the left one is in pieces.
                let header = rows
                    .iter()
                    .find(|r| r.contains("ndividual"))
                    .unwrap_or_else(|| panic!("{ctx}: the menu header is on screen\n{}", rows.join("\n")));
                assert!(
                    header.contains("The Party") && header.contains("Individual Commands"),
                    "{ctx}: the menu header is missing a label — got {header:?}"
                );
            }
        }
    }
}

/// (q) SQ-0780 — …and the rule ABUTS each of those labels, on both sides.
///
/// The user, at a 159-column terminal: *"starting at 159 width a blank space is added
/// after 'Individual Commands' and the horizontal line."* One blank cell between the
/// right-hand title and the rule that continues past it, while `The Party`'s rule
/// abutted its title at the same width — and that asymmetry is the whole mechanism.
///
/// Release 30 draws the header rule first and the two titles over it, and a stray `─`
/// survives inside each title's native span (`The Party` runs native 152..224 with one
/// at 176, `Individual Commands` runs 368..520 with one at 448). A title is positioned
/// through the letterbox scale and then advances ONE TERMINAL COLUMN per character; a
/// fragment is positioned through the scale and stops. Past about 1.9 columns per
/// native cell the second rate outruns the first, and 80 native pixels into a
/// 19-character title that is a whole column: the stray landed one past the title's
/// last drawn cell, too far right for SQ-0747's over-a-word guard, and the rule behind
/// it — which starts no further left than the last thing DRAWN — began one further
/// right again. `The Party`'s stray is only 24 native pixels in and still lands inside
/// its nine drawn columns at every width swept, so it was suppressed and its rule
/// abutted. Not a regression from the SQ-0750 frame fix: 3b21009d, the commit before
/// it, shows the identical gap at 159x64.
///
/// The pane widths here bracket the onset exactly — 154 is clean, 155 is the first
/// width that shows it, 156 is clean again, and every width from 157 up shows it, which
/// is why one width proves nothing in this lineage.
///
/// FALSIFY by dropping the `under_label` filter from `collapse_row_rules`' divider
/// branch: `floppy honor=true pane 155x51: the menu header's rule does not reach
/// "Individual Commands" — one blank cell stands between the label and the rule …`.
#[test]
fn journey_release_30_menu_header_rule_abuts_both_labels() {
    let Some(mut session) = journey_floppy_at_menu() else { return };
    let transcript = session.take_transcript();
    let model = session.screen();
    for honor in [true, false] {
        for h in FILL_HEIGHTS {
            for w in FILL_WIDTHS {
                let (_, area, buf) = render_pane(&model, honor, (1, 1, w, h), &transcript);
                let ctx = format!("floppy honor={honor} pane {w}x{h}");
                let rows: Vec<String> = (area.y..area.bottom()).map(|y| pane_row(&buf, area, y)).collect();
                let header: Vec<char> = rows
                    .iter()
                    .find(|r| r.contains("ndividual"))
                    .unwrap_or_else(|| panic!("{ctx}: the menu header is on screen"))
                    .chars()
                    .collect();
                let text: String = header.iter().collect();
                for label in ["The Party", "Individual Commands"] {
                    let at = text[..text.find(label).unwrap_or_else(|| panic!("{ctx}: {label:?} is on screen"))]
                        .chars()
                        .count();
                    for (side, c) in [("before", at.checked_sub(1)), ("after", Some(at + label.chars().count()))] {
                        let Some(cell) = c.and_then(|i| header.get(i)).copied() else { continue };
                        assert_eq!(
                            cell,
                            '─',
                            "{ctx}: the menu header's rule does not reach {label:?} — one blank \
                             cell stands {side} the label and the rule. A rule is a DISTANCE the \
                             game drew across, and a leftover fragment the game printed the label \
                             over must not push its edge off the label's own last column.\n{}",
                            text
                        );
                    }
                }
            }
        }
    }
}

/// (o) SQ-0747 — the picture is drawn INSIDE the flank that holds it, on the boot frame
/// as well as the play frame.
///
/// The user's capture at pane 115x61 (scale 1.43) reads `win3 … cells: 45x22 at (2,2)`
/// against a left flank of `48x50 at (1,3)`, which looks like the picture's top row
/// escaping one row above its column — *"the top is peeking out"*. It is not a draw: a
/// window's `cells:` line is the diagnostic MAPPING of its native rect onto the pane,
/// pushed for every chrome window, and in the ring a chrome Graphics leaf is rasterized
/// into the canvas and reaches the screen only through the strips (which is why the dump
/// labels it `rasterised into the ring`). What is actually drawn is the flank panel's
/// dest, and this case asserts on THAT — over the widths where the mapping does escape
/// and the ones where it does not, so the difference is pinned as harmless rather than
/// re-inferred every pass.
///
/// Boot frame AND play frame: they compose different bands (at 115x61 the boot frame has
/// no `menu:art` at all), so a bound that holds in one need not hold in the other.
#[test]
fn journey_flank_picture_is_drawn_inside_its_own_flank() {
    for steps in [0usize, 40] {
        let Some(mut session) = journey_floppy(steps) else { continue };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            for pane in [(1, 1, 115, 61), (1, 1, 138, 68), (1, 1, 157, 61), USER_PANE] {
                let (state, _, _) = render_pane(&model, honor, pane, &transcript);
                let ctx = format!("floppy steps={steps} honor={honor} pane {}x{}", pane.2, pane.3);
                let strips: Vec<Quad> = state
                    .v6_cell_map
                    .borrow()
                    .iter()
                    .filter(|e| e.label == "strip:art")
                    .map(|e| e.cells)
                    .collect();
                for art in flank_records(&state, "flank-art").into_iter().map(|(c, _)| c) {
                    let inside = strips.iter().any(|s| {
                        art.0 >= s.0 && art.1 >= s.1 && art.0 + art.2 <= s.0 + s.2 && art.1 + art.3 <= s.1 + s.3
                    });
                    assert!(
                        inside,
                        "{ctx}: the flank picture is drawn at {art:?}, which no flank strip \
                         {strips:?} contains — the column's art has left the column"
                    );
                }
            }
        }
    }
}

/// (m) …and the standing invariant the earlier passes could not check, now that a
/// stretched band reports itself: NOTHING the pixel path places lands on the menu.
///
/// The user's *"the left side (graphics) are overruning and showing up in the menu below
/// as garbage"*, stated as a rule rather than as one pane. An uploaded image composites
/// ABOVE the cell layer, so a band whose rows reach the command strip eats the menu's
/// text whatever the buffer says is written there — which is why every cell-level sweep
/// of the truncated header came back clean. Asserted on the PLACEMENTS, across the same
/// 48 panes per profile per honour mode.
#[test]
fn journey_no_pixel_band_is_placed_on_the_menu_rows() {
    for build in BUILDS {
        let Some(mut session) = journey_build(build) else { continue };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            for h in FILL_HEIGHTS {
                for w in FILL_WIDTHS {
                    let (state, _, _) = render_pane(&model, honor, (1, 1, w, h), &transcript);
                    let ctx = format!("{build:?} honor={honor} pane {w}x{h}");
                    let menus: Vec<Quad> = state
                        .v6_cell_map
                        .borrow()
                        .iter()
                        .filter(|e| e.label.starts_with("menu:text"))
                        .map(|e| e.cells)
                        .collect();
                    let placed: Vec<Quad> = {
                        let gr = state.graphics_render.borrow();
                        gr.ops()
                            .iter()
                            .filter_map(|o| match o {
                                app::render::graphics::GraphicsOp::Place { at, .. } => Some(*at),
                                _ => None,
                            })
                            .collect()
                    };
                    for p in &placed {
                        for m in &menus {
                            assert!(
                                !rects_overlap(*p, *m),
                                "{ctx}: an uploaded band placed at {p:?} covers the command \
                                 menu's own rows {m:?} — an image composites above the cells, so \
                                 the menu's labels are eaten however whole the buffer holds them"
                            );
                        }
                    }
                }
            }
        }
    }
}
