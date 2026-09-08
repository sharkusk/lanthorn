//! Shogun's `Score:` / `Moves:` fields line up at every pane width — SQ-0757.
//!
//! The report, playing Shogun off its Amiga release floppy: *"the score: and moves:
//! in the score area are only aligned if the width is 82 or 83 (without map pane).
//! they always align and right justify with the ibmpc version."*
//!
//! The game right-justifies them itself, in native pixels, and puts BOTH labels at
//! native x 503 and both values at native x 586 — so the two rows are aligned before
//! the renderer ever sees them, and any disagreement about where those columns are
//! is ours. Measured across the pane, the Amiga route placed them like this (hybrid,
//! `honor_game_colours = true`, the shipped default):
//!
//! | pane | `Score:` col | `Moves:` col |
//! |------|--------------|--------------|
//! |   76 |           61 |           62 |
//! |   80 |           63 |           63 |
//! |   82 |           64 |           63 |
//! |   90 |           68 |           63 |
//! |  120 |           85 |           66 |
//!
//! One column of the two moves with the pane and the other barely moves at all, so
//! they coincide over a two-column window and nowhere else — which is exactly the
//! shape the report describes.
//!
//! The cause is not the profile. It is what the profile makes the GAME do: under
//! interpreter 4 Shogun paints its status band one run per CELL, padding included,
//! and [`merge_strip_fragments`]'s predecessor glued each row's padding onto the
//! field behind it. A glued run is positioned once through the letterbox scale and
//! then advances one terminal column per character — two rates that agree only where
//! a terminal column IS a native 8px cell — so row 0 (glued from native x 351) and
//! row 1 (glued from native x 47) disagreed about where x 503 lands by an amount
//! that grows with the pane. The same game under the IBM PC profile emits one run
//! per FIELD with no padding at all, so nothing was ever glued: the report's own
//! control, and it is pinned below beside the defect.
//!
//! Swept across a RANGE of widths, deliberately including panes narrower and much
//! wider than the game's own 80 columns — a fix that only holds where the two rates
//! coincide is the bug in another costume — and across pane heights, both
//! `honor_game_colours` modes, and both graphics modes (hybrid primary, raster
//! secondary).
//!
//! **Two different releases.** `James Clavell's Shogun.adf` is release 295 / serial
//! 890321; `shogun-r322-s890706.z6` is release **322** / serial 890706 (CLAUDE.md,
//! "a disk image is a different release"). Both are swept under both profiles, so
//! neither the release nor the profile can be the thing that holds the fix up.
//! `Shogun.blb` is a resource-only Blorb — it carries no executable — so it is not a
//! story fixture at all.
//!
//! The stories are gitignored, so every case skips cleanly when absent.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The Amiga release floppy the report was filed against — release 295 / serial
/// 890321, a DIFFERENT BUILD from the bare story file below.
const AMIGA_RELEASE: &str = "James Clavell's Shogun.adf";
/// The IBM PC control the report names — release 322 / serial 890706.
const PC_RELEASE: &str = "shogun-r322-s890706.z6";

/// Pane widths swept by every case. 80 is the game's own screen, where a terminal
/// column and a native 8px text cell coincide; 82 and 83 are the two the report
/// found correct (an 80-column story pane inside the frame's borders); the rest lie
/// either side, out to a pane far wider than the game was written for.
const WIDTHS: [u16; 14] = [70, 74, 76, 78, 80, 81, 82, 83, 84, 88, 96, 110, 124, 140];

/// Pane heights. 51 is the ordinary terminal; 30 and 71 cover the short and tall
/// regimes, where the letterbox scale stops being width-limited.
const HEIGHTS: [u16; 3] = [30, 51, 71];

/// Every fixture/profile pairing: the report's own configuration, its control, and
/// the two crossings — so a fix that leant on either the release or the profile
/// would be caught.
const CASES: [(&str, InterpreterProfile); 4] = [
    (AMIGA_RELEASE, InterpreterProfile::Amiga),
    (PC_RELEASE, InterpreterProfile::IbmPc),
    (AMIGA_RELEASE, InterpreterProfile::IbmPc),
    (PC_RELEASE, InterpreterProfile::Amiga),
];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Shogun under `profile` and drive it into gameplay, where the status band
/// carries a location, a score and a move count.
fn shogun_in_play(name: &str, profile: InterpreterProfile, honor: bool) -> Option<GameSession> {
    let story_path = stories_dir().join(name);
    let story_bytes = match app::hints::load_story(&story_path) {
        Ok(app::hints::LoadedStory::ZCode(b)) => b,
        _ => {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
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
    .expect("Shogun (v6) should load and boot without a ZError");
    // SQ-1393: the machine's own colour table. `new_with_trace` is the
    // no-machine door and presents §8.3.1's own, so a harness that boots a
    // press states the press's table here.
    session.machine.set_palette(profile.palette());
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    // Enter takes START off the boot menu; a couple of turns then put a location, a
    // score and a move count in the band.
    for turn in 0..6 {
        let _ = match session.pending_input() {
            InputKind::Line => session.submit(if turn % 2 == 0 { "look" } else { "wait" }),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
    }
    Some(session)
}

/// A hybrid render at a real kitty-ish cell (8x18). `Picker::halfblocks()` reports a
/// 1x2 cell, a layout regime that never reproduces a scale defect at all (SQ-0548).
#[allow(deprecated)]
fn render_hybrid(model: &app::engine::ScreenModel, honor: bool, cols: u16, rows: u16) -> (Rect, Buffer) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    (area, buf)
}

fn row_text(buf: &Buffer, area: Rect, y: u16) -> String {
    (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
}

/// The terminal row and COLUMN a status label was stamped at.
///
/// Column, not byte offset. `str::find` answers in bytes, which is the same number
/// only while every cell on the row holds an ASCII glyph — and a status row that
/// crosses the frame's own ornament does not: SQ-0894 lets a flank own the band's
/// rows in its own columns, so the row begins with half-block art (`▀`/`▄`, three
/// bytes each) and every byte offset past it is inflated by two per glyph. That is
/// what this suite's swept assertion caught first, and it was measuring the ruler,
/// not the thing: the two labels really do share column 64 at an 81x30 pane, and
/// their byte offsets are 74 and 76 because the two rows begin with five and six
/// art glyphs.
fn find_label(buf: &Buffer, area: Rect, label: &str) -> Option<(u16, usize)> {
    let want: Vec<char> = label.chars().collect();
    (0..area.height).find_map(|y| {
        let row: Vec<char> = row_text(buf, area, y).chars().collect();
        row.windows(want.len()).position(|w| w == want.as_slice()).map(|c| (y, c))
    })
}

/// The rightmost inked column of a terminal row — where the row's right-justified
/// field ENDS. The band is flooded with spaces, so "inked" is "not a space"; the
/// frame's own ornament is not the row's ink either (see [`find_label`]).
fn right_edge(buf: &Buffer, area: Rect, y: u16) -> Option<usize> {
    let row: Vec<char> = row_text(buf, area, y).chars().collect();
    row.iter().rposition(|&c| c != ' ' && !is_frame_art(c))
}

/// A half-block the ring drew for the frame's side artwork, not a glyph the game
/// printed.
///
/// Block Elements ONLY (U+2580..U+259F), deliberately — this is NOT `screen.rs`'s
/// `is_box_glyph`, which starts at U+2500 and takes in Box Drawing as well. Widening
/// it to match would be a defect: Box Drawing is exactly what a v6 game prints when
/// it draws its frame with CHARACTERS rather than artwork (Journey's `│` rules under
/// the Amiga profile, which SQ-0750 keeps as glyphs on purpose), and those are the
/// row's own ink. What the half-block backend emits for a rasterised image is `▀`
/// and `▄` and nothing else, so the narrow range is the one that separates "the ring
/// drew this" from "the game printed this".
fn is_frame_art(c: char) -> bool {
    ('\u{2580}'..='\u{259F}').contains(&c)
}

// ── The defect, swept ─────────────────────────────────────────────────────────

/// `Score:` and `Moves:` start in the SAME terminal column, and their values END in
/// the same one, at every pane width and height — under both interpreter profiles,
/// on both releases, in both colour modes.
///
/// FALSIFY by restoring the plain `merge_row_fragments(&pending, 4)` calls in
/// `collapse_row_rules`: the Amiga cases report `Score:` and `Moves:` in different
/// columns at every width except 80 and 81, which is the reported symptom.
#[test]
fn score_and_moves_share_a_column_at_every_pane_width() {
    for (name, profile) in CASES {
        for honor in [true, false] {
            let Some(session) = shogun_in_play(name, profile, honor) else { return };
            let model = session.screen();
            for rows in HEIGHTS {
                for cols in WIDTHS {
                    let (area, buf) = render_hybrid(&model, honor, cols, rows);
                    let where_ = format!("{name} {profile:?} honor={honor} {cols}x{rows}");
                    let Some((score_row, score_col)) = find_label(&buf, area, "Score:") else {
                        panic!("{where_}: the status band must carry Score:")
                    };
                    let Some((moves_row, moves_col)) = find_label(&buf, area, "Moves:") else {
                        panic!("{where_}: the status band must carry Moves:")
                    };
                    assert_ne!(score_row, moves_row, "{where_}: the two fields are on their own rows");
                    assert_eq!(
                        score_col, moves_col,
                        "{where_}: the game puts both labels at native x 503, so they must land in \
                         one column — not merely where a terminal column happens to be a native cell"
                    );
                    assert_eq!(
                        right_edge(&buf, area, score_row),
                        right_edge(&buf, area, moves_row),
                        "{where_}: the game right-justifies both values at native x 586, so the two \
                         rows must end in the same column"
                    );
                }
            }
        }
    }
}

/// The label must stay LEGIBLE while it is being aligned: splitting a row's runs
/// apart could just as easily drop a field on top of its neighbour. Both fields
/// arrive whole, with their values beside them, at every pane swept.
#[test]
fn the_fields_stay_whole_beside_their_values() {
    for (name, profile) in CASES {
        let Some(session) = shogun_in_play(name, profile, true) else { return };
        let model = session.screen();
        for cols in WIDTHS {
            let (area, buf) = render_hybrid(&model, true, cols, 51);
            let where_ = format!("{name} {profile:?} {cols} cols");
            let (score_row, score_col) = find_label(&buf, area, "Score:").expect(&where_);
            let (moves_row, moves_col) = find_label(&buf, area, "Moves:").expect(&where_);
            // The location and the game's own title share the score row's band and
            // must survive the split too.
            let top = row_text(&buf, area, score_row);
            let bottom = row_text(&buf, area, moves_row);
            assert!(top.contains("Erasmus"), "{where_}: the ship's name stays whole: {top:?}");
            assert!(top.contains("SHOGUN"), "{where_}: the centred title stays whole: {top:?}");
            assert!(bottom.contains("Bridge"), "{where_}: the location stays whole: {bottom:?}");
            // …and a value follows each label rather than being overwritten by it.
            for (row, col, text) in
                [(score_row, score_col, &top), (moves_row, moves_col, &bottom)]
            {
                let tail = &text[col + "Score:".len()..];
                assert!(
                    tail.chars().any(|c| c.is_ascii_digit()),
                    "{where_}: row {row} carries its value after the label: {text:?}"
                );
            }
        }
    }
}

// ── Raster, secondary ─────────────────────────────────────────────────────────

/// The raster composite draws in NATIVE pixels, so the game's own right-justification
/// is preserved there by construction — pinned so a future change to the strip path
/// cannot quietly take the other mode with it. The two band rows' painted ink must
/// end at the same native x.
#[test]
fn the_raster_composite_keeps_both_rows_right_justified() {
    for (name, profile) in CASES {
        for honor in [true, false] {
            let Some(session) = shogun_in_play(name, profile, honor) else { return };
            let model = session.screen();
            let WinNode::Layered(items) = &model.root else { panic!("v6 publishes a Layered root") };

            // The status band's own runs, straight off the model: both rows put their
            // right-hand value at one native x, which is the fact the raster canvas
            // then paints and the cell path had to be taught.
            let mut last_x: std::collections::BTreeMap<u16, u16> = Default::default();
            for pw in items {
                let WinNode::Grid(g) = &pw.node else { continue };
                for t in &g.px_texts {
                    if t.text.trim().is_empty() {
                        continue;
                    }
                    let row = (t.y.max(1) - 1) / 16;
                    if row > 1 {
                        continue;
                    }
                    let end = (t.x.max(1) - 1) + 8 * t.text.chars().count() as u16;
                    let e = last_x.entry(row).or_default();
                    *e = (*e).max(end);
                }
            }
            let where_ = format!("{name} {profile:?} honor={honor}");
            assert_eq!(last_x.len(), 2, "{where_}: the band is two native rows");
            assert_eq!(
                last_x.get(&0),
                last_x.get(&1),
                "{where_}: the game itself right-justifies both rows to one native x — the \
                 premise the cell path has to preserve"
            );

            // And the raster path renders that band without dropping it.
            let mut state = app::state::AppState::default();
            state.colors = app::colors::ColorScheme::terminal_default();
            state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
            state.config.v6_render = app::config::V6RenderMode::Raster;
            state.config.honor_game_colours = honor;
            let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
            let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
            let (canvas, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
            let band_ink = |row: u32| -> Option<u32> {
                let bg = *canvas.get_pixel(0, row * 16);
                (0..canvas.width())
                    .rev()
                    .find(|&x| (0..16).any(|dy| *canvas.get_pixel(x, row * 16 + dy) != bg))
            };
            let (top, bottom) = (band_ink(0), band_ink(1));
            assert!(top.is_some() && bottom.is_some(), "{where_}: both band rows carry ink in raster");
            let (top, bottom) = (top.unwrap(), bottom.unwrap());
            assert!(
                top.abs_diff(bottom) < 8,
                "{where_}: raster draws in native pixels, so the two rows end within one text \
                 cell of each other; got {top} and {bottom}"
            );
        }
    }
}

/// The band's VALUES sit on the band's last column, and nothing paints below it —
/// SQ-1073.
///
/// Shogun right-aligns the score and the move count to native x **586**, which
/// with an 8-px cell ends at 593: exactly the right edge of a 548-px window at
/// native x 47. That final glyph is the one a wrap limit gets wrong, because
/// **548 is 68.5 cells** — quantized to 68 the limit is 544, four pixels short,
/// and the glyph trips a break that sends the value to the next line and out of a
/// two-row band into the story.
///
/// # Why this case boots differently from the rest of the suite
///
/// [`shogun_in_play`] goes through `GameSession::new_with_trace`, which is the
/// honest **no-machine** door: it takes an interpreter number and nothing else,
/// so the session's wrap regime stays `Attributes` whatever profile the case
/// names. That is fine for what those cases measure — they are about where the
/// RENDERER puts columns the game has already aligned — but it means none of them
/// can see this defect, and the full gate was green while the score sat in the
/// story area. A case guarding a MACHINE's behaviour has to boot the way
/// `startup.rs` boots (CLAUDE.md), through `MachineBoot` and `new_for_machine`.
///
/// Asserted on the engine's own painted runs rather than on a rendered pane: the
/// defect is in the model, and both render paths were faithfully drawing it.
#[test]
fn the_band_values_sit_on_its_last_column_and_nothing_paints_below_it() {
    let path = stories_dir().join(AMIGA_RELEASE);
    let Ok((loaded, medium)) = app::hints::load_mounted_story(&path) else {
        eprintln!("SKIP: gitignored medium missing at {}", path.display());
        return;
    };
    let bytes = loaded.bytes().to_vec();
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 295, "{AMIGA_RELEASE}: release");
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), "890321", "{AMIGA_RELEASE}: serial");
    let (profile, source) =
        InterpreterProfile::resolve_with_source(&path, None, None, medium);
    assert_eq!(profile, InterpreterProfile::Amiga, "the floppy names the machine");
    let mut picts = PictSource::resolve(&path, None);
    let dims = picts.all_pict_dims();
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        app::native_font::resolve(&app::native_font::FaceRequest {
            story_path: &path,
            entry: None,
            profile,
            source,
            art_scale: picts.art_scale(),
            disks: Some(&app::system_fonts::UserDisks::new("")),
        }),
        zvm::screen::Palette::Standard,
        None,
    );
    let mut s = GameSession::new_for_machine(bytes, true, false, false, dims, None, None, &boot)
        .expect("Shogun boots off its Amiga floppy");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    for _ in 0..14 {
        match s.pending_input() {
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            _ => {
                s.submit("");
            }
        }
    }
    let r = s.submit("look");
    assert!(r.fault.is_none(), "`look` faulted: {:?}", r.fault);

    let w1 = &s.machine.screen.v6.as_ref().expect("v6").windows[1];
    // The frame's own signature first: a band that stopped being 548 px wide, or a
    // release that stopped right-aligning to 586, must fail here rather than pass
    // vacuously below.
    assert_eq!(
        (w1.x_coord, w1.y_coord, w1.x_size, w1.y_size),
        (47, 1, 548, 32),
        "the Amiga's status band, two rows of a 548-px window",
    );
    let runs: Vec<(u16, u16, String)> =
        w1.texts.iter().map(|t| (t.y, t.x, t.text.clone())).collect();

    // Each of the two rows carries a NUMERAL on the band's last column.
    for row in [1u16, 17] {
        let value = runs
            .iter()
            .find(|(y, x, t)| *y == row && *x == 586 && t.chars().all(|c| c.is_ascii_digit()));
        assert!(
            value.is_some(),
            "row {row}: the right-aligned value belongs at native x=586, on the band's \
             last column; got\n{:#?}",
            runs.iter().filter(|(y, _, _)| *y == row).collect::<Vec<_>>(),
        );
    }

    // …and NOTHING is painted past the band's bottom edge. The defect put a
    // numeral at native y=33, one row below a window that ends at 32.
    let below: Vec<&(u16, u16, String)> = runs.iter().filter(|(y, _, _)| *y > 32).collect();
    assert!(
        below.is_empty(),
        "the band is two rows tall and nothing may wrap out of it; got {below:#?}",
    );
}
