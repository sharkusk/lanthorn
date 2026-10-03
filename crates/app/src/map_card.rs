//! The Map tab's room card (SQ-1688): a compact summary of the SELECTED room
//! drawn under the map, matching the web Journal.
//!
//! It appears exactly when a room is pinned (`AppState::selected_room`) on the
//! Map tab and is hidden otherwise — it never falls back to the room the player
//! stands in. The card is carved out of the Journal body BEFORE the map is laid
//! out (`layout::compute_pane_layout_with_card`), so the map rect shrinks by the
//! card's height and map hit-testing stays exact; the card's own rect is
//! outside it, which is also why a click on the card cannot reach the map.
//!
//! Top to bottom: a rule, the room name with a muted `on <layer>`, the captured
//! description (wrapped, truncated with `…`), `Seen at move N` when a turn is
//! known, and a row of buttons. The buttons run exactly what the room's
//! right-click menu items run ([`crate::room_menu::ROOM_MENU`]); Details
//! switches to the Room tab, as a double-click does.
//!
//! Selectors: `journal.map.card` (ground), `.border`, `.name`, `.layer`,
//! `.description`, `.seen`, `.button`.

use mapper::graph::{MapGraph, RoomId};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::journal::JournalTab;
use crate::state::AppState;
use crate::textwidth::{clip_to_cols_ellipsis, str_cells};

/// Fewest rows the map keeps above the card; below that the card is not drawn.
const MIN_MAP_ROWS: u16 = 4;
/// Description text considered for wrapping — more than any card can show.
const DESC_CHARS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapCardButton {
    Rename,
    Notes,
    Move,
    Details,
}

impl MapCardButton {
    pub const ALL: [MapCardButton; 4] =
        [MapCardButton::Rename, MapCardButton::Notes, MapCardButton::Move, MapCardButton::Details];

    pub fn label(self) -> &'static str {
        match self {
            MapCardButton::Rename => "[ Rename… ]",
            MapCardButton::Notes => "[ Notes… ]",
            MapCardButton::Move => "[ Move to another layer… ]",
            MapCardButton::Details => "[ Details ]",
        }
    }

    /// The `Context::Map` command this button runs: the SAME string the
    /// right-click menu item dispatches. `None` for Details, which is not a
    /// command but a tab switch ([`MapCardButton::action`]).
    pub fn command(self) -> Option<&'static str> {
        match self {
            MapCardButton::Rename => Some("rename-room"),
            MapCardButton::Notes => Some("edit-notes"),
            MapCardButton::Move => Some("move-region"),
            MapCardButton::Details => None,
        }
    }

    /// The direct action for the button that is not a command.
    pub fn action(self) -> Option<crate::input::Action> {
        match self {
            MapCardButton::Details => Some(crate::input::Action::SetJournalTab(JournalTab::Room)),
            _ => None,
        }
    }
}

/// What a click on the card means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardClick {
    Button(MapCardButton),
    /// On the card but not on a button: swallowed.
    Body,
}

/// What the card shows for one room.
struct Content {
    name: String,
    layer: String,
    description: Option<String>,
    turn: Option<u32>,
}

fn content(graph: &MapGraph, id: RoomId) -> Option<Content> {
    let room = graph.room(id)?;
    Some(Content {
        name: room.label().to_string(),
        layer: graph.layer_name(graph.layer_of(id)).to_string(),
        description: room.description.clone().filter(|d| !d.trim().is_empty()),
        turn: room.description_turn,
    })
}

/// The card's rows, resolved for a width.
struct Plan {
    desc: Vec<String>,
    seen: bool,
    /// Button rows: each button with its column offset from the card's left edge.
    buttons: Vec<Vec<(MapCardButton, u16)>>,
}

impl Plan {
    fn rows(&self) -> u16 {
        // rule + name + description + seen + buttons
        (2 + self.desc.len() + self.seen as usize + self.buttons.len()) as u16
    }
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let text: String = text.chars().take(DESC_CHARS).collect();
    for word in text.split_whitespace() {
        let word = if str_cells(word) > width {
            crate::textwidth::truncate_to_cols(word, width)
        } else {
            word
        };
        let need = if cur.is_empty() { 0 } else { str_cells(&cur) + 1 } + str_cells(word);
        if !cur.is_empty() && need > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Buttons one cell apart, wrapping to a new row when the next does not fit.
fn button_rows(width: u16) -> Vec<Vec<(MapCardButton, u16)>> {
    let mut rows: Vec<Vec<(MapCardButton, u16)>> = vec![Vec::new()];
    let mut x: u16 = 1;
    for b in MapCardButton::ALL {
        let w = str_cells(b.label()) as u16;
        if x > 1 && x + w > width {
            rows.push(Vec::new());
            x = 1;
        }
        rows.last_mut().unwrap().push((b, x));
        x += w + 1;
    }
    rows
}

/// Resolve the card for `width` columns inside at most `cap` rows (never fewer
/// than its minimum: rule, name and the button rows).
fn plan(c: &Content, width: u16, cap: u16) -> Plan {
    let buttons = button_rows(width);
    let seen = c.turn.is_some();
    let fixed = 2 + seen as u16 + buttons.len() as u16;
    let room = cap.saturating_sub(fixed) as usize;
    let text_w = (width as usize).saturating_sub(2).max(1);
    let mut desc = match &c.description {
        Some(d) if room > 0 => wrap(d, text_w),
        _ => Vec::new(),
    };
    if desc.len() > room {
        desc.truncate(room);
        if let Some(last) = desc.last_mut() {
            *last = clip_to_cols_ellipsis(&format!("{last}…"), text_w);
        }
    }
    Plan { desc, seen, buttons }
}

/// The room the card describes: only while the Map tab is up and a room is
/// pinned.
fn wanted(state: &AppState) -> Option<RoomId> {
    if state.debug.is_some() || state.journal_tab != JournalTab::Map {
        return None;
    }
    state.selected_room
}

/// How many rows the card takes from `body` (the Journal body); 0 when it is
/// hidden. Capped at about a third of the body, but never below the minimum,
/// and hidden when the map would be left too short.
pub fn rows_for(graph: &MapGraph, state: &AppState, body: Rect) -> u16 {
    let Some(id) = wanted(state) else { return 0 };
    let Some(c) = content(graph, id) else { return 0 };
    if body.width < 8 {
        return 0;
    }
    let rows = plan(&c, body.width, body.height / 3).rows();
    if body.height < rows + MIN_MAP_ROWS {
        return 0;
    }
    rows
}

fn fill(buf: &mut Buffer, area: Rect, style: Style) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
    }
}

fn put(buf: &mut Buffer, area: Rect, y: u16, x: u16, s: &str, style: Style) -> u16 {
    crate::render::draw_str_clipped(buf, x, y, s, style, area);
    (str_cells(s) as u16).min(area.right().saturating_sub(x))
}

/// Draw the card into `rect` (the rect [`rows_for`] sized) and return each
/// button's hit-rect.
pub fn draw(
    graph: &MapGraph,
    state: &AppState,
    rect: Rect,
    buf: &mut Buffer,
) -> Vec<(MapCardButton, Rect)> {
    let mut hits = Vec::new();
    let Some(id) = wanted(state) else { return hits };
    let Some(c) = content(graph, id) else { return hits };
    if rect.width == 0 || rect.height == 0 {
        return hits;
    }
    let t = |n: &str| state.colors.theme.get(n).style;
    let ground = t("journal.map.card");
    let on = |n: &str| ground.patch(t(n));
    fill(buf, rect, ground);
    let p = plan(&c, rect.width, rect.height);
    let mut y = rect.y;
    let border = on("journal.map.card.border");
    for x in rect.x..rect.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_symbol("─").set_style(border);
        }
    }
    y += 1;
    if y < rect.bottom() {
        let name_w = put(buf, rect, y, rect.x + 1, &c.name, on("journal.map.card.name"));
        if !c.layer.is_empty() {
            put(buf, rect, y, rect.x + 1 + name_w + 1, &format!("on {}", c.layer), on("journal.map.card.layer"));
        }
        y += 1;
    }
    for line in &p.desc {
        if y >= rect.bottom() {
            break;
        }
        put(buf, rect, y, rect.x + 1, line, on("journal.map.card.description"));
        y += 1;
    }
    if let (true, Some(turn)) = (p.seen, c.turn) {
        if y < rect.bottom() {
            put(buf, rect, y, rect.x + 1, &format!("Seen at move {turn}"), on("journal.map.card.seen"));
            y += 1;
        }
    }
    let button = on("journal.map.card.button");
    for row in &p.buttons {
        if y >= rect.bottom() {
            break;
        }
        for (b, off) in row {
            let w = put(buf, rect, y, rect.x + off, b.label(), button);
            if w > 0 {
                hits.push((*b, Rect::new(rect.x + off, y, w, 1)));
            }
        }
        y += 1;
    }
    hits
}

/// Resolve a click at `(col, row)`: a button, the card's body, or `None` when
/// the point is outside the card.
pub fn click_at(card: Rect, buttons: &[(MapCardButton, Rect)], col: u16, row: u16) -> Option<CardClick> {
    let pt = ratatui::layout::Position { x: col, y: row };
    if !card.contains(pt) {
        return None;
    }
    Some(match buttons.iter().find(|(_, r)| r.contains(pt)) {
        Some((b, _)) => CardClick::Button(*b),
        None => CardClick::Body,
    })
}
