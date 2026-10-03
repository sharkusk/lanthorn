//! The Journal (SQ-1684): the tabbed panel on the story pane's right, the
//! TUI's counterpart of the web UI's Journal. One tab shows at a time; a row of
//! tab labels along its top picks which.
//!
//! Adding a tab is one [`JournalTab`] variant (its name, label and short label
//! go in the `match`es below) plus one arm in the renderer that draws the
//! Journal body (`main.rs`'s draw closure) — nothing here is keyed on a count.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::colors::ColorScheme;

/// Which tab the Journal is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JournalTab {
    /// The automap (drawn or matrix view).
    #[default]
    Map,
    /// The room panel: notes, exit card and objects for the room in focus.
    Room,
    /// The inventory panel: what is carried and where everything else was seen.
    Inventory,
    /// The story's hint file (InvisiClues), run as a Z-machine companion in the
    /// tab's own window (SQ-1685). Between Inventory and Documents.
    Hints,
    /// The current game's documents folder: manuals, maps and feelies, read in
    /// place (SQ-1681).
    Documents,
}

impl JournalTab {
    /// Every tab, in display order.
    pub const ALL: [JournalTab; 5] = [
        JournalTab::Map,
        JournalTab::Room,
        JournalTab::Inventory,
        JournalTab::Hints,
        JournalTab::Documents,
    ];

    /// The command argument and sidecar spelling.
    pub fn name(self) -> &'static str {
        match self {
            JournalTab::Map => "map",
            JournalTab::Room => "room",
            JournalTab::Inventory => "inventory",
            JournalTab::Hints => "hints",
            JournalTab::Documents => "documents",
        }
    }

    /// The label drawn on the tab bar.
    pub fn label(self) -> &'static str {
        match self {
            JournalTab::Map => "Map",
            JournalTab::Room => "Room",
            JournalTab::Inventory => "Inventory",
            JournalTab::Hints => "Hints",
            JournalTab::Documents => "Documents",
        }
    }

    /// The abbreviation used when the full labels do not fit.
    pub fn short_label(self) -> &'static str {
        match self {
            JournalTab::Map => "Map",
            JournalTab::Room => "Rm",
            JournalTab::Inventory => "Inv",
            JournalTab::Hints => "Hint",
            JournalTab::Documents => "Docs",
        }
    }

    /// Parse [`JournalTab::name`]'s spelling (case-insensitive; `inv` and
    /// `items` are accepted for `inventory`).
    pub fn from_name(s: &str) -> Option<JournalTab> {
        match s.trim().to_ascii_lowercase().as_str() {
            "map" => Some(JournalTab::Map),
            "room" => Some(JournalTab::Room),
            "inventory" | "inv" | "items" => Some(JournalTab::Inventory),
            "hints" | "hint" => Some(JournalTab::Hints),
            "documents" | "docs" => Some(JournalTab::Documents),
            _ => None,
        }
    }

    /// 0-based position in [`JournalTab::ALL`].
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    /// The tab after this one, wrapping.
    pub fn next(self) -> JournalTab {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    /// The tab before this one, wrapping.
    pub fn prev(self) -> JournalTab {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// What a click on the tab bar means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabBarHit {
    /// A tab label.
    Tab(JournalTab),
    /// The ‹ marker of the narrow form: the previous tab.
    Prev,
    /// The › marker of the narrow form: the next tab.
    Next,
}

/// One drawn piece of the tab bar: its text and the 0-based column it starts at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabBarCell {
    pub hit: TabBarHit,
    pub col: u16,
    pub text: String,
}

fn padded(label: &str) -> String {
    format!(" {label} ")
}

/// Lay the tab bar out for `width` columns with `active` selected.
///
/// Three forms, widest that fits: full labels, abbreviated labels, and — when
/// even those overflow — only the active tab between ‹ › markers (clicking a
/// marker steps to the neighbouring tab). A bar narrower than the narrow form
/// is truncated by the drawing clip, not here.
pub fn tab_bar_cells(width: u16, active: JournalTab) -> Vec<TabBarCell> {
    let strip = |label: fn(JournalTab) -> &'static str| {
        let mut col = 0u16;
        let mut cells = Vec::new();
        for t in JournalTab::ALL {
            let text = padded(label(t));
            let w = text.chars().count() as u16;
            cells.push(TabBarCell { hit: TabBarHit::Tab(t), col, text });
            col += w;
        }
        (cells, col)
    };
    for label in [JournalTab::label as fn(JournalTab) -> &'static str, JournalTab::short_label] {
        let (cells, total) = strip(label);
        if total <= width {
            return cells;
        }
    }
    let mid = padded(active.label());
    let mid_w = mid.chars().count() as u16;
    vec![
        TabBarCell { hit: TabBarHit::Prev, col: 0, text: "\u{2039}".into() },
        TabBarCell { hit: TabBarHit::Tab(active), col: 1, text: mid },
        TabBarCell { hit: TabBarHit::Next, col: 1 + mid_w, text: "\u{203a}".into() },
    ]
}

/// The mark drawn before the Hints tab's label while the hint session has the
/// keyboard (SQ-1685).
pub const HINTS_FOCUS_MARK: char = '\u{25b8}';

/// Draw the tab bar into the one-row `area` and return each cell's hit rect.
/// Selectors: `journal.tabbar` (the row's fill), `journal.tab`,
/// `journal.tab:active`, and — on the Hints label while the hint session holds
/// the keyboard — `journal.hints.tab:focused`.
pub fn draw_tab_bar(
    area: Rect,
    active: JournalTab,
    hints_focused: bool,
    colors: &ColorScheme,
    buf: &mut Buffer,
) -> Vec<(TabBarHit, Rect)> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let bar: Style = colors.theme.get("journal.tabbar").style;
    let tab: Style = colors.theme.get("journal.tab").style;
    let on: Style = colors.theme.get("journal.tab:active").style;
    let focused: Style = colors.theme.get("journal.hints.tab:focused").style;
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.set_symbol(" ").set_style(bar);
        }
    }
    let mut hits = Vec::new();
    for c in tab_bar_cells(area.width, active) {
        let marked = hints_focused && c.hit == TabBarHit::Tab(JournalTab::Hints);
        let style = match c.hit {
            TabBarHit::Tab(t) if t == active => on,
            _ => tab,
        };
        let style = if marked { style.patch(focused) } else { style };
        let x = area.x + c.col;
        // The focus marker takes the label's leading pad cell, so nothing shifts.
        let text = if marked {
            let mut t = c.text.clone();
            t.replace_range(0..1, &HINTS_FOCUS_MARK.to_string());
            t
        } else {
            c.text.clone()
        };
        crate::render::draw_str_clipped(buf, x, area.y, &text, style, area);
        let w = (c.text.chars().count() as u16).min(area.right().saturating_sub(x));
        if w > 0 {
            hits.push((c.hit, Rect::new(x, area.y, w, 1)));
        }
    }
    hits
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_cycle() {
        for t in JournalTab::ALL {
            assert_eq!(JournalTab::from_name(t.name()), Some(t));
            assert_eq!(t.next().prev(), t);
        }
        assert_eq!(JournalTab::Documents.next(), JournalTab::Map);
        assert_eq!(JournalTab::Map.prev(), JournalTab::Documents);
        assert_eq!(JournalTab::ALL.last(), Some(&JournalTab::Documents), "Documents is the last tab");
        assert_eq!(
            JournalTab::ALL,
            [JournalTab::Map, JournalTab::Room, JournalTab::Inventory, JournalTab::Hints, JournalTab::Documents],
            "Hints sits between Inventory and Documents"
        );
        assert_eq!(JournalTab::Hints.index(), 3, "Alt+4 is the fourth tab");
        assert_eq!(JournalTab::Inventory.next(), JournalTab::Hints);
        assert_eq!(JournalTab::Documents.prev(), JournalTab::Hints);
        assert_eq!(JournalTab::from_name("docs"), Some(JournalTab::Documents));
        assert_eq!(JournalTab::from_name("INV"), Some(JournalTab::Inventory));
        assert_eq!(JournalTab::from_name("hints"), Some(JournalTab::Hints));
    }

    #[test]
    fn wide_bar_shows_full_labels_in_order() {
        let cells = tab_bar_cells(40, JournalTab::Map);
        let text: Vec<&str> = cells.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(text, [" Map ", " Room ", " Inventory ", " Hints ", " Documents "]);
        assert_eq!(cells[1].col, 5);
    }

    #[test]
    fn narrower_bar_abbreviates_then_collapses_to_the_active_tab() {
        let short = tab_bar_cells(26, JournalTab::Map);
        assert_eq!(short[2].text, " Inv ");
        assert_eq!(short[3].text, " Hint ", "Hints abbreviates too");
        assert_eq!(short[4].text, " Docs ");
        let narrow = tab_bar_cells(25, JournalTab::Room);
        let kinds: Vec<TabBarHit> = narrow.iter().map(|c| c.hit).collect();
        assert_eq!(
            kinds,
            [TabBarHit::Prev, TabBarHit::Tab(JournalTab::Room), TabBarHit::Next]
        );
        assert_eq!(narrow[1].text, " Room ");
    }
}

/// Frame a Journal tab body the way the Inventory tab frames its list: the shared
/// themed panel (`panel.border`, `:active` when `highlighted`), the tab's name as
/// the title strip, and a `panel.border` of `none` (frameless) dropping the
/// border exactly as it does for the other tabs. Returns the inner content rect.
pub fn frame_body(buf: &mut Buffer, area: Rect, title: &str, colors: &ColorScheme, highlighted: bool) -> Rect {
    use crate::render::paneframe::{InsetSegment, PaneGlyphs};
    use crate::render::panel::{draw_panel, PanelSpec, PanelStrip};
    let style = colors.theme.get("inventory_panel").style;
    let border_selector = if highlighted { "panel.border:active" } else { "panel.border" };
    let border_color = if highlighted { colors.theme.get("panel.border:active").style } else { style };
    let spec = PanelSpec {
        area,
        border_selector,
        border_color: Some(border_color),
        border_style: None,
        glyphs: &PaneGlyphs::default(),
        header_on: true,
        strip: Some(PanelStrip { segments: &[InsetSegment { text: title, active: false }], base: style, active: style }),
        body_fill: None,
    };
    draw_panel(buf, &spec, &colors.theme).content
}
