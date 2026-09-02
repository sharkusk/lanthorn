use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::paneframe::{BorderStyle, InsetSegment, PaneGlyphs};
use super::panel::{draw_panel, PanelSpec, PanelStrip};
use crate::theme::resolve::Theme;

// ── centered_rect ─────────────────────────────────────────────────────────────

/// Return a rect of size `w`x`h` centered within `area`, clamped to `area`.
pub fn centered_rect(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    Rect::new(x, y, w, h)
}

/// Resolve the final rect for a centered modal of size `w`x`h` under a placement
/// and margin. `Center` is exactly `centered_rect` (behavior-preserving); edges
/// anchor to the matching side(s) with `margin` cells of gap, with the other axis
/// kept centered. The result is clamped to stay within `area`.
pub fn resolve_dialog_rect(
    placement: DialogPlacement,
    margin: u16,
    w: u16,
    h: u16,
    area: Rect,
) -> Rect {
    use DialogPlacement::*;
    if placement == Center {
        return centered_rect(area, w, h);
    }
    let w = w.min(area.width);
    let h = h.min(area.height);
    // Centered position on each axis (matches centered_rect's formula).
    let cx = area.x + (area.width - w) / 2;
    let cy = area.y + (area.height - h) / 2;
    // Edge-anchored positions with margin.
    let left_x = area.x + margin;
    let right_x = area.right().saturating_sub(w).saturating_sub(margin);
    let top_y = area.y + margin;
    let bottom_y = area.bottom().saturating_sub(h).saturating_sub(margin);
    let (x, y) = match placement {
        Center => (cx, cy), // handled above; unreachable
        Top => (cx, top_y),
        Bottom => (cx, bottom_y),
        Left => (left_x, cy),
        Right => (right_x, cy),
        TopLeft => (left_x, top_y),
        TopRight => (right_x, top_y),
        BottomLeft => (left_x, bottom_y),
        BottomRight => (right_x, bottom_y),
    };
    // Clamp so the rect never spills outside `area`.
    let max_x = area.right().saturating_sub(w);
    let max_y = area.bottom().saturating_sub(h);
    let x = x.clamp(area.x, max_x.max(area.x));
    let y = y.clamp(area.y, max_y.max(area.y));
    Rect::new(x, y, w, h)
}

// ── Placement ─────────────────────────────────────────────────────────────────

pub enum Placement {
    Centered { w: u16, h: u16 },
    Positioned(Rect),
}

// ── DialogPlacement ─────────────────────────────────────────────────────────────

/// Where a centered modal is anchored within the screen. `Center` (the default)
/// reproduces today's behavior exactly; the edges/corners anchor the modal to the
/// matching side(s) with a configurable margin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DialogPlacement {
    #[default]
    Center,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl DialogPlacement {
    /// Parse a placement token. Unknown tokens (and `center`) map to `Center`.
    pub fn from_token(s: &str) -> DialogPlacement {
        match s.trim().to_lowercase().as_str() {
            "top" => DialogPlacement::Top,
            "bottom" => DialogPlacement::Bottom,
            "left" => DialogPlacement::Left,
            "right" => DialogPlacement::Right,
            "top-left" => DialogPlacement::TopLeft,
            "top-right" => DialogPlacement::TopRight,
            "bottom-left" => DialogPlacement::BottomLeft,
            "bottom-right" => DialogPlacement::BottomRight,
            _ => DialogPlacement::Center,
        }
    }

    /// The canonical token for this placement (inverse of `from_token`).
    pub fn as_token(self) -> &'static str {
        match self {
            DialogPlacement::Center => "center",
            DialogPlacement::Top => "top",
            DialogPlacement::Bottom => "bottom",
            DialogPlacement::Left => "left",
            DialogPlacement::Right => "right",
            DialogPlacement::TopLeft => "top-left",
            DialogPlacement::TopRight => "top-right",
            DialogPlacement::BottomLeft => "bottom-left",
            DialogPlacement::BottomRight => "bottom-right",
        }
    }
}

// ── ButtonId ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonId {
    Save,
    SaveGame,
    Cancel,
    Ok,
    Done,
    Close,
    Reset,
    Resume,
    NewGame,
    Archive,
    Global,
    PlayAgain,
    Restore,
    Quit,
    /// The region prompt's three outcomes (SQ-0439): carve these rooms out now, ask again the
    /// next time this passage is crossed, never ask about it again.
    Separate,
    Later,
    Never,
    /// The region prompt's confirm when it is a pick rather than a suggestion.
    MoveRegion,
    /// The keep-this-download prompt's middle answer (SQ-1086): keep the fetched
    /// story AND the file the library already holds under that name, rather than
    /// replacing one with the other.
    KeepBoth,
}

// ── DialogButton ──────────────────────────────────────────────────────────────

pub struct DialogButton {
    pub id: ButtonId,
    pub label: &'static str,
}

// ── DialogStyle ───────────────────────────────────────────────────────────────

pub struct DialogStyle<'a> {
    /// The resolved theme, so `draw_dialog` can drive the shared `draw_panel`
    /// (which resolves terminator caps and border overrides from it).
    pub theme: &'a Theme,
    pub frame: Style,
    /// Border colour for the modal frame (from `dialog.border`), drawn over the
    /// `frame` fill. Distinct from `frame` so the border can differ from the body.
    pub border: Style,
    pub box_style: BorderStyle,
    pub glyphs: PaneGlyphs,
    pub title: Style,
    pub button: Style,
    pub button_active: Style,
    pub shadow: Style,
    pub shadow_on: bool,
    /// Where a centered modal is anchored on screen (default `Center`).
    pub placement: DialogPlacement,
    /// Cells of gap from the anchored edge(s); ignored for `Center`.
    pub margin: u16,
}

impl DialogStyle<'_> {
    /// Build the dialog chrome from a `ColorScheme`. This is the single seam that
    /// every modal uses, so cross-cutting dialog concerns (placement, later
    /// animation) live in one place instead of being duplicated per modal.
    pub fn from_colors(cs: &crate::colors::ColorScheme) -> DialogStyle<'_> {
        DialogStyle {
            theme: &cs.theme,
            frame: cs.theme.get("dialog.background").style,
            // The modal frame is dialog's own border (§2c), single + bold by
            // default; its colour and glyph style both come from `dialog.border`.
            border: cs.theme.get("dialog.border").style,
            box_style: cs
                .theme
                .get("dialog.border")
                .border
                .unwrap_or(crate::render::paneframe::BorderStyle::None),
            glyphs: cs.dialog_glyphs.clone(),
            title: cs.theme.get("dialog.title").style,
            button: cs.theme.get("dialog.button").style,
            button_active: cs.theme.get("dialog.button:active").style,
            shadow: cs.theme.get("dialog.shadow").style,
            shadow_on: cs.dialog_shadow_on,
            placement: cs.dialog_placement,
            margin: cs.dialog_margin,
        }
    }
}

// ── DialogField ───────────────────────────────────────────────────────────────

/// An optional single-line caret text field drawn in the top row of a dialog's
/// content area. Purely additive: a `DialogSpec` with `field: None` renders
/// exactly as before. The caller owns the buffer (a `TextField`) and supplies the
/// display state here; `draw_dialog` returns the field's row rect for hit-testing.
pub struct DialogField<'a> {
    /// Leading label, e.g. `"Name: "`.
    pub label: &'a str,
    /// The current buffer contents to display.
    pub value: &'a str,
    /// Caret position as a char index into `value`.
    pub cursor: usize,
    /// Draw a block caret at `cursor` (typically only when the field is focused
    /// and actively being edited).
    pub show_caret: bool,
    /// Render `value` in the placeholder (dimmed) style rather than the normal
    /// text style — used when the field still holds its greyed default.
    pub dim: bool,
    /// Style for the label and the (non-dim) value text.
    pub text_style: Style,
    /// Style for the value when `dim` (placeholder).
    pub dim_style: Style,
    /// Style for the block caret cell.
    pub caret_style: Style,
}

// ── DialogSpec ────────────────────────────────────────────────────────────────

pub struct DialogSpec<'a> {
    pub title: &'a str,
    pub placement: Placement,
    pub buttons: &'a [DialogButton],
    pub show_close: bool,
    /// The confirm button: rendered underlined; Enter triggers it by default.
    pub default: Option<ButtonId>,
    /// Index into `buttons` to highlight with `button_active` (Tab focus).
    pub focus: Option<usize>,
    /// Optional caret text field drawn in the content's top row. `None` (the
    /// default for every existing dialog) renders identically to before.
    pub field: Option<DialogField<'a>>,
}

// ── DialogRects ───────────────────────────────────────────────────────────────

pub struct DialogRects {
    pub area: Rect,
    pub content: Rect,
    pub close: Option<Rect>,
    pub buttons: Vec<(ButtonId, Rect)>,
    /// Row rect of the optional text field (its whole content row), for click
    /// hit-testing. `None` when the spec carried no field.
    pub field: Option<Rect>,
}

// ── draw_dialog ───────────────────────────────────────────────────────────────

/// Draw a modal dialog. `Placement::Centered` is centered/anchored within
/// `bounds` (usually the caller's region, e.g. a graphics-free dialog area);
/// pass `*buf.area()` to center over the whole terminal.
pub fn draw_dialog(buf: &mut Buffer, bounds: Rect, spec: &DialogSpec, st: &DialogStyle) -> DialogRects {
    // Coerce BorderStyle::None to Single so every modal always has a visible border.
    let box_style = match st.box_style {
        BorderStyle::None => BorderStyle::Single,
        other => other,
    };

    // (1) Resolve area from placement — centered within `bounds`; positioned as-is.
    let buf_area = *buf.area();
    let area = match spec.placement {
        Placement::Centered { w, h } => resolve_dialog_rect(st.placement, st.margin, w, h, bounds),
        Placement::Positioned(r) => r,
    };

    // (2) If shadow_on, paint shadow at +1/+1 offset (bottom+right), clamped to buffer
    if st.shadow_on {
        // Bottom row of shadow: area.y+1 .. area.bottom()+1, col area.right()
        let shadow_right = area.right();
        let shadow_bottom = area.bottom();

        // Right column shadow: rows area.y+1 .. area.bottom(), col area.right()
        if shadow_right < buf_area.right() {
            for row in (area.y + 1)..shadow_bottom.min(buf_area.bottom()) {
                if let Some(cell) = buf.cell_mut((shadow_right, row)) {
                    cell.set_style(st.shadow);
                }
            }
        }

        // Bottom row shadow: cols area.x+1 .. area.right()+1, row area.bottom()
        if shadow_bottom < buf_area.bottom() {
            for col in (area.x + 1)..=(shadow_right.min(buf_area.right().saturating_sub(1))) {
                if let Some(cell) = buf.cell_mut((col, shadow_bottom)) {
                    cell.set_style(st.shadow);
                }
            }
        }
    }

    // (3) Fill area OPAQUE with Style::reset().patch(st.frame)
    let fill_style = Style::reset().patch(st.frame);
    for row in area.y..area.bottom() {
        for col in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((col, row)) {
                cell.set_symbol(" ");
                cell.set_style(fill_style);
            }
        }
    }

    // (4+5) Frame (in dialog.border's colour) + centered title strip via the
    // shared themed panel. `box_style` preserves the None→Single modal coercion
    // above; the title caps now track that border style (via the theme).
    let title_seg = InsetSegment { text: spec.title, active: false };
    let pane = draw_panel(buf, &PanelSpec {
        area,
        border_selector: "dialog.border",
        border_color: Some(st.border),
        border_style: Some(box_style),
        glyphs: &st.glyphs,
        header_on: true,
        strip: Some(PanelStrip { segments: &[title_seg], base: st.title, active: st.title }),
        body_fill: None,
    }, st.theme);

    // (6) If show_close, draw ✕ just inside the top-right border
    let close = if spec.show_close && area.width >= 3 && area.height >= 1 {
        // Just inside top-right: col = area.right()-2, row = area.y
        let cx = area.right().saturating_sub(2);
        let cy = area.y;
        if let Some(cell) = buf.cell_mut((cx, cy)) {
            cell.set_symbol("✕").set_style(st.frame);
        }
        Some(Rect::new(cx, cy, 1, 1))
    } else {
        None
    };

    // (7) If buttons non-empty, draw a right-aligned bottom button row
    let mut button_rects: Vec<(ButtonId, Rect)> = Vec::new();
    let content = if !spec.buttons.is_empty() && pane.content.height > 0 {
        // Reserve the last row of content for buttons
        let button_row_y = pane.content.bottom().saturating_sub(1);
        // Each button is its label in reverse video with a reversed space of
        // padding on each side (no brackets), and a single non-reversed space
        // separating adjacent buttons. Lay out right-to-left.
        let mut col = pane.content.right();
        let n = spec.buttons.len();
        for (rev_i, btn) in spec.buttons.iter().rev().enumerate() {
            let orig_i = n - 1 - rev_i;
            let label_chars = btn.label.chars().count() as u16;
            let btn_width = label_chars + 2; // " " + label + " ", all reversed
            if col < btn_width || col.saturating_sub(btn_width) < pane.content.x {
                break;
            }
            col = col.saturating_sub(btn_width);
            let bx = col;

            // Focused button uses button_active; default button is underlined.
            let mut style = if spec.focus == Some(orig_i) { st.button_active } else { st.button };
            if spec.default == Some(btn.id) {
                style = style.add_modifier(ratatui::style::Modifier::UNDERLINED);
            }

            let btn_str = format!(" {} ", btn.label);
            let mut draw_x = bx;
            for ch in btn_str.chars() {
                if draw_x < pane.content.right() {
                    if let Some(cell) = buf.cell_mut((draw_x, button_row_y)) {
                        let mut tmp = [0u8; 4];
                        cell.set_symbol(ch.encode_utf8(&mut tmp)).set_style(style);
                    }
                    draw_x += 1;
                }
            }

            button_rects.push((btn.id, Rect::new(bx, button_row_y, btn_width, 1)));
            // Leave one non-reversed separator column before the next button.
            col = col.saturating_sub(1);
        }
        button_rects.reverse();

        // Content is frame content minus the button row
        if pane.content.height > 1 {
            Rect::new(
                pane.content.x,
                pane.content.y,
                pane.content.width,
                pane.content.height - 1,
            )
        } else {
            Rect::new(pane.content.x, pane.content.y, pane.content.width, 0)
        }
    } else {
        pane.content
    };

    // (8) Optional caret text field, drawn in the content's top row. Additive:
    // dialogs without a field skip this entirely and render as before.
    let field_rect = spec.field.as_ref().and_then(|f| {
        if content.height == 0 || content.width == 0 {
            return None;
        }
        let row = content.y;
        let mut x = content.x;
        // Label (always in the normal text style).
        for ch in f.label.chars() {
            if x >= content.right() {
                break;
            }
            if let Some(cell) = buf.cell_mut((x, row)) {
                let mut tmp = [0u8; 4];
                cell.set_symbol(ch.encode_utf8(&mut tmp)).set_style(f.text_style);
            }
            x += 1;
        }
        // Value chars; the char at `cursor` carries the caret style when shown.
        let value_style = if f.dim { f.dim_style } else { f.text_style };
        let chars: Vec<char> = f.value.chars().collect();
        // Horizontal scroll (SQ-0554). The field is a single row of `avail` cells,
        // so a longer value has to be viewed through a window. Anchor that window
        // on the caret: the cell being edited must always be on screen. Before
        // this, drawing simply started at char 0 and stopped at `content.right()`,
        // taking the caret with the overflow — so editing a value wider than the
        // dialog left you unable to see either your text or your cursor.
        //
        // `caret` indexes 0..=len: at `len` it is the trailing end-of-text block,
        // which occupies a cell of its own, hence `caret + 1` cells to reach it.
        // While that fits, `start` is 0 and nothing scrolls.
        let avail = content.right().saturating_sub(x) as usize;
        let caret = f.cursor.min(chars.len());
        let start = (caret + 1).saturating_sub(avail);
        for (i, ch) in chars.iter().enumerate().skip(start) {
            if x >= content.right() {
                break;
            }
            let style = if f.show_caret && i == caret { f.caret_style } else { value_style };
            if let Some(cell) = buf.cell_mut((x, row)) {
                let mut tmp = [0u8; 4];
                cell.set_symbol(ch.encode_utf8(&mut tmp)).set_style(style);
            }
            x += 1;
        }
        // Caret at end-of-text: a trailing block cell.
        if f.show_caret && caret >= chars.len() && x < content.right() {
            if let Some(cell) = buf.cell_mut((x, row)) {
                cell.set_symbol(" ").set_style(f.caret_style);
            }
        }
        Some(Rect::new(content.x, row, content.width, 1))
    });

    DialogRects { area, content, close, buttons: button_rects, field: field_rect }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use super::super::paneframe::PaneGlyphs;

    static TEST_THEME: std::sync::LazyLock<crate::theme::resolve::Theme> =
        std::sync::LazyLock::new(|| crate::colors::ColorScheme::terminal_default().theme);

    #[test]
    fn dialog_opaque_bg_covers_underlying_and_records_rects() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Style, Modifier, Color}};
        let full = Rect::new(0,0,40,12);
        let mut buf = Buffer::empty(full);
        // pre-fill a REVERSED cell where the dialog will sit
        buf.cell_mut((20,6)).unwrap().set_symbol("X").set_style(Style::new().add_modifier(Modifier::REVERSED));
        let st = DialogStyle{ theme: &TEST_THEME, frame: Style::new().bg(Color::Black), border: Style::default(), box_style: BorderStyle::Single, glyphs: PaneGlyphs::default(), title: Style::default(), button: Style::default(), button_active: Style::default(), shadow: Style::default(), shadow_on:false , placement: DialogPlacement::Center, margin: 0 };
        let spec = DialogSpec{ title:"Settings", placement: Placement::Centered{w:20,h:8}, buttons: &[DialogButton{id:ButtonId::Save,label:"Save"},DialogButton{id:ButtonId::Cancel,label:"Cancel"}], show_close:true, default: None, focus: None, field: None };
        let r = draw_dialog(&mut buf, full, &spec, &st);
        // opaque: the covered cell no longer REVERSED
        assert!(!buf.cell((20,6)).unwrap().modifier.contains(Modifier::REVERSED));
        assert!(r.close.is_some());
        assert_eq!(r.buttons.len(), 2);
        assert!(r.content.width > 0 && r.content.height > 0);
    }

    #[test]
    fn dialog_centers_within_bounds_not_whole_buffer() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        // Buffer is 40 wide, but the dialog is confined to the right half — as if
        // the left half held a graphics window (SQ-0203).
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 12));
        let bounds = Rect::new(20, 0, 20, 12);
        let st = DialogStyle {
            theme: &TEST_THEME,
            frame: Style::default(), border: Style::default(), box_style: BorderStyle::Single, glyphs: PaneGlyphs::default(),
            title: Style::default(), button: Style::default(), button_active: Style::default(),
            shadow: Style::default(), shadow_on: false, placement: DialogPlacement::Center, margin: 0,
        };
        let spec = DialogSpec {
            title: "T", placement: Placement::Centered { w: 10, h: 4 },
            buttons: &[], show_close: false, default: None, focus: None, field: None,
        };
        let r = draw_dialog(&mut buf, bounds, &spec, &st);
        // Centered within bounds: x = 20 + (20-10)/2 = 25 — not 15 (whole-buffer center).
        assert_eq!(r.area, Rect::new(25, 4, 10, 4));
        // Entirely inside the bounds; never spills into the left (graphics) half.
        assert!(r.area.x >= bounds.x && r.area.right() <= bounds.right());
    }

    #[test]
    fn dialog_placement_token_round_trips_and_defaults() {
        assert_eq!(DialogPlacement::from_token("center"), DialogPlacement::Center);
        assert_eq!(DialogPlacement::from_token("top"), DialogPlacement::Top);
        assert_eq!(DialogPlacement::from_token("bottom"), DialogPlacement::Bottom);
        assert_eq!(DialogPlacement::from_token("left"), DialogPlacement::Left);
        assert_eq!(DialogPlacement::from_token("right"), DialogPlacement::Right);
        assert_eq!(DialogPlacement::from_token("top-left"), DialogPlacement::TopLeft);
        assert_eq!(DialogPlacement::from_token("top-right"), DialogPlacement::TopRight);
        assert_eq!(DialogPlacement::from_token("bottom-left"), DialogPlacement::BottomLeft);
        assert_eq!(DialogPlacement::from_token("bottom-right"), DialogPlacement::BottomRight);
        // Unknown and case/whitespace handling.
        assert_eq!(DialogPlacement::from_token("nonsense"), DialogPlacement::Center);
        assert_eq!(DialogPlacement::from_token("  TOP-LEFT  "), DialogPlacement::TopLeft);
        // Default is Center.
        assert_eq!(DialogPlacement::default(), DialogPlacement::Center);
        // as_token is the inverse for every variant.
        for p in [
            DialogPlacement::Center, DialogPlacement::Top, DialogPlacement::Bottom,
            DialogPlacement::Left, DialogPlacement::Right, DialogPlacement::TopLeft,
            DialogPlacement::TopRight, DialogPlacement::BottomLeft, DialogPlacement::BottomRight,
        ] {
            assert_eq!(DialogPlacement::from_token(p.as_token()), p);
        }
    }

    #[test]
    fn from_colors_matches_inline_build() {
        // Guard: DialogStyle::from_colors must reproduce the themed per-modal
        // build byte-for-byte (SQ-0309: colour fields now read `cs.theme.get(...)`
        // rather than the legacy `ColorScheme` fields directly).
        let cs = crate::colors::ColorScheme::terminal_default();
        let ds = DialogStyle::from_colors(&cs);
        assert_eq!(ds.frame, cs.theme.get("dialog.background").style);
        assert_eq!(ds.box_style, cs.theme.get("dialog.border").border.unwrap());
        assert_eq!(ds.glyphs, cs.dialog_glyphs);
        assert_eq!(ds.title, cs.theme.get("dialog.title").style);
        assert_eq!(ds.button, cs.theme.get("dialog.button").style);
        assert_eq!(ds.button_active, cs.theme.get("dialog.button:active").style);
        assert_eq!(ds.shadow, cs.theme.get("dialog.shadow").style);
        assert_eq!(ds.shadow_on, cs.dialog_shadow_on);
    }

    #[test]
    fn resolve_dialog_rect_center_equals_centered_rect() {
        use ratatui::layout::Rect;
        let area = Rect::new(0, 0, 40, 12);
        // Behavior-preserving: Center is byte-for-byte centered_rect, margin ignored.
        assert_eq!(
            resolve_dialog_rect(DialogPlacement::Center, 0, 20, 8, area),
            centered_rect(area, 20, 8)
        );
        assert_eq!(
            resolve_dialog_rect(DialogPlacement::Center, 5, 20, 8, area),
            centered_rect(area, 20, 8)
        );
    }

    #[test]
    fn resolve_dialog_rect_edges_and_corners_with_margin() {
        use ratatui::layout::Rect;
        let area = Rect::new(0, 0, 40, 12);
        let (w, h, m) = (20u16, 4u16, 2u16);
        // Centered axis values: cx = (40-20)/2 = 10, cy = (12-4)/2 = 4.
        // Edge values with margin 2: left=2, right=40-20-2=18, top=2, bottom=12-4-2=6.
        assert_eq!(resolve_dialog_rect(DialogPlacement::Top, m, w, h, area), Rect::new(10, 2, w, h));
        assert_eq!(resolve_dialog_rect(DialogPlacement::Bottom, m, w, h, area), Rect::new(10, 6, w, h));
        assert_eq!(resolve_dialog_rect(DialogPlacement::Left, m, w, h, area), Rect::new(2, 4, w, h));
        assert_eq!(resolve_dialog_rect(DialogPlacement::Right, m, w, h, area), Rect::new(18, 4, w, h));
        assert_eq!(resolve_dialog_rect(DialogPlacement::TopLeft, m, w, h, area), Rect::new(2, 2, w, h));
        assert_eq!(resolve_dialog_rect(DialogPlacement::TopRight, m, w, h, area), Rect::new(18, 2, w, h));
        assert_eq!(resolve_dialog_rect(DialogPlacement::BottomLeft, m, w, h, area), Rect::new(2, 6, w, h));
        assert_eq!(resolve_dialog_rect(DialogPlacement::BottomRight, m, w, h, area), Rect::new(18, 6, w, h));
    }

    #[test]
    fn resolve_dialog_rect_clamps_within_area() {
        use ratatui::layout::Rect;
        let area = Rect::new(0, 0, 40, 12);
        // A huge margin must not push the rect off-screen. For TopLeft the margin
        // adds to x/y, so it overshoots and clamps to (max_x=20, max_y=8).
        let r = resolve_dialog_rect(DialogPlacement::TopLeft, 100, 20, 4, area);
        assert!(r.right() <= area.right() && r.bottom() <= area.bottom());
        assert_eq!(r, Rect::new(20, 8, 20, 4));
        // For BottomRight the margin subtracts, saturating to the top-left corner;
        // still within bounds.
        let br = resolve_dialog_rect(DialogPlacement::BottomRight, 100, 20, 4, area);
        assert_eq!(br, Rect::new(0, 0, 20, 4));
        // Oversized w/h clamp to the area dimensions.
        let big = resolve_dialog_rect(DialogPlacement::Top, 0, 100, 100, area);
        assert!(big.width <= area.width && big.height <= area.height);
    }

    #[test]
    fn centered_rect_centers_and_clamps() {
        use ratatui::layout::Rect;
        assert_eq!(centered_rect(Rect::new(0,0,40,12), 20, 8), Rect::new(10,2,20,8));
        let big = centered_rect(Rect::new(0,0,10,4), 20, 8); // clamps to area
        assert!(big.width <= 10 && big.height <= 4);
    }

    #[test]
    fn dialog_shadow_paints_offset_cells_when_on() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Style,Color}};
        let mut buf = Buffer::empty(Rect::new(0,0,40,12));
        let st = DialogStyle{ theme: &TEST_THEME, frame: Style::new().bg(Color::Black), border: Style::default(), box_style: BorderStyle::Single, glyphs: PaneGlyphs::default(), title:Style::default(), button:Style::default(), button_active:Style::default(), shadow: Style::new().bg(Color::DarkGray), shadow_on:true , placement: DialogPlacement::Center, margin: 0 };
        let spec = DialogSpec{ title:"T", placement: Placement::Centered{w:10,h:5}, buttons:&[], show_close:false, default: None, focus: None, field: None };
        let r = draw_dialog(&mut buf, Rect::new(0,0,40,12), &spec, &st);
        // a cell just below-right of the frame carries the shadow bg
        let sx = r.area.right(); let sy = r.area.bottom();
        if sx < 40 && sy < 12 { assert_eq!(buf.cell((sx, sy)).unwrap().style().bg, Some(Color::DarkGray)); }
    }

    #[test]
    fn dialog_field_renders_label_value_and_caret_and_records_rect() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Style, Modifier}};
        let full = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(full);
        let st = DialogStyle {
            theme: &TEST_THEME,
            frame: Style::default(), border: Style::default(), box_style: BorderStyle::Single, glyphs: PaneGlyphs::default(),
            title: Style::default(), button: Style::default(), button_active: Style::default(),
            shadow: Style::default(), shadow_on: false, placement: DialogPlacement::Center, margin: 0,
        };
        let field = DialogField {
            label: "Name: ", value: "ab", cursor: 2, show_caret: true, dim: false,
            text_style: Style::default(),
            dim_style: Style::default().add_modifier(Modifier::DIM),
            caret_style: Style::default().add_modifier(Modifier::REVERSED),
        };
        let spec = DialogSpec {
            title: "Save", placement: Placement::Centered { w: 30, h: 6 },
            buttons: &[DialogButton { id: ButtonId::Save, label: "Save" }],
            show_close: true, default: Some(ButtonId::Save), focus: None,
            field: Some(field),
        };
        let rects = draw_dialog(&mut buf, full, &spec, &st);
        let fr = rects.field.expect("field rect recorded");
        // The label + value appear on the field row.
        let row: String = (fr.x..fr.right())
            .filter_map(|x| buf.cell((x, fr.y)).map(|c| c.symbol().chars().next().unwrap_or(' ')))
            .collect();
        assert!(row.contains("Name: ab"), "label+value drawn, got {row:?}");
        // Caret at end-of-text: the cell just past "ab" carries the caret (REVERSED).
        let caret_x = fr.x + "Name: ab".chars().count() as u16;
        assert!(buf.cell((caret_x, fr.y)).unwrap().style().add_modifier.contains(Modifier::REVERSED),
            "end caret cell is reversed");
    }

    /// Render a field with `value`/`cursor` in a dialog narrow enough to overflow,
    /// and return `(row_text, caret_column_within_row)`.
    fn field_row(value: &str, cursor: usize, dialog_w: u16) -> (String, Option<usize>) {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Style, Modifier}};
        let full = Rect::new(0, 0, 60, 8);
        let mut buf = Buffer::empty(full);
        let st = DialogStyle {
            theme: &TEST_THEME,
            frame: Style::default(), border: Style::default(), box_style: BorderStyle::Single, glyphs: PaneGlyphs::default(),
            title: Style::default(), button: Style::default(), button_active: Style::default(),
            shadow: Style::default(), shadow_on: false, placement: DialogPlacement::Center, margin: 0,
        };
        let field = DialogField {
            label: "N: ", value, cursor, show_caret: true, dim: false,
            text_style: Style::default(),
            dim_style: Style::default().add_modifier(Modifier::DIM),
            caret_style: Style::default().add_modifier(Modifier::REVERSED),
        };
        let spec = DialogSpec {
            title: "T", placement: Placement::Centered { w: dialog_w, h: 6 },
            buttons: &[DialogButton { id: ButtonId::Save, label: "Ok" }],
            show_close: true, default: Some(ButtonId::Save), focus: None,
            field: Some(field),
        };
        let rects = draw_dialog(&mut buf, full, &spec, &st);
        let fr = rects.field.expect("field rect recorded");
        let cells: Vec<_> = (fr.x..fr.right()).filter_map(|x| buf.cell((x, fr.y))).collect();
        let row: String = cells.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        let caret = cells.iter().position(|c| c.style().add_modifier.contains(Modifier::REVERSED));
        (row, caret)
    }

    #[test]
    fn a_field_wider_than_the_dialog_scrolls_to_keep_the_caret_visible() {
        // SQ-0554: the renderer used to draw from char 0 and stop at the content
        // edge, so a long value lost both its tail AND its caret — you could not
        // see what you were editing or where the cursor was. Prefilling
        // edit-notes (SQ-0524) made this easy to hit.
        let long = "abcdefghijklmnopqrstuvwxyz";

        // Caret at end-of-text: the window must show the TAIL, not the head.
        let (row, caret) = field_row(long, long.chars().count(), 20);
        assert!(caret.is_some(), "the caret must be drawn somewhere in the row: {row:?}");
        assert!(row.contains("xyz"), "the end of the value must be visible, got {row:?}");
        assert!(!row.contains("abc"), "the head must have scrolled off, got {row:?}");

        // Caret at the very start: the window shows the HEAD and the caret is on 'a'.
        let (row, caret) = field_row(long, 0, 20);
        assert!(row.contains("abc"), "with the caret at 0 the head is visible, got {row:?}");
        let caret = caret.expect("caret drawn");
        assert_eq!(row.chars().nth(caret), Some('a'), "caret sits on the first char: {row:?}");

        // A mid-string caret is visible, and sits on the char it indexes.
        let (row, caret) = field_row(long, 12, 20);
        let caret = caret.expect("caret drawn");
        assert_eq!(row.chars().nth(caret), Some('m'), "caret sits on chars[12] = 'm': {row:?}");
    }

    #[test]
    fn a_field_that_fits_does_not_scroll() {
        // Guards against the window sliding when there is nothing to scroll: the
        // caret-anchored offset must be 0 while the caret still fits.
        let (row, caret) = field_row("ab", 2, 30);
        assert!(row.starts_with("N: ab"), "short value stays left-aligned, got {row:?}");
        let caret = caret.expect("caret drawn");
        assert_eq!(caret, "N: ab".chars().count(), "end caret directly follows the value");
    }

    #[test]
    fn dialog_without_field_records_none() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let full = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(full);
        let st = DialogStyle {
            theme: &TEST_THEME,
            frame: Style::default(), border: Style::default(), box_style: BorderStyle::Single, glyphs: PaneGlyphs::default(),
            title: Style::default(), button: Style::default(), button_active: Style::default(),
            shadow: Style::default(), shadow_on: false, placement: DialogPlacement::Center, margin: 0,
        };
        let spec = DialogSpec {
            title: "T", placement: Placement::Centered { w: 20, h: 6 },
            buttons: &[], show_close: false, default: None, focus: None, field: None,
        };
        let rects = draw_dialog(&mut buf, full, &spec, &st);
        assert!(rects.field.is_none(), "no field => rects.field is None");
    }

    #[test]
    fn dialog_underlines_default_and_highlights_focus() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Style, Modifier}};
        let full = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(full);
        let st = DialogStyle {
            theme: &TEST_THEME,
            frame: Style::default(),
            border: Style::default(), box_style: BorderStyle::Single,
            glyphs: PaneGlyphs::default(),
            title: Style::default(),
            button: Style::default(),
            button_active: Style::default().add_modifier(Modifier::REVERSED),
            shadow: Style::default(),
            shadow_on: false,
            placement: DialogPlacement::Center,
            margin: 0,
        };
        let spec = DialogSpec {
            title: "T",
            placement: Placement::Centered { w: 30, h: 6 },
            buttons: &[
                DialogButton { id: ButtonId::Save, label: "Save" },
                DialogButton { id: ButtonId::Cancel, label: "Cancel" },
            ],
            show_close: true,
            default: Some(ButtonId::Save),
            focus: Some(1),
            field: None,
        };
        let rects = draw_dialog(&mut buf, full, &spec, &st);
        // The Save (default) button label cells carry UNDERLINED.
        let (_, save_rect) = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Save).unwrap();
        let save_cell = buf.cell((save_rect.x + 2, save_rect.y)).unwrap(); // inside the label
        assert!(save_cell.style().add_modifier.contains(Modifier::UNDERLINED),
            "default button must be underlined");
        // The Cancel (focused idx 1) button cells carry REVERSED (button_active).
        let (_, cancel_rect) = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Cancel).unwrap();
        let cancel_cell = buf.cell((cancel_rect.x + 2, cancel_rect.y)).unwrap();
        assert!(cancel_cell.style().add_modifier.contains(Modifier::REVERSED),
            "focused button must use button_active");
    }

    #[test]
    fn dialog_buttons_have_no_brackets_and_a_plain_separator() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Style, Modifier}};
        let full = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(full);
        let st = DialogStyle {
            theme: &TEST_THEME,
            frame: Style::default(),
            border: Style::default(), box_style: BorderStyle::Single,
            glyphs: PaneGlyphs::default(),
            title: Style::default(),
            button: Style::default().add_modifier(Modifier::REVERSED),
            button_active: Style::default().add_modifier(Modifier::REVERSED),
            shadow: Style::default(),
            shadow_on: false,
            placement: DialogPlacement::Center,
            margin: 0,
        };
        let spec = DialogSpec {
            title: "T",
            placement: Placement::Centered { w: 30, h: 6 },
            buttons: &[
                DialogButton { id: ButtonId::Save, label: "Save" },
                DialogButton { id: ButtonId::Cancel, label: "Cancel" },
            ],
            show_close: false, default: None, focus: None, field: None,
        };
        let rects = draw_dialog(&mut buf, full, &spec, &st);
        let (_, save) = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Save).unwrap();
        let (_, cancel) = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Cancel).unwrap();
        // The button rect is the label plus one reversed pad space each side — no "[ ]" chrome.
        assert_eq!(save.width, 6, "Save chip is ' Save '");
        assert_eq!(cancel.width, 8, "Cancel chip is ' Cancel '");
        // The whole button row carries no brackets.
        let row: String = (0..full.width)
            .map(|x| buf.cell((x, save.y)).unwrap().symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(!row.contains('['), "no '[' on the button row: {row:?}");
        assert!(!row.contains(']'), "no ']' on the button row: {row:?}");
        // Save sits to the left of Cancel with a one-column gap between them, and
        // that separator cell is NOT reversed (the button chips are).
        let sep_x = save.x + save.width;
        assert_eq!(sep_x, cancel.x - 1, "one-column separator between buttons");
        assert!(buf.cell((save.x, save.y)).unwrap().style().add_modifier.contains(Modifier::REVERSED),
            "chip (including its pad space) is reversed");
        assert!(!buf.cell((sep_x, save.y)).unwrap().style().add_modifier.contains(Modifier::REVERSED),
            "separator space is not reversed");
    }

    #[test]
    fn dialog_default_theme_uses_single_title_cap() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let full = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(full);
        let st = DialogStyle {
            theme: &TEST_THEME,
            frame: Style::default(), border: Style::default(), box_style: BorderStyle::Single,
            glyphs: PaneGlyphs::default(), title: Style::default(), button: Style::default(),
            button_active: Style::default(), shadow: Style::default(), shadow_on: false,
            placement: DialogPlacement::Center, margin: 0,
        };
        let spec = DialogSpec {
            title: "Settings", placement: Placement::Centered { w: 24, h: 6 },
            buttons: &[], show_close: false, default: None, focus: None, field: None,
        };
        let r = draw_dialog(&mut buf, full, &spec, &st);
        // The title-strip caps now track the border style: a single border yields
        // the single terminator ┤ (not the old always-thick ┫).
        let top: String = (r.area.x..r.area.right())
            .map(|x| buf.cell((x, r.area.y)).unwrap().symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(top.contains('┤'), "single border → left cap ┤, got {top:?}");
        assert!(!top.contains('┫'), "no thick cap on a single-border dialog: {top:?}");
    }

    #[test]
    fn dialog_double_border_frame_and_cap() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        use crate::theme::resolve::resolve_theme;
        let scheme = crate::colors::GhosttyScheme::default();
        let parsed = crate::theme::toml_schema::parse("[dialog]\nborder = { style = \"double\" }\n").unwrap();
        let theme = resolve_theme(&scheme, &parsed);
        let full = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(full);
        let st = DialogStyle {
            theme: &theme,
            frame: Style::default(), border: Style::default(), box_style: BorderStyle::Double,
            glyphs: PaneGlyphs::default(), title: Style::default(), button: Style::default(),
            button_active: Style::default(), shadow: Style::default(), shadow_on: false,
            placement: DialogPlacement::Center, margin: 0,
        };
        let spec = DialogSpec {
            title: "Save", placement: Placement::Centered { w: 24, h: 6 },
            buttons: &[], show_close: false, default: None, focus: None, field: None,
        };
        let r = draw_dialog(&mut buf, full, &spec, &st);
        // Double border → double corner and double terminator cap on the title.
        assert_eq!(buf.cell((r.area.x, r.area.y)).unwrap().symbol(), "╔");
        let top: String = (r.area.x..r.area.right())
            .map(|x| buf.cell((x, r.area.y)).unwrap().symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(top.contains('╡'), "double border → left cap ╡, got {top:?}");
    }
}
