//! Debug harness: reconstruct a live window layout from `/dump-windows` output
//! and render it with multi-line placeholder text, so multi-window rendering
//! (alignment, per-pane fills, right-edge margin) can be reproduced and inspected
//! deterministically WITHOUT driving a real game. (SQ-0332)
//!
//! `model_from_dump` parses the exact text `/dump-windows` prints; `render_to_text`
//! renders a parsed model into an area and returns a plain-text grid (one char per
//! cell) plus a per-cell background map, for eyeballing in a test.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::engine::{BufferWindow, GridWindow, ScreenModel, Split, StatusModel, WinNode};

#[derive(Clone, Copy, PartialEq)]
enum LeafTy {
    Grid,
    Buffer,
    Graphics,
}

enum PNode {
    Leaf { ty: LeafTy, id: u32, primary: bool, bg: Option<u32>, fg: Option<u32> },
    Pair { vertical: bool, border: bool, split: u16, key_bg: Option<u32> },
}

/// Extract `key=#RRGGBB` → `Some(0x00RRGGBB)` from a dump line's tokens.
fn hex_after(line: &str, prefix: &str) -> Option<u32> {
    line.split_whitespace()
        .find_map(|t| t.strip_prefix(prefix))
        .and_then(|h| u32::from_str_radix(h.trim_start_matches('#'), 16).ok())
}

/// Parse the `(WxH)` from the `Window layout (WxH):` header line.
fn parse_size(header: &str) -> (u16, u16) {
    let inner = header.split('(').nth(1).and_then(|s| s.split(')').next()).unwrap_or("0x0");
    let mut it = inner.split('x');
    let w = it.next().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let h = it.next().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    (w, h)
}

fn parse_line(line: &str) -> (usize, PNode) {
    let depth = (line.len() - line.trim_start().len()) / 2;
    let t = line.trim_start();
    if t.starts_with("Pair") {
        let vertical = t.split_whitespace().any(|w| w == "vertical");
        // "border" vs "no-border": match the standalone token only.
        let border = t.split_whitespace().any(|w| w == "border");
        let split = t
            .split_whitespace()
            .find_map(|w| w.strip_prefix("split="))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        (depth, PNode::Pair { vertical, border, split, key_bg: hex_after(t, "key=") })
    } else {
        let ty = if t.starts_with("Grid") {
            LeafTy::Grid
        } else if t.starts_with("Buffer") {
            LeafTy::Buffer
        } else {
            LeafTy::Graphics
        };
        let id = t
            .split_whitespace()
            .find_map(|w| w.strip_prefix("id="))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let primary = t.contains("(primary)");
        (depth, PNode::Leaf { ty, id, primary, bg: hex_after(t, "bg="), fg: hex_after(t, "fg=") })
    }
}

/// A non-primary buffer filled with `n` lines of visible placeholder text.
fn filled_buffer(id: u32, n: usize, bg: Option<u32>, fg: Option<u32>, primary: bool) -> BufferWindow {
    let lines: Vec<String> = if primary {
        Vec::new() // primary content comes from the transcript, not here
    } else {
        (0..n).map(|k| format!("w{id} line {k}")).collect()
    };
    let len = lines.len();
    BufferWindow {
        win: id,
        runs: vec![Vec::new(); len],
        para: vec![crate::state::ParaFmt::default(); len],
        images: vec![None; len],
        lines,
        scroll: 0,
        primary,
        bg,
        fg,
        panel: false,
        px_runs: Vec::new(),
        reads_input: false,
    }
}

/// A grid whose first row reads visible placeholder text.
fn filled_grid(id: u32, bg: Option<u32>) -> GridWindow {
    let text = format!("grid{id} status");
    let mut g = GridWindow::default();
    g.resize(1, text.chars().count().max(1) as u16);
    for (i, ch) in text.chars().enumerate() {
        g.put(1, i as u16 + 1, ch, 0);
    }
    g.active_rows = 1;
    g.bg = bg;
    g
}

fn build(nodes: &[(usize, PNode)], idx: &mut usize, n: usize) -> WinNode {
    let (_, node) = &nodes[*idx];
    *idx += 1;
    match node {
        PNode::Leaf { ty, id, primary, bg, fg } => match ty {
            LeafTy::Buffer => WinNode::Buffer(filled_buffer(*id, n, *bg, *fg, *primary)),
            LeafTy::Grid => WinNode::Grid(filled_grid(*id, *bg)),
            LeafTy::Graphics => {
                // Solid magenta so the harness exercises the graphics-as-cells path
                // (a game's divider chrome) and it's visible in the rendered grid.
                let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 255, 255]));
                WinNode::Graphics(crate::engine::GraphicsWindow { win: *id, canvas: std::sync::Arc::new(img), version: 1, upscale: false })
            }
        },
        PNode::Pair { vertical, border, split, key_bg } => {
            let (vertical, border, split, key_bg) = (*vertical, *border, *split, *key_bg);
            let first = build(nodes, idx, n);
            let second = build(nodes, idx, n);
            WinNode::Pair {
                vertical,
                split: Split { fixed: split },
                border,
                key_bg,
                key_fg: None,
                first: Box::new(first),
                second: Box::new(second),
            }
        }
    }
}

/// Reconstruct a `ScreenModel` from `/dump-windows` text, filling each text
/// window with `lines_per_win` lines of placeholder content.
pub fn model_from_dump(dump: &str, lines_per_win: usize) -> ScreenModel {
    let mut lines = dump.lines().filter(|l| !l.trim().is_empty());
    let header = lines.next().unwrap_or("Window layout (0x0):");
    let content_size = parse_size(header);
    let parsed: Vec<(usize, PNode)> = lines.map(parse_line).collect();
    let mut idx = 0;
    let root = if parsed.is_empty() { WinNode::Blank } else { build(&parsed, &mut idx, lines_per_win) };
    // The page bg is the primary buffer's own bg (as `AppGlk::screen_model` does).
    fn primary_bg(n: &WinNode) -> Option<u32> {
        match n {
            WinNode::Buffer(b) if b.primary => b.bg,
            WinNode::Pair { first, second, .. } => primary_bg(first).or_else(|| primary_bg(second)),
            _ => None,
        }
    }
    let pbg = primary_bg(&root);
    let bg = match pbg {
        Some(rgb) => crate::state::pack_zcolour(zvm::screen::ZColour::True24(rgb)),
        None => crate::state::pack_zcolour(zvm::screen::ZColour::Default),
    };
    ScreenModel { root, status: StatusModel::HostManaged, bg, fg: 0, content_size }
}

/// Render a parsed dump model into `area` and return `(rows, buf)` where each
/// `rows[y]` is a `Vec<char>` of exactly `area.width` cells (char-indexed so
/// multi-byte box glyphs don't break slicing) and `buf` is the raw buffer.
pub fn render_to_text(dump: &str, area: Rect, lines_per_win: usize) -> (Vec<Vec<char>>, Buffer) {
    let model = model_from_dump(dump, lines_per_win);
    let mut state = crate::state::AppState {
        colors: crate::colors::ColorScheme::terminal_default(),
        ..Default::default()
    };
    state.config.honor_game_colours = true;
    // A dump records the origins a REAL session laid out, and that session reserved
    // a gutter for every bordered pair. Since SQ-0821 the gutter is a theme
    // decision, defaulting to none — so a harness that replays a dump must ask for
    // the borders the dump was taken with, or every origin below the first split
    // reads one row high and the harness stops reproducing the thing it captured.
    state.colors.upper_window_border_sides =
        crate::render::paneframe::PaneSides::all(crate::render::paneframe::BorderStyle::Single);
    for k in 0..lines_per_win {
        state.push_transcript(&format!("MAIN line {k}"));
    }
    let mut buf = Buffer::empty(area);
    crate::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let rows: Vec<Vec<char>> = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect()
        })
        .collect();
    (rows, buf)
}

/// A row's cells `[from, to)` as a `String`, for readable asserts/prints.
#[cfg(all(test, feature = "t-render"))]
fn span(row: &[char], from: usize, to: usize) -> String {
    row.iter().skip(from).take(to.saturating_sub(from)).collect()
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    // The Kerkerkruip 125×60 layout the user captured (bg omitted → no colours set
    // in that state), trimmed of the "(primary)" spacing for the parser.
    const DUMP_125: &str = "Window layout (125x60):
Pair  horizontal  border  split=93
  Pair  horizontal  border  split=91
    Pair  horizontal  border  split=27
      Pair  horizontal  border  split=1
        Graphics id=65  1x60 @(0,0)
        Pair  horizontal  border  split=23
          Pair  vertical  border  split=58
            Pair  vertical  border  split=1
              Buffer id=71  23x1 @(2,0)
              Pair  vertical  border  split=1
                Graphics id=73  23x1 @(2,2)
                Buffer id=63  23x54 @(2,4)
            Graphics id=69  23x1 @(2,59)
          Graphics id=67  1x60 @(26,0)
      Pair  vertical  border  split=1
        Grid id=75  63x1 @(28,0)
        Pair  vertical  border  split=1
          Graphics id=77  63x1 @(28,2)
          Buffer id=4 (primary)  63x56 @(28,4)
    Graphics id=47  1x60 @(92,0)
  Pair  horizontal  border  split=29
    Pair  vertical  border  split=1
      Buffer id=49  29x1 @(94,0)
      Pair  vertical  border  split=1
        Graphics id=51  29x1 @(94,2)
        Pair  vertical  border  split=42
          Pair  vertical  border  split=40
            Buffer id=43  29x40 @(94,4)
            Graphics id=55  29x1 @(94,45)
          Pair  vertical  border  split=11
            Pair  vertical  border  split=1
              Buffer id=59  29x1 @(94,47)
              Pair  vertical  border  split=1
                Graphics id=61  29x1 @(94,49)
                Buffer id=53  29x7 @(94,51)
            Graphics id=57  29x1 @(94,59)
    Graphics id=45  1x60 @(124,0)";

    #[test]
    fn dump_parses_to_expected_geometry() {
        let m = model_from_dump(DUMP_125, 3);
        assert_eq!(m.content_size, (125, 60));
        // The primary buffer exists in the tree.
        fn has_primary(n: &WinNode) -> bool {
            match n {
                WinNode::Buffer(b) => b.primary,
                WinNode::Pair { first, second, .. } => has_primary(first) || has_primary(second),
                _ => false,
            }
        }
        assert!(has_primary(&m.root), "id=4 primary present");
    }

    // Render at NATIVE width (125): every non-primary pane's placeholder text must
    // appear at its dumped origin, proving multi-line content renders in place.
    #[test]
    fn native_width_renders_pane_text() {
        let (rows, _buf) = render_to_text(DUMP_125, Rect::new(0, 0, 125, 60), 3);
        // buf63 (left body @2,4) shows "w63 line 0" on row 4 starting at col 2.
        assert!(span(&rows[4], 2, 20).starts_with("w63 line 0"), "left body row 4: {:?}", span(&rows[4], 0, 40));
        // buf43 (right upper @94,4) shows "w43 line 0" on row 4 at col 94.
        assert!(span(&rows[4], 94, 112).starts_with("w43 line 0"), "right body row 4: {:?}", span(&rows[4], 92, 125));
        // buf53 (lower-right @94,51) shows its text on row 51.
        assert!(span(&rows[51], 94, 112).starts_with("w53 line 0"), "lower-right row 51: {:?}", span(&rows[51], 92, 125));
    }

    // The graphics divider columns (Kerkerkruip's rules) render as thin line glyphs
    // in the game's colour, not image blits or full-cell blocks: col 0 (gfx id=65, a
    // 1×60 vertical rule) is a │ in the magenta the harness fills those windows with.
    #[test]
    fn graphics_dividers_render_as_thin_rules() {
        use ratatui::style::Color;
        let (rows, buf) = render_to_text(DUMP_125, Rect::new(0, 0, 125, 60), 3);
        for y in 0..60u16 {
            assert_eq!(rows[y as usize][0], '\u{2502}', "col 0 vertical divider is a │ rule");
            assert_eq!(
                buf.cell((0, y)).unwrap().style().fg,
                Some(Color::Rgb(255, 0, 255)),
                "col 0 graphics divider drawn in its own colour (magenta) at row {y}"
            );
        }
    }

    // Render into a WIDER area (165) than the dump's 125: reproduce the user's
    // "extra space on the right". content_bounds clamps the tree to 125 and blanks
    // cols 125..165 — this test documents that margin so the fix can target it.
    #[test]
    fn wider_area_leaves_right_margin() {
        let (rows, _buf) = render_to_text(DUMP_125, Rect::new(0, 0, 165, 60), 3);
        let margin = span(&rows[4], 125, 165);
        eprintln!("--- 165-wide render, row 4 full ---\n{}", span(&rows[4], 0, 165));
        eprintln!("--- margin cols 125..165 ---\n[{}]", margin);
        assert!(margin.trim().is_empty(), "right margin blank on row 4: {:?}", margin);
    }
}
