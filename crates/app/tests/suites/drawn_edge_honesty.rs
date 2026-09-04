//! The drawn view's honesty markers (SQ-0666).
//!
//! The drawn map used to say the same thing about a two-way corridor, a one-way drop and a
//! passage whose return is a different direction: one arrowhead leaving the origin. This pins the
//! three ways that changed — an arrival arrowhead on a one-way edge, per-kind style selectors, and
//! the self-loop badge — plus the promise that nothing LOOKS different until someone styles it.

use mapper::direction::Direction;
use mapper::graph::MapGraph;
use ratatui::{buffer::Buffer, layout::Rect};

use app::render::map::render_map;
use app::state::AppState;

fn render(g: &MapGraph) -> String {
    let rm = mapper::render::render(g);
    let mut st = AppState::default();
    st.scroll = rm.bounds.0;
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    render_map(&rm, &st, area, &mut buf);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" ").to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn pair(reciprocal: bool) -> MapGraph {
    let mut g = MapGraph::new();
    g.upsert_room(1, "A".into());
    g.upsert_room(2, "B".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (1, 0));
    g.add_edge(1, Direction::E, 2);
    if reciprocal {
        g.add_edge(2, Direction::W, 1);
    }
    g
}

/// Every arrow on a room border is that room's own EXIT — the map's one arrow rule (SQ-0688,
/// reversing the arrival arrow SQ-0666 added: an inbound arrow on B's border read as an exit B
/// does not have). A one-way passage therefore shows exactly one arrow, at its departure, and
/// the line ending bare on B IS the "no known way back" reading; a reciprocal pair shows each
/// room's own departure.
#[test]
fn a_one_way_passage_shows_only_its_departure_arrow() {
    let arrows = AppState::default().symbols.arrows;
    let oneway = render(&pair(false));
    let both = render(&pair(true));

    // A→(east)→B. The departure arrow on A points east in both.
    assert!(oneway.contains(arrows.east), "the departure arrow is unchanged");
    // B's border: the reciprocal draws B's own westward departure; the one-way draws NOTHING —
    // B has no exit along this line, so no arrow of any direction sits on it.
    assert!(both.contains(arrows.west), "a reciprocal pair shows B leaving west");
    assert!(!oneway.contains(arrows.west), "a one-way pair has no westward exit to show");
    assert_eq!(
        oneway.matches(arrows.east).count(),
        1,
        "one-way: an arrow leaving A and a bare line end at B\n{oneway}"
    );
}

/// Both new edge selectors default to the plain connector, so an existing map is pixel-identical
/// until a themer chooses otherwise. The hook exists; the appearance does not change.
#[test]
fn the_edge_kind_selectors_default_to_the_connector_appearance() {
    let st = AppState::default();
    let connector = st.colors.theme.get("map.connector").style;
    assert_eq!(st.colors.theme.get("map.edge:oneway").style, connector);
    assert_eq!(st.colors.theme.get("map.edge:asym").style, connector);
}

/// A self-loop is a badge on the box, never a drawn loop: a line leaving a room and coming back
/// would need its own lane, cross whatever sits beside the room, and say no more than `↩w` does.
#[test]
fn a_self_loop_is_a_badge_on_the_box_and_never_a_connector() {
    let mut g = pair(true);
    let before = render(&g);
    assert!(!before.contains('↩'));

    assert!(g.add_self_loop(1, Direction::W));
    let after = render(&g);
    assert!(after.contains("↩w"), "the badge names the direction that loops:\n{after}");

    // It is not a route: no extra connector cells, and the layout did not move a thing.
    assert_eq!(g.room(1).unwrap().pos, Some((0, 0)), "a loop has no geometry to lay out");
    assert_eq!(g.room(2).unwrap().pos, Some((1, 0)));
    assert!(
        g.connections().iter().all(|c| !c.distorted),
        "and a loop is never 'distorted' — there is nothing for it to disagree with: {:?}",
        g.connections()
    );

    // Several loops share one badge.
    g.add_self_loop(1, Direction::NE);
    assert!(render(&g).contains("↩wne"), "both looping directions ride one badge");
}

/// The whole reason self-loops had to be kept out of the layout: a compass-offset placer asked
/// where a room sits relative to itself would answer "distorted", forever, on every relayout.
#[test]
fn a_self_loop_survives_a_full_relayout_without_disturbing_it() {
    let mut g = MapGraph::new();
    for (id, n) in [(1u32, "A"), (2, "B"), (3, "C")] {
        g.upsert_room(id, n.into());
    }
    for (a, b) in [(1u32, 2u32), (2, 3)] {
        g.add_edge(a, Direction::E, b);
        g.add_edge(b, Direction::W, a);
    }
    mapper::layout::relayout_auto(&mut g);
    let clean: Vec<_> = g.rooms().map(|r| (r.id, r.pos)).collect();
    let clean_distorted = g.connections().iter().filter(|c| c.distorted).count();

    g.add_self_loop(2, Direction::N);
    mapper::layout::relayout_auto(&mut g);
    assert_eq!(g.rooms().map(|r| (r.id, r.pos)).collect::<Vec<_>>(), clean, "no room moved");
    assert_eq!(
        g.connections().iter().filter(|c| c.distorted).count(),
        clean_distorted,
        "and no edge was newly flagged distorted"
    );
    assert_eq!(g.self_loops(2), vec![Direction::N], "the loop is still recorded afterwards");
}
