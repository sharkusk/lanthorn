//! SQ-0761: `/dump-cells` writes the rendered cell buffer — glyphs AND styling —
//! as plain text, and taking it through a bound key does not disturb the frame it
//! describes.
//!
//! Two halves, both measured here.
//!
//! **The dump has to be lossless about STYLE.** Every Journey defect chased in the
//! session that asked for this was a colour landing in a cell — a panel fill nine
//! rows deep under the menu, border cells carrying the fill's colour instead of the
//! frame's, a label the buffer held and the screen did not. `/dump-windows` reports
//! geometry and could show none of them. So the cases below drive the real Amiga
//! release to its command menu, render it at the pane the report came from, and
//! check that every cell's colour survives the round trip into text and back.
//!
//! **And taking it must not move what it reports** (SQ-0759's lesson, which this
//! command inherits rather than relearns): reached through the palette, a cell dump
//! would describe the palette — not a stale frame, a completely different picture,
//! since a modal is drawn straight over the cells.

use std::path::PathBuf;

use app::cell_dump::{format_cell_dump, is_graphics_cell, DumpMeta};
use app::config::KeymapConfig;
use app::engine::Engine;
use app::graphics::PictSource;
use app::input::{key_to_command, KeyResolve};
use app::interpreter::InterpreterProfile;
use app::keymap::{Context, KeyMap, KeySpec};
use app::session::{GameSession, InputKind};
use app::state::{AppState, Focus, TranscriptKind};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// The pane the capture is taken at.
const PANE: Rect = Rect { x: 0, y: 0, width: 115, height: 61 };

/// The Amiga release, on its release floppy — release 30 / serial 890322, a
/// DIFFERENT BUILD from `journey-r83-s890706.z6` and the one the open menu defects
/// are reported against (see CLAUDE.md, "a disk image is a different release").
const AMIGA_RELEASE: &str = "Journey - The Quest Begins.adf";

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

#[allow(deprecated)]
fn fresh_state() -> AppState {
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = true;
    state
}

/// Boot the Amiga release off its floppy and press through to the command menu,
/// accumulating the transcript the way `turn::apply_game_driven_result` does —
/// Journey's prose never reaches the story window's `lines`, it arrives as the
/// session transcript and is drawn from `AppState` (SQ-0755).
fn journey_at_menu() -> Option<(GameSession, AppState)> {
    let story_path = stories_dir().join(AMIGA_RELEASE);
    let story_bytes = match app::hints::load_story(&story_path) {
        Ok(app::hints::LoadedStory::ZCode(b)) => b,
        _ => {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return None;
        }
    };
    let profile = InterpreterProfile::Amiga;
    let mut picts = PictSource::resolve(&story_path, None);
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

    let mut state = fresh_state();
    state.push_transcript_runs(&session.take_transcript(), TranscriptKind::Story, &[]);
    for _ in 0..40 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if r.transcript_elems.is_empty() {
            state.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
        } else {
            app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
        }
        if state.transcript.iter().any(|l| l.contains("Praxix")) {
            break;
        }
    }
    Some((session, state))
}

/// Render the menu frame at the capture pane, and dump it.
fn menu_frame() -> Option<(AppState, Buffer, Vec<String>)> {
    let (session, state) = journey_at_menu()?;
    let model = session.screen();
    let mut buf = Buffer::empty(PANE);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, PANE, &mut buf);
    state.note_frame_cells(&buf);
    let lines = state.last_frame_cells.borrow().as_ref().expect("a frame was filed").lines();
    Some((state, buf, lines))
}

// ── (a) The real frame: the styling survives, exactly ─────────────────────────

/// The property the whole command rests on: for every cell of a REAL Journey frame,
/// the style key printed under its glyph resolves — through the legend — back to
/// that cell's actual foreground, background and attributes.
///
/// A dump that rounds colours, or coalesces two nearly-equal styles, would read
/// perfectly and answer the wrong question. This checks the round trip on 7015 live
/// cells rather than on a synthetic pair.
#[test]
fn every_cell_of_a_real_frame_round_trips_through_the_legend() {
    let Some((_state, buf, lines)) = menu_frame() else { return };

    let legend = parse_legend(&lines);
    let (glyphs, keys) = parse_grid(&lines);
    assert_eq!(glyphs.len(), PANE.height as usize, "one glyph row per terminal row");
    assert_eq!(keys.len(), PANE.height as usize, "one style row per glyph row");

    let mut checked = 0u32;
    for y in 0..PANE.height {
        for x in 0..PANE.width {
            let cell = buf.cell((x, y)).unwrap();
            let key = keys[y as usize][x as usize];
            // A picture rendered INTO cells gives every pixel pair its own style, so
            // the tail past the legend limit shares one mark. Those cells are counted
            // and bounded by the bucket line, not named — see LEGEND_LIMIT.
            if key == '*' {
                continue;
            }
            let got = legend
                .get(&key)
                .unwrap_or_else(|| panic!("style key {key:?} at ({x},{y}) is not in the legend"));
            assert_eq!(got.0, color_text(cell.fg), "({x},{y}) fg: the legend must name the cell's real fg");
            assert_eq!(got.1, color_text(cell.bg), "({x},{y}) bg: the legend must name the cell's real bg");
            if cell.modifier.is_empty() {
                assert_eq!(got.2, "-", "({x},{y}) carries no attributes, so the legend must not add any");
            } else {
                assert_ne!(got.2, "-", "({x},{y}) carries attributes and the legend must say so");
            }
            checked += 1;
        }
    }
    // The bucket must stay a tail. It holds the halfblock-rendered picture and
    // nothing else on this frame; if it ever swallowed the chrome the dump would
    // stop answering the question it exists for.
    let total = u32::from(PANE.width) * u32::from(PANE.height);
    assert!(
        checked * 100 / total >= 85,
        "the named legend must cover the frame, not a corner of it: {checked}/{total} cells"
    );
    assert!(
        lines.iter().any(|l| l.contains("further style(s)") && l.contains("cells  rows")),
        "and the tail it did not name must state its own count and extent"
    );
}

/// The glyph grid has to read as text — a label is what tells you WHERE in the
/// frame a colour landed, and the menu labels are what the reports name.
#[test]
fn the_glyph_grid_reads_as_the_screen_does() {
    let Some((_state, _buf, lines)) = menu_frame() else { return };
    let (glyphs, _) = parse_grid(&lines);
    let text: Vec<String> = glyphs.iter().map(|r| r.iter().collect()).collect();
    // The command menu's own entries and the party roster, straight off the grid.
    // (The menu HEADER's labels are truncated by the rule glyphs at this pane —
    // which the dump shows too, and which is somebody else's quest.)
    for label in ["Proceed", "Praxix", "Inventory", "Individual Comm"] {
        assert!(
            text.iter().any(|r| r.contains(label)),
            "the menu header's {label:?} must be readable in the dump:\n{}",
            text.join("\n")
        );
    }
}

/// The dump must survive a copy-paste as PLAIN TEXT. An escape sequence in it would
/// paste as the same soup that makes the screen itself uncopyable (SQ-0756), and a
/// v6 frame is where the graphics protocol lives — so a real one is the place to
/// check, not a synthetic pair of cells.
#[test]
fn a_real_v6_frame_dumps_without_one_escape_sequence() {
    let Some((_state, _buf, lines)) = menu_frame() else { return };
    for (i, line) in lines.iter().enumerate() {
        assert!(!line.contains('\u{1b}'), "line {i} carries an escape: {line:?}");
        assert!(!line.contains('\u{10eeee}'), "line {i} carries a kitty placeholder: {line:?}");
    }
}

/// Every region an uploaded image covers is named. Under kitty an image draws ABOVE
/// the terminal cells, so those cells are not blank — they are hidden, which is a
/// different thing and the one a reader has to be told about (SQ-0747).
#[test]
fn the_regions_an_image_covers_are_named_not_left_blank() {
    let Some((state, buf, lines)) = menu_frame() else { return };
    let head = lines.iter().take_while(|l| !l.starts_with("styles:")).cloned().collect::<Vec<_>>();
    assert!(
        head.iter().any(|l| l.starts_with("graphics:")),
        "the dump must open with the graphics it excluded:\n{}",
        head.join("\n")
    );
    // Every graphics cell in the buffer lies inside one of the reported rects.
    let rects: Vec<(u16, u16, u16, u16)> = head
        .iter()
        .filter_map(|l| l.split_once("cells (").map(|(_, r)| r.to_string()))
        .filter_map(|r| {
            let (xy, wh) = r.split_once(") ")?;
            let (x, y) = xy.split_once(',')?;
            let (w, h) = wh.trim().split_once('x')?;
            Some((x.parse().ok()?, y.parse().ok()?, w.parse().ok()?, h.parse().ok()?))
        })
        .collect();
    for y in 0..PANE.height {
        for x in 0..PANE.width {
            if !buf.cell((x, y)).is_some_and(is_graphics_cell) {
                continue;
            }
            assert!(
                rects.iter().any(|&(rx, ry, rw, rh)| x >= rx && x < rx + rw && y >= ry && y < ry + rh),
                "({x},{y}) is covered by an image and no reported rect contains it"
            );
        }
    }
    // The renderer's own art placements ride along, and they are the only signal a
    // backend that paints WITHOUT escape cells leaves — halfblocks and sixel write
    // ordinary coloured cells, and the headless harness here is one such. Without
    // this half, a picture region would read as ordinary text on those backends.
    let images = state.last_frame_cells.borrow().as_ref().unwrap().images.clone();
    assert!(!images.is_empty(), "this frame records art strips, and the dump carries them");
    for img in &images {
        let (x, y, w, h) = img.rect;
        assert!(
            head.iter().any(|l| l.contains(&format!("at ({x},{y}) {w}x{h}"))),
            "the placement {} at ({x},{y}) {w}x{h} must be named:\n{}",
            img.label,
            head.join("\n")
        );
    }
}

/// The line the panel-fill investigations needed. "These nine rows all carry the
/// panel fill's background" was, every time, a fact reconstructed from a screenshot;
/// here it is one line, naming the colour and the row range.
///
/// Measured against the frame both ways round, so the section can neither miss a row
/// nor invent one: every row the BUFFER says is a single background end to end must
/// appear under that colour, and every row the section claims must really be one.
#[test]
fn a_background_that_owns_whole_rows_states_the_range() {
    let Some((_state, buf, lines)) = menu_frame() else { return };

    let claimed = parse_row_backgrounds(&lines);
    assert!(!claimed.is_empty(), "this frame has rows of one background, and they are stated");

    for y in 0..PANE.height {
        let bg = buf.cell((0, y)).unwrap().bg;
        let uniform = (0..PANE.width).all(|x| buf.cell((x, y)).unwrap().bg == bg);
        let stated = claimed.iter().find(|(_, rows)| rows.contains(&y));
        match (uniform, stated) {
            (true, Some((named, _))) => assert_eq!(
                *named,
                color_text(bg),
                "row {y} is entirely {} and the dump names another colour",
                color_text(bg)
            ),
            (true, None) => panic!("row {y} is entirely {} and the dump does not say so", color_text(bg)),
            (false, Some((named, _))) => {
                panic!("row {y} is not one background, but the dump claims {named}")
            }
            (false, None) => {}
        }
    }

    // And the same fact, from the other direction: the style that owns whole rows
    // says so in the legend too, so a reader scanning either section finds it.
    assert!(
        lines.iter().any(|l| l.contains("FULL rows ")),
        "a frame with uniform rows has at least one style owning one:\n{}",
        lines.iter().take(20).cloned().collect::<Vec<_>>().join("\n")
    );
}

// ── (b) The bound key, and the frame it must not move ─────────────────────────

/// A state whose keymap is exactly what the user typed into `config.toml`.
fn state_bound(entries: &[(&str, &str)]) -> (AppState, Vec<String>) {
    let mut cfg = KeymapConfig::default();
    for (key, cmd) in entries {
        cfg.global.insert((*key).to_string(), (*cmd).to_string());
    }
    let (keymap, warnings) = KeyMap::resolve(&cfg);
    let mut state = fresh_state();
    state.keymap = keymap;
    state.focus = Focus::Game;
    (state, warnings)
}

/// The documented binding resolves, and it resolves to the command itself with no
/// modal opened on the way.
///
/// A **Ctrl** key specifically: the run loop hands every plain keypress to a story
/// waiting on one, and Journey — the game this exists for — waits on a keypress at
/// every screen. Reaching a Ctrl binding means clearing `hotkeys.is_direct_name`,
/// which is why `dump-cells` joined the direct set.
///
/// FALSIFY by dropping `"dump-cells"` from `DEFAULT_DIRECT_COMMANDS`: this resolves
/// to `KeyResolve::None` and the key does nothing at all.
#[test]
fn the_documented_ctrl_binding_dispatches_with_no_modal() {
    let (state, warnings) = state_bound(&[("ctrl+g", "dump-cells")]);
    assert!(warnings.is_empty(), "the template's own line must resolve cleanly: {warnings:?}");
    assert_eq!(
        state.keymap.lookup(&"ctrl+g".parse::<KeySpec>().unwrap(), Context::Global),
        Some("dump-cells")
    );
    assert!(
        state.hotkeys.is_direct_name("dump-cells"),
        "dump-cells must be directly available, or no Ctrl binding for it can ever fire"
    );
    match key_to_command(&state, KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)) {
        KeyResolve::Command(cmd, ctx) => {
            assert_eq!(cmd, "dump-cells");
            assert_eq!(ctx, Context::Global);
        }
        other => panic!("Ctrl+G must dispatch the command, got {other:?}"),
    }
    assert!(!state.any_modal_overlay_open(), "the bound key must not open a modal");

    // …and from map focus, which applies the same direct-set filter.
    let (mut map_focused, _) = state_bound(&[("ctrl+g", "dump-cells")]);
    map_focused.focus = Focus::Map;
    assert!(matches!(
        key_to_command(&map_focused, KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)),
        KeyResolve::Command(ref c, _) if c == "dump-cells"
    ));
}

/// The measurement SQ-0759 established, applied to this command: a bound-key capture
/// moves neither the render-path history nor the band-upload count, and the frame it
/// describes is the one on screen.
#[test]
fn a_bound_key_capture_leaves_the_frame_it_reports_untouched() {
    let Some((session, transcript_state)) = journey_at_menu() else { return };
    let model = session.screen();

    let (mut state, _) = state_bound(&[("ctrl+g", "dump-cells")]);
    state.transcript = transcript_state.transcript.clone();
    let draw = |state: &AppState| {
        let mut buf = Buffer::empty(PANE);
        let _ = app::render::screen::render_story_pane(&model, false, None, state, PANE, &mut buf);
        state.note_frame_cells(&buf);
    };

    // Settle: the first frames upload the chrome bands, later ones hit the cache.
    for _ in 0..4 {
        draw(&state);
    }
    let paths_before = state.v6_path_log.borrow().clone();
    let uploads_before = state.graphics_render.borrow().band_encodes;
    assert!(uploads_before > 0, "precondition: this frame uploads bands");

    // The capture: the key resolves straight to the command, and reading the
    // snapshot is all the command does.
    assert!(matches!(
        key_to_command(&state, KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)),
        KeyResolve::Command(ref c, _) if c == "dump-cells"
    ));
    let lines = state.last_frame_cells.borrow().as_ref().expect("a frame is on record").lines();
    assert!(lines.len() > PANE.height as usize, "the dump is a real grid, not a stub");

    assert_eq!(
        state.graphics_render.borrow().band_encodes,
        uploads_before,
        "a bound-key capture must upload no bands"
    );
    assert_eq!(
        state.v6_path_log.borrow().clone(),
        paths_before,
        "a bound-key capture must add no render path to the history"
    );
    assert!(
        lines.iter().any(|l| l.contains("the frame on screen now")),
        "no modal frame stands between the dump and the frame it reports:\n{}",
        lines.iter().take(3).cloned().collect::<Vec<_>>().join("\n")
    );
}

/// The contrast that gives the case above its meaning — and the reason this command
/// cannot be a palette-only one. A modal is drawn straight OVER the cells, so a dump
/// taken through the palette does not merely report a stale frame: it reports the
/// palette's own box, sitting where the game's picture was.
#[test]
fn the_palette_frame_never_becomes_the_dump() {
    let mut state = fresh_state();
    state.push_transcript("GAME FRAME");
    let mut game = Buffer::empty(Rect::new(0, 0, 12, 2));
    game.set_string(0, 0, "GAME FRAME  ", ratatui::style::Style::default());
    state.note_frame_cells(&game);

    // The palette opens and draws its own frames over the top.
    state.overlays.palette = Some(app::state::PaletteState::new(false));
    let mut modal = Buffer::empty(Rect::new(0, 0, 12, 2));
    modal.set_string(0, 0, "PALETTE BOX ", ratatui::style::Style::default());
    for _ in 0..3 {
        state.note_frame_cells(&modal);
    }

    let frame = state.last_frame_cells.borrow().clone().expect("the game frame is still on record");
    let text = frame.lines().join("\n");
    assert!(text.contains("GAME FRAME"), "the game's own cells are what gets dumped:\n{text}");
    assert!(!text.contains("PALETTE BOX"), "the modal's cells must never reach the dump:\n{text}");
    assert!(
        text.contains("3 modal frame(s) ago"),
        "and the dump must say how stale it is:\n{text}"
    );
}

/// With nothing drawn yet there is no frame, and the command has to say so rather
/// than invent one.
#[test]
fn an_empty_area_dumps_as_empty_rather_than_as_a_grid() {
    let buf = Buffer::empty(Rect::new(0, 0, 0, 0));
    let lines = format_cell_dump(&buf, Rect::new(0, 0, 0, 0), &DumpMeta::default());
    assert!(lines.iter().any(|l| l.contains("empty area")), "{lines:?}");
    assert!(AppState::default().last_frame_cells.borrow().is_none(), "no frame before the first draw");
}

// ── Parsing the dump back, so the assertions read it the way a human does ──────

/// The legend, as `key → (fg, bg, attrs)`.
fn parse_legend(lines: &[String]) -> std::collections::HashMap<char, (String, String, String)> {
    let mut out = std::collections::HashMap::new();
    for l in lines {
        let Some(rest) = l.strip_prefix("  ") else { continue };
        let mut it = rest.split_whitespace();
        let (Some(key), Some("fg"), Some(fg), Some("on"), Some("bg"), Some(bg), Some("attrs"), Some(attrs)) =
            (it.next(), it.next(), it.next(), it.next(), it.next(), it.next(), it.next(), it.next())
        else {
            continue;
        };
        if key.chars().count() != 1 {
            continue;
        }
        out.insert(key.chars().next().unwrap(), (fg.to_string(), bg.to_string(), attrs.to_string()));
    }
    out
}

/// The interleaved grid, split back into its glyph rows and its style-key rows.
fn parse_grid(lines: &[String]) -> (Vec<Vec<char>>, Vec<Vec<char>>) {
    let mut glyphs = Vec::new();
    let mut keys = Vec::new();
    for l in lines {
        if let Some((_, row)) = l.split_once(" g|") {
            glyphs.push(row.chars().collect());
        } else if let Some((_, row)) = l.split_once(" s|") {
            keys.push(row.chars().collect());
        }
    }
    (glyphs, keys)
}

/// The `row backgrounds:` section, as `(colour, rows)`.
fn parse_row_backgrounds(lines: &[String]) -> Vec<(String, Vec<u16>)> {
    lines
        .iter()
        .filter_map(|l| l.strip_prefix("  bg "))
        .filter_map(|l| l.split_once(" — rows "))
        .map(|(color, rows)| (color.to_string(), expand_ranges(rows)))
        .collect()
}

/// `"3-5,9"` → `[3, 4, 5, 9]`.
fn expand_ranges(spec: &str) -> Vec<u16> {
    let mut out = Vec::new();
    for part in spec.split(',') {
        match part.trim().split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.parse::<u16>(), b.parse::<u16>()) {
                    out.extend(a..=b);
                }
            }
            None => out.extend(part.trim().parse::<u16>().ok()),
        }
    }
    out
}

/// The dump's own colour spelling, so the assertions compare like with like.
fn color_text(c: Color) -> String {
    match c {
        Color::Reset => "default".to_string(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(n) => format!("idx{n}"),
        other => format!("{other:?}").to_lowercase(),
    }
}
