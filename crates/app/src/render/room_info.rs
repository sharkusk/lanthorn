//! Room-info body: the story-facing view of one room, drawn by the room dock.
//!
//! Shows the room's notes and its EXIT CARD — one line per direction, in the matrix view's
//! vocabulary, with destination names spelled out (SQ-0666). When the displayed room is the
//! player's current room, also lists the objects in that room queried live from the Z-machine
//! object tree. (The room's NAME and layer are the dock header's job — see
//! [`crate::render::room_dock`] — so the body does not repeat them.)
//!
//! The card is the per-room form of the matrix: same seven cells, same meanings, one room at a
//! time and no numbering to decode. It replaced a plain `dir -> name` list that could not say
//! whether a direction had been tried, and — with the matrix — the room inspector's compass rose
//! and the map's untried-exits overlay, which each said less.
//!
//! SQ-0692 retired the floating-dialog wrapper this used to live in: the body draws into a plain
//! `Rect` now, so the dock owns the chrome and there is exactly one panel describing a room.

use mapper::direction::Direction;
use mapper::graph::{MapGraph, RoomId};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::draw_str_clipped;
use crate::theme::resolve::Theme;

/// A destination's display name, agreeing with whatever the matrix table itself would print for
/// it (SQ-0685): the NUMBERED row label ("Maze 4") when the destination shares `labels`' layer —
/// every cell `card_detail` handles except `LeavesLayer` names one of those — falling back to the
/// room's bare name for a destination outside it, which has no row in `labels` to number. Numbers
/// are minted by discovery order, not room id, but that is `labels`' concern entirely; naming here
/// only has to keep asking the one function that knows, so the card and the matrix can never
/// disagree about what a room is called.
fn dest_name(graph: &MapGraph, labels: &mapper::matrix::MatrixLabels, layer: mapper::layer::LayerId, id: RoomId) -> String {
    if graph.layer_of(id) == layer {
        let row = labels.row_of(id);
        if !row.is_empty() {
            return row.to_string();
        }
    }
    graph.room(id).map(|r| r.label().to_owned()).unwrap_or_else(|| format!("#{id}"))
}

/// One card line for a direction: the glyph, and what it means spelled out.
///
/// Deliberately more verbose than the matrix cell it mirrors. The matrix is a table you scan
/// across twelve columns; the card is one room you are reading about, so there is room to say
/// "Maze 4" instead of "4" and "back: W" instead of "⇠w".
fn card_detail(
    graph: &MapGraph,
    labels: &mapper::matrix::MatrixLabels,
    layer: mapper::layer::LayerId,
    cell: mapper::matrix::MatrixCell,
) -> (&'static str, String) {
    use mapper::matrix::MatrixCell as C;
    let name = |id: RoomId| dest_name(graph, labels, layer, id);
    match cell {
        C::Reciprocal { dest } => ("⇄", name(dest)),
        C::ReturnBy { dest, back } => {
            ("→", format!("{}  back: {}", name(dest), dir_label(back)))
        }
        C::OneWay { dest } => ("⇢", name(dest)),
        C::SelfLoop => ("↩", "leads back here".to_string()),
        C::LeavesLayer { dest } => {
            // Cross-layer: `dest` has no row in THIS layer's `labels` to number it with, exactly
            // like the matrix's own `⇱out` footnote, which names the same way.
            let raw = graph.room(dest).map(|r| r.label().to_owned()).unwrap_or_else(|| format!("#{dest}"));
            ("⇱", format!("{} · {}", raw, graph.layer_name(graph.layer_of(dest))))
        }
        C::Probed => ("×", "tried, no way through".to_string()),
        C::Untried => ("·", String::new()),
        C::Random => ("?", "destination varies".to_string()),
    }
}

// Direction display labels (cardinal + diagonal + portal).
fn dir_label(dir: Direction) -> &'static str {
    match dir {
        Direction::N  => "N",
        Direction::NE => "NE",
        Direction::E  => "E",
        Direction::SE => "SE",
        Direction::S  => "S",
        Direction::SW => "SW",
        Direction::W  => "W",
        Direction::NW => "NW",
        Direction::Up => "Up",
        Direction::Down => "Dn",
        Direction::In => "In",
        Direction::Out => "Out",
        Direction::Unknown => "?",
    }
}

/// A room's display name as every surface that names it agrees to spell it: the matrix's NUMBERED
/// row label ("Maze 4") when its layer numbers it, the bare room name otherwise (SQ-0685).
///
/// **The dock header calls this and the exit card calls [`dest_name`], and this
/// now calls that** (SQ-1065). The sentence here used to say both surfaces called
/// this one, which was never true — `display_name` has a single caller
/// (`room_dock`) and the card has always named rooms through `dest_name`, a second
/// independent spelling of the same three-step rule. They agreed for every input,
/// which is the only reason nothing was ever misnamed.
///
/// The two differ only by `dest_name`'s `graph.layer_of(id) == layer` gate, and
/// here that gate is trivially true because `layer` IS `graph.layer_of(room_id)`.
/// So this is that function asked about a room on its own layer.
pub fn display_name(graph: &MapGraph, room_id: RoomId) -> String {
    let layer = graph.layer_of(room_id);
    dest_name(graph, &mapper::matrix::labels(graph, layer), layer, room_id)
}

// ── The exit card's column layout (SQ-0694) ──────────────────────────────────

/// Display cells between two card columns. Two, not one: the card's lines end in a room name, and
/// a single space would read as part of it.
const CARD_GAP: usize = 2;

/// The most columns the exit card will use, however wide the dock gets.
///
/// Not a width limit — a MEANING one. [`mapper::matrix::MATRIX_DIRS`] is three groups of four
/// (N/S/E/W, NE/NW/SE/SW, Up/Down/In/Out), so a three-column column-major grid puts each group in
/// its own column: the arrangement you would have drawn by hand. Letting the width alone decide
/// gives six columns of two on a wide pane, which is more compact and reads worse — the groups
/// shatter across columns and the eye has nothing to follow.
const MAX_CARD_COLS: usize = 3;

/// Where the exit card's columns sit: how many ROWS the grid is, and each column's `(x offset,
/// width)` in display cells from the card's left edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardLayout {
    pub rows: usize,
    pub cols: Vec<(usize, usize)>,
}

/// Lay `entry_widths` (in DISPLAY CELLS — measure with [`crate::textwidth::str_cells`], never a
/// char count: a full-width destination name occupies two columns per glyph) into the widest grid
/// that fits `width`, filled COLUMN-MAJOR.
///
/// The card is the Info body's height driver — twelve fixed lines — and a dock is far wider than
/// one of those lines. Spending the width instead of the height is what lets the whole card, the
/// objects and the header share a dock about a third the height the single column needed.
///
/// Column-major matters for more than tidiness: [`mapper::matrix::MATRIX_DIRS`] runs
/// N/S/E/W, NE/NW/SE/SW, Up/Down/In/Out, so a three-column grid puts the cardinals, the diagonals
/// and the portals each in their own column — the grouping you would have drawn by hand.
///
/// Fewest rows wins (the `ls` rule): try one row, then two, until the per-column widths plus the
/// gaps fit. Falling all the way through means even ONE column is wider than `area` — the caller
/// clips, and the rows that fit still read.
pub fn layout_card(entry_widths: &[usize], width: u16) -> CardLayout {
    let n = entry_widths.len();
    if n == 0 {
        return CardLayout { rows: 0, cols: Vec::new() };
    }
    let avail = width as usize;

    let col_widths = |rows: usize| -> Vec<usize> {
        (0..n.div_ceil(rows))
            .map(|c| {
                entry_widths[c * rows..((c + 1) * rows).min(n)]
                    .iter()
                    .copied()
                    .max()
                    .unwrap_or(0)
            })
            .collect()
    };

    // Start at the row count that yields at most MAX_CARD_COLS columns; fewer rows than that
    // would only buy sparser columns.
    let min_rows = n.div_ceil(MAX_CARD_COLS).max(1);
    for rows in min_rows..=n {
        let widths = col_widths(rows);
        let total: usize = widths.iter().sum::<usize>() + CARD_GAP * (widths.len() - 1);
        if total > avail {
            continue;
        }
        // The rightmost column has nothing to its right, so hand it every leftover cell: a long
        // destination name there is truncated only when the DOCK is too narrow, not because its
        // neighbours in the same column happened to be short.
        let mut cols = Vec::with_capacity(widths.len());
        let mut x = 0usize;
        for (i, w) in widths.iter().enumerate() {
            let w = if i + 1 == widths.len() { avail.saturating_sub(x).max(*w) } else { *w };
            cols.push((x, w));
            x += w + CARD_GAP;
        }
        return CardLayout { rows, cols };
    }

    // Not even one column fits: one column, clipped to the area.
    CardLayout { rows: n, cols: vec![(0, avail)] }
}

/// Draw the room-info body into `area` — no chrome, no borders: the caller (the room dock) owns
/// those.
///
/// - `graph`: the mapper graph for notes/exits.
/// - `room_objects`: the objects located in this room, already queried from the
///   engine's introspection (empty when introspection is unavailable, e.g. the
///   map is in tidy-anim mode). Shown only when this is the current room.
/// - `room_id`: the room to display.
/// - `current_room`: the player's actual current room (used to gate object listing).
/// - `theme`: for the shared `map.matrix.cell:frontier` dimming, so the card and the matrix agree.
/// - `body` / `heading`: the styles for ordinary lines and for section labels.
#[allow(clippy::too_many_arguments)]
pub fn draw_room_info_body(
    graph: &MapGraph,
    room_objects: &[String],
    room_id: RoomId,
    current_room: Option<RoomId>,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    body: Style,
    heading: Style,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let Some(room) = graph.room(room_id) else { return };
    // Computed once and threaded through every name in this body, so the card can never disagree
    // with the matrix table or its `⇲`/`⇱out` footnotes about what a room is numbered (SQ-0685):
    // both ultimately read the same `labels`.
    let layer = graph.layer_of(room_id);
    let labels = mapper::matrix::labels(graph, layer);

    // The exit card: every one of the twelve travel directions, classified exactly as the matrix
    // view classifies it. All twelve, including the untried ones — "where haven't I been?" is the
    // question this panel inherited when the untried-exits overlay was retired, and a direction
    // left off the card is a direction the player stops considering.
    let card: Vec<(Direction, &'static str, String)> = mapper::matrix::MATRIX_DIRS
        .iter()
        .map(|&d| {
            let (glyph, detail) =
                card_detail(graph, &labels, layer, mapper::matrix::classify(graph, room_id, d));
            (d, glyph, detail)
        })
        .collect();
    // Non-compass passages (xyzzy, pray) have no column in the twelve and would otherwise vanish
    // from the card entirely.
    let odd: Vec<String> =
        graph
            .connections()
            .iter()
            .filter(|c| c.origin == room_id && c.dir == Direction::Unknown)
            .map(|c| dest_name(graph, &labels, layer, c.dest))
            .collect();

    // Show objects only when this is the current room.
    let objects: Vec<String> = if current_room == Some(room_id) {
        room_objects.to_vec()
    } else {
        Vec::new()
    };

    let value_style = body;
    let section_style = heading;

    let inner_x = area.x;
    let inner_w = area.width;
    let clip = area;
    let mut row = area.y;
    let max_y = area.bottom().saturating_sub(1);

    // Notes (if any), word-wrapped char/width-aware (SQ-0638): a raw byte-offset
    // slice panics on a multibyte note (e.g. one full of '€') since a slice
    // boundary can land mid-character.
    if !room.notes.is_empty() && row <= max_y {
        for line in crate::render::transcript::wrap_line(&room.notes, inner_w) {
            if row > max_y { break; }
            draw_str_clipped(buf, inner_x, row, &line, value_style, clip);
            row += 1;
        }
    }

    // "Also seen as: ..." (SQ-1257 Phase 3) — the other names the story has printed for this
    // room, e.g. Lost Pig's gnome tunnels rerolling a fresh name on every step. Under the notes,
    // above the exit card, shown only when the room actually has any.
    if !room.aliases.is_empty() && row <= max_y {
        let aliases_style = theme.get("room_panel.aliases").style;
        let line = format!("Also seen as: {}", room.aliases.join(", "));
        for wrapped in crate::render::transcript::wrap_line(&line, inner_w) {
            if row > max_y { break; }
            draw_str_clipped(buf, inner_x, row, &wrapped, aliases_style, clip);
            row += 1;
        }
    }

    // Objects (only for the current room) come BEFORE the card (SQ-0692). The card is a fixed
    // thirteen-line block, so in a dock shortened past its natural height it is the section that
    // runs off the bottom — and it degrades gracefully, because every one of its rows is the same
    // shape and the ones that fit are still readable. A short "Here:" list buried underneath it
    // was simply invisible at any dock height a normal terminal can spare.
    if !objects.is_empty() && row <= max_y {
        draw_str_clipped(buf, inner_x, row, "Here:", section_style, clip);
        row += 1;
        for name in &objects {
            if row > max_y { break; }
            let line = format!("  {}", name);
            draw_str_clipped(buf, inner_x, row, &line, value_style, clip);
            row += 1;
        }
    }

    // Exits — the card, laid out in as many COLUMNS as the dock is wide enough for (SQ-0694).
    // Untried and dead-end directions are dimmed with the same selector the matrix dims its
    // frontier cells with, so the two surfaces read alike.
    let frontier_style = theme.get("map.matrix.cell:frontier").style;
    // The `?` random-exit glyph (SQ-1257) gets the matrix's own `map.matrix.cell:random` selector
    // — not `frontier`, since a random exit is explored, not unexplored ground.
    let random_style = theme.get("map.matrix.cell:random").style;
    if row <= max_y {
        draw_str_clipped(buf, inner_x, row, "Exits:", section_style, clip);
        row += 1;
    }

    // One entry per line of the card: the twelve travel directions, then the non-compass
    // passages, which are card lines of the same shape and belong in the same grid.
    let entries: Vec<(String, Style)> = card
        .iter()
        .map(|(dir, glyph, detail)| {
            let line = format!("  {:<3} {} {}", dir_label(*dir), glyph, detail);
            let style = if *glyph == "?" {
                random_style
            } else if detail.is_empty() || *glyph == "×" {
                frontier_style
            } else {
                value_style
            };
            (line.trim_end().to_string(), style)
        })
        .chain(odd.iter().map(|dest| (format!("  ?   ⇢ {dest}"), value_style)))
        .collect();

    let widths: Vec<usize> =
        entries.iter().map(|(t, _)| crate::textwidth::str_cells(t)).collect();
    let plan = layout_card(&widths, inner_w);
    let card_top = row;
    for (i, (text, style)) in entries.iter().enumerate() {
        let (c, r) = (i / plan.rows.max(1), i % plan.rows.max(1));
        let y = card_top + r as u16;
        // A row past the bottom is simply not drawn: the grid degrades the way the single column
        // did, and every row that fits still reads in full.
        if y > max_y {
            continue;
        }
        let Some(&(dx, w)) = plan.cols.get(c) else { continue };
        draw_str_clipped(
            buf,
            inner_x + dx as u16,
            y,
            crate::textwidth::truncate_to_cols(text, w),
            *style,
            clip,
        );
    }
}

/// List the display names of everything the player can see in room `room_id`.
///
/// Not simply the room object's direct children (SQ-0678). A Z-machine room
/// holds three kinds of visible thing, and only the first is a child of it:
///
/// - things on the floor — direct children;
/// - things on a supporter or inside an open container standing in the room —
///   children of *that furniture*, so the sack and bottle on Zork I's kitchen
///   table are two levels down, not one;
/// - shared scenery named by the room but parked in a bucket object — the
///   window at Behind House is never a child of any room.
///
/// `model` supplies the story-specific conventions needed to find the last two
/// safely; see [`zvm::world`] for how they are inferred and for the guarantee
/// that a closed container's contents never appear here.
pub(crate) fn list_room_objects(
    model: &zvm::world::WorldModel,
    names: Option<&zvm::objects::ParseNames>,
    mem: &zvm::memory::Memory,
    room_id: RoomId,
) -> Vec<grammar_model::ObjectWords> {
    list_room_objects_excluding(model, names, mem, room_id, 0)
}

/// Same traversal as [`list_room_objects`], but skipping the object whose
/// id is `exclude` — and its whole subtree (0 excludes nothing: 0 is never a
/// valid object id). Used to keep the player object out of the command band's
/// "here" column (SQ-0667): filtering by id here, during the same walk that
/// builds the names, is what makes the exclusion exact rather than a fragile
/// name-match against whatever the player object happens to be called. Skipping
/// the subtree matters more now that the walk nests — the player is a holder
/// too, and their pockets are the *carried* column, never *here*.
pub(crate) fn list_room_objects_excluding(
    model: &zvm::world::WorldModel,
    names: Option<&zvm::objects::ParseNames>,
    mem: &zvm::memory::Memory,
    room_id: RoomId,
    exclude: u16,
) -> Vec<grammar_model::ObjectWords> {
    // Name-only rooms have no backing object; never read the object table by a
    // synthetic id (it would be outside the table).
    if crate::roomid::is_synthetic_room(room_id) {
        return Vec::new();
    }
    model
        .visible_room_objects(mem, room_id, exclude)
        .into_iter()
        .map(|o| crate::inventory::object_words(mem, names, o))
        // An object the story holds neither a printed name nor a parse name for
        // is not something a panel can show or a player can type (SQ-1042); the
        // filter was on the printed name alone, which drops every Inform 7
        // object, whose words are the only text naming it.
        .filter(|o| o.display_name().is_some())
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use mapper::graph::MapGraph;
    use ratatui::style::{Color, Style};

    #[test]
    fn list_room_objects_empty_for_synthetic_id() {
        // A synthetic RoomId (high bit set) must not read the object table.
        // Build a minimal v5 story in the same style as headless.rs's minimal_machine.
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 5; // version = 5
        buf[0x04] = 0x00; buf[0x05] = 0x40; // high_mem_base = 0x0040
        buf[0x06] = 0x00; buf[0x07] = 0x40; // initial_pc = 0x0040
        buf[0x08] = 0x00; buf[0x09] = 0x80; // dict = 0x0080
        buf[0x0A] = 0x01; buf[0x0B] = 0x00; // object table = 0x0100
        buf[0x0C] = 0x03; buf[0x0D] = 0x00; // global vars = 0x0300
        buf[0x0E] = 0x04; buf[0x0F] = 0x00; // static_mem = 0x0400
        buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev = 0x0060
        buf[0x0040] = 0xba; // quit opcode
        let mem = zvm::memory::Memory::new(buf).unwrap();
        let synth = crate::roomid::SYNTHETIC_ROOM_FLAG | 0x0123;
        let model = zvm::world::WorldModel::discover(&mem);
        assert!(list_room_objects(&model, None, &mem, synth).is_empty());
    }


    fn test_theme() -> crate::theme::resolve::Theme {
        crate::colors::ColorScheme::terminal_default().theme
    }

    /// Draw the body into a plain rect the way the room dock does, and return the
    /// whole buffer as text.
    fn render_body(
        g: &MapGraph,
        objects: &[String],
        room: RoomId,
        current: Option<RoomId>,
        w: u16,
        h: u16,
    ) -> String {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let theme = test_theme();
        draw_room_info_body(
            g, objects, room, current, area, &mut buf, &theme,
            Style::default(), Style::default().fg(Color::Cyan),
        );
        (0..h)
            .map(|y| {
                (0..w).map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" ")).collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn make_graph_with_rooms() -> (MapGraph, RoomId, RoomId) {
        let mut g = MapGraph::new();
        g.upsert_room(1, "West of House".into());
        g.upsert_room(2, "Forest Path".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, mapper::direction::Direction::E, 2);
        (g, 1, 2)
    }

    /// SQ-0666: the exits list became a CARD — one line per direction, in the matrix view's
    /// vocabulary, with the destination spelled out. It has to say all four things the old
    /// `dir -> name` list could not: which way a passage comes back, that it does not, that a
    /// direction was tried and refused, and that a direction was never tried at all. The last
    /// two are the coverage the retired untried-exits overlay handed over.
    #[test]
    fn the_exit_card_states_every_direction_in_the_matrix_vocabulary() {
        use mapper::direction::Direction;
        let (mut g, room1, room2) = make_graph_with_rooms();
        g.add_edge(room2, Direction::W, room1); // E is reciprocal
        g.upsert_room(3, "Cellar".into());
        g.set_pos(3, (0, 1));
        g.add_edge(room1, Direction::S, 3); // one-way
        g.add_edge(3, Direction::N, room1);
        g.relabel_connection(3, Direction::N, Direction::NE); // …no: comes back by NE
        g.mark_tried(room1, Direction::W); // typed west, hit a wall

        let text = render_body(&g, &[], room1, None, 70, 30);
        assert!(text.contains("⇄ Forest Path"), "east is reciprocal, and names where it goes:\n{text}");
        assert!(text.contains("→ Cellar"), "south reaches the Cellar:\n{text}");
        assert!(text.contains("back: NE"), "…and the way back is spelled out, not left as `⇠ne`");
        assert!(text.contains("W   × tried, no way through"), "west was typed and refused:\n{text}");
        assert!(text.contains("NE  ·"), "and an untried direction is still listed:\n{text}");
        for d in ["N ", "S ", "E ", "W ", "NE", "NW", "SE", "SW", "Up", "Dn", "In", "Out"] {
            assert!(text.contains(d), "the card lists every travel direction; {d} is missing:\n{text}");
        }
    }

    /// SQ-0685: when a destination shares its bare name with other rooms on the layer, the card
    /// must name it the same way the matrix table's rows and its `⇲`/`⇱out` footnotes do — the
    /// NUMBERED form ("Maze 2"), not the bare, undisambiguating room name every one of those rooms
    /// shares. Both surfaces read the numbering off the same `mapper::matrix::labels`, so they
    /// cannot disagree about what to call the same room.
    #[test]
    fn the_exit_card_names_a_same_named_destination_by_its_matrix_number() {
        use mapper::direction::Direction;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Maze".into());
        g.upsert_room(2, "Maze".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);

        // Independently computed, exactly as the matrix view itself would compute it.
        let expect_room2 = mapper::matrix::labels(&g, mapper::layer::MAIN_LAYER).row_of(2).to_string();
        assert_eq!(expect_room2, "Maze 2");

        let text = render_body(&g, &[], 1, None, 60, 20);
        assert!(
            text.contains(&expect_room2),
            "the exit card names its destination the way the matrix would:\n{text}"
        );
        // SQ-0692: the room's OWN numbered name moved to the dock header, which reads it from
        // the same place — so `display_name` must agree with the matrix too.
        assert_eq!(display_name(&g, 1), "Maze 1");
        assert_eq!(display_name(&g, 2), "Maze 2");
    }

    /// SQ-0685: a destination that LEAVES the layer has no row in this room's `labels` to number
    /// it with — same as the matrix table's own `⇱out` footnote — so it keeps its bare name rather
    /// than showing an empty or wrong number.
    #[test]
    fn a_cross_layer_destination_keeps_its_bare_name_not_a_number_from_the_wrong_layer() {
        use mapper::direction::Direction;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Maze".into());
        g.set_pos(1, (0, 0));
        g.upsert_room(2, "Maze".into()); // same bare name, but on ANOTHER layer
        g.set_pos(2, (0, 0));
        let other = g.new_layer(Some(mapper::layer::MAIN_LAYER), "Elsewhere".into());
        g.set_room_layer(2, other);
        g.add_edge(1, Direction::Down, 2);

        let text = render_body(&g, &[], 1, None, 60, 20);
        assert!(text.contains("Maze"), "the destination is still named");
        assert!(!text.contains("Maze 2"), "…but must not borrow a number that means nothing here:\n{text}");
        assert!(text.contains("Elsewhere"), "the crossing still names the destination layer:\n{text}");
        // Room 1 is alone on Main, so its own display name has no number either.
        assert_eq!(display_name(&g, 1), "Maze");
    }

    /// SQ-0638: a room note packed with multibyte chars (each '€' is 3 bytes)
    /// used to panic — the wrap loop sliced `&notes[offset..end]` at a fixed
    /// BYTE offset that could land mid-character.
    #[test]
    fn room_info_notes_with_multibyte_chars_does_not_panic() {
        let (mut g, room1, _) = make_graph_with_rooms();
        g.set_notes(room1, "€".repeat(12));
        let text = render_body(&g, &[], room1, None, 60, 20);
        assert!(text.contains("€"), "the multibyte note text should still render");
    }

    #[test]
    fn room_info_body_shows_exits_but_not_the_room_name() {
        // SQ-0692: the name (and layer) belong to the dock header now, so the body
        // starts at the notes / exit card. Repeating the name inside the panel that
        // already titles itself with it was the first thing to go when the two
        // floating dialogs became one dock.
        let (g, room1, _room2) = make_graph_with_rooms();
        let text = render_body(&g, &[], room1, None, 60, 20);
        assert!(!text.contains("West of House"), "the body does not repeat the header's name:\n{text}");
        assert!(text.contains("Exits:"), "it starts at the exit card:\n{text}");
        assert!(text.contains("Forest Path"), "…which names the destination");
    }

    /// SQ-1257 Phase 3: a room the story has renamed shows an "Also seen as: ..." line, under
    /// the notes and above the exit card, listing every OTHER name in first-seen order — never
    /// the current one.
    #[test]
    fn room_info_shows_also_seen_as_when_the_room_has_aliases() {
        let (mut g, room1, _) = make_graph_with_rooms();
        g.upsert_room(room1, "Confusing Passage".into());
        g.upsert_room(room1, "Strange Place".into());
        let text = render_body(&g, &[], room1, None, 60, 20);
        assert!(
            text.contains("Also seen as: West of House, Confusing Passage"),
            "both older names appear, in first-seen order:\n{text}"
        );
        assert!(!text.contains("Strange Place,"), "the CURRENT name is never listed as an alias:\n{text}");
    }

    /// The companion case: a room with no rename shows no "Also seen as" line at all.
    #[test]
    fn room_info_shows_no_also_seen_as_line_without_aliases() {
        let (g, room1, _) = make_graph_with_rooms();
        let text = render_body(&g, &[], room1, None, 60, 20);
        assert!(!text.contains("Also seen as"), "no aliases, so no line:\n{text}");
    }

    #[test]
    fn room_info_no_objects_for_non_current_room() {
        let (g, room1, room2) = make_graph_with_rooms();
        // room2 is not the current room, so no objects section — even with objects passed in.
        let text = render_body(&g, &["lamp".to_string()], room2, Some(room1), 60, 20);
        assert!(!text.contains("Here:"), "objects section should not appear for a non-current room");
        assert!(!text.contains("lamp"), "nor the objects themselves");
    }

    #[test]
    fn room_info_lists_objects_for_the_current_room() {
        let (g, room1, _) = make_graph_with_rooms();
        let text = render_body(&g, &["brass lantern".to_string()], room1, Some(room1), 60, 24);
        assert!(text.contains("Here:"), "the current room's objects get a section:\n{text}");
        assert!(text.contains("brass lantern"), "{text}");
    }

    #[test]
    fn room_info_body_is_silent_for_a_missing_room() {
        let g = MapGraph::new();
        let text = render_body(&g, &[], 99, None, 60, 20);
        assert!(text.trim().is_empty(), "a room that is not in the graph draws nothing:\n{text}");
    }

    // ── The exit card's columns (SQ-0694) ────────────────────────────────────

    /// The card spends WIDTH instead of height: the column count falls out of the available
    /// width against the widest line, and a narrow dock still gets the single column it always
    /// had. This is the whole point of the change — the card was a fixed thirteen rows and the
    /// Info body's height driver.
    #[test]
    fn the_card_takes_more_columns_as_the_dock_widens_and_one_when_narrow() {
        let w = vec![10usize; 12];

        // Too narrow for two 10-cell columns plus the gap → one column, twelve rows.
        let one = layout_card(&w, 15);
        assert_eq!(one.cols.len(), 1, "a narrow dock keeps the single column");
        assert_eq!(one.rows, 12);

        // 10 + 2 + 10 = 22 fits → two columns of six.
        let two = layout_card(&w, 22);
        assert_eq!(two.cols.len(), 2);
        assert_eq!(two.rows, 6);

        // 10 + 2 + 10 + 2 + 10 = 34 fits → three columns of four…
        let three = layout_card(&w, 34);
        assert_eq!(three.cols.len(), 3);
        assert_eq!(three.rows, 4);

        // …and no wider, however much room there is: three is the cap.
        let wide = layout_card(&w, 400);
        assert_eq!(wide.cols.len(), MAX_CARD_COLS, "the card never fans past three columns");
        assert_eq!(wide.rows, 4);
    }

    /// Column-major, and that is not merely tidiness: `MATRIX_DIRS` is three groups of four, so a
    /// three-column grid puts the cardinals, the diagonals and the portals each in their own
    /// column. Row-major would deal them across the grid like cards.
    #[test]
    fn three_columns_group_the_cardinals_the_diagonals_and_the_portals() {
        use mapper::direction::Direction as D;
        let plan = layout_card(&[10usize; 12], 40);
        assert_eq!((plan.rows, plan.cols.len()), (4, 3));

        // Entry i sits at column i / rows — read the direction back out of MATRIX_DIRS.
        let column_of = |d: D| {
            mapper::matrix::MATRIX_DIRS.iter().position(|&x| x == d).expect("a card direction")
                / plan.rows
        };
        for d in [D::N, D::S, D::E, D::W] {
            assert_eq!(column_of(d), 0, "{d:?} is a cardinal: first column");
        }
        for d in [D::NE, D::NW, D::SE, D::SW] {
            assert_eq!(column_of(d), 1, "{d:?} is a diagonal: second column");
        }
        for d in [D::Up, D::Down, D::In, D::Out] {
            assert_eq!(column_of(d), 2, "{d:?} is a portal: third column");
        }
    }

    /// Widths are DISPLAY CELLS, not char counts. A full-width destination name occupies two
    /// columns per glyph, so measuring it as one would lay a column over its neighbour.
    #[test]
    fn the_card_measures_full_width_names_in_cells_not_chars() {
        // Six CJK ideographs: 6 chars, 12 cells.
        let name = "\u{8ff7}\u{5bab}\u{306e}\u{90e8}\u{5c4b}\u{3067}";
        assert_eq!(name.chars().count(), 6);
        assert_eq!(crate::textwidth::str_cells(name), 12);

        // Two entries, one of them that name: a 14-cell budget holds ONE 12-cell column, not two
        // (which a char count would have called 6 + 2 + 6 = 14 and wrongly accepted).
        let widths = vec![crate::textwidth::str_cells(name), 2];
        assert_eq!(layout_card(&widths, 14).cols.len(), 1, "cells, not chars, decide the fit");
        assert_eq!(layout_card(&widths, 16).cols.len(), 2, "…and 12 + 2 + 2 does fit");
    }

    /// The DRAWN card measures its lines in display cells too, not just the pure layout function.
    /// A full-width destination name is twice as wide on screen as its char count claims, so a
    /// card measured in chars packs a column too many and the columns overlap on screen.
    #[test]
    fn the_drawn_card_measures_its_own_lines_in_cells_not_chars() {
        use mapper::direction::Direction;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Start".into());
        // Six CJK ideographs: 6 chars, 12 cells.
        g.upsert_room(2, "\u{8ff7}\u{5bab}\u{306e}\u{90e8}\u{5c4b}\u{3067}".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);

        // At 34 cells the honest measurement admits two columns of six; a char count would call
        // the wide line 14 wide instead of 20 and squeeze in a third column of four.
        let text = render_body(&g, &[], 1, None, 34, 14);
        let card_rows = text.lines().filter(|l| l.contains('·') || l.contains('⇢')).count();
        assert_eq!(
            card_rows, 6,
            "the wide name is measured in cells, so the card is two columns of six:\n{text}"
        );
    }

    /// Nothing fits — one column is wider than the dock. The layout still reports one column, the
    /// caller clips into it, and the rows that fit still read.
    #[test]
    fn an_over_wide_card_falls_back_to_one_clipped_column() {
        let plan = layout_card(&[40, 40, 40], 10);
        assert_eq!(plan.cols, vec![(0, 10)], "one column, clipped to the dock");
        assert_eq!(plan.rows, 3);
    }

    #[test]
    fn an_empty_card_lays_out_to_nothing() {
        assert_eq!(layout_card(&[], 80), CardLayout { rows: 0, cols: Vec::new() });
    }

    /// The rendered card really is a grid: at a dock width a split-pane map actually has, three
    /// directions from different groups share a row, and the whole card fits in four rows.
    #[test]
    fn the_rendered_card_puts_three_directions_on_one_row() {
        let (g, room1, _) = make_graph_with_rooms();
        let text = render_body(&g, &[], room1, None, 58, 12);
        let card_row = text
            .lines()
            .find(|l| l.contains("N ") && l.contains("NE"))
            .unwrap_or_else(|| panic!("a cardinal and a diagonal must share a row:\n{text}"));
        assert!(card_row.contains("Up"), "…and a portal too: {card_row:?}");

        // Four card rows, not twelve: every direction is drawn, in a quarter of the height.
        let drawn = text.lines().filter(|l| l.contains('·') || l.contains('⇄')).count();
        assert_eq!(drawn, 4, "the twelve directions occupy four rows:\n{text}");
        for d in ["N ", "S ", "E ", "W ", "NE", "NW", "SE", "SW", "Up", "Dn", "In", "Out"] {
            assert!(text.contains(d), "{d} is still on the card:\n{text}");
        }
    }

    /// Graceful degradation survives the grid: a dock too short for every card row draws the rows
    /// that fit, in full, rather than dropping the card or smearing it.
    #[test]
    fn a_short_dock_draws_the_card_rows_that_fit() {
        let (g, room1, _) = make_graph_with_rooms();
        // Two body rows total: "Exits:" plus ONE card row.
        let text = render_body(&g, &[], room1, None, 58, 2);
        assert!(text.contains("Exits:"), "the section label still draws:\n{text}");
        assert!(text.contains("N "), "the first card row draws in full:\n{text}");
        assert!(text.contains("NE"), "…all three of its columns:\n{text}");
        assert!(!text.contains("S  "), "and the rows that do not fit are simply absent:\n{text}");
    }

    #[test]
    fn room_info_body_zero_area_does_not_panic() {
        let (g, room1, _) = make_graph_with_rooms();
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        let theme = test_theme();
        draw_room_info_body(
            &g, &[], room1, None, Rect::new(0, 0, 0, 0), &mut buf, &theme,
            Style::default(), Style::default(),
        );
    }
}
