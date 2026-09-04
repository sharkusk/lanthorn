//! The drawn map rendered against REAL player data (SQ-0688 / SQ-0689).
//!
//! `unit_tests/zork1_underground_map.json` is a verbatim copy of the `map.json` from a lanthorn
//! archive: one player's partial mapping of Zork I, whose underground layer holds the two shapes
//! these bugs need — a one-way NE passage arriving on a box corner (North-South Passage → Deep
//! Canyon), and a staircase collapsed into a compass connector (Chasm's Up return to the
//! East-West Passage, folded into the passage's own N edge).

use mapper::layer::MapView;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use app::render::map::render_map_layered;
use app::state::AppState;

const DEEP_CANYON: mapper::graph::RoomId = 170;
const CHASM: mapper::graph::RoomId = 112;
const EW_PASSAGE: mapper::graph::RoomId = 136;
const NS_PASSAGE: mapper::graph::RoomId = 183;
const LAYER: u16 = 6;

fn underground() -> mapper::mapper::Mapper {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../unit_tests/zork1_underground_map.json");
    let json = std::fs::read_to_string(path).expect("fixture");
    let mut m = mapper::persist::from_json(&json).expect("parse");
    // The save's positions are negative; translate the layer into the viewport, which is a
    // camera concern the renderer under test does not own.
    m.graph.set_layer_view(LAYER, Some(MapView::Drawn));
    for id in m.graph.rooms_in_layer(LAYER) {
        if let Some(pos) = m.graph.room(id).and_then(|r| r.pos) {
            m.graph.set_pos(id, (pos.0 + 5, pos.1 + 4));
        }
    }
    m
}

fn draw(m: &mapper::mapper::Mapper) -> (Buffer, Vec<(mapper::graph::RoomId, Rect)>) {
    let mut st = AppState::default();
    st.set_viewed_layer(Some(LAYER));
    let rm = mapper::render::render_layer(&m.graph, LAYER);
    let area = Rect::new(0, 0, 150, 60);
    let mut buf = Buffer::empty(area);
    let hits = render_map_layered(&rm, &m.graph, &st, area, &mut buf);
    (buf, hits.room_rects)
}

fn rect_of(hits: &[(mapper::graph::RoomId, Rect)], id: mapper::graph::RoomId) -> Rect {
    hits.iter().find(|(h, _)| *h == id).map(|(_, r)| *r).expect("room drawn")
}

fn sym(buf: &Buffer, x: u16, y: u16) -> &str {
    buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" ")
}

/// SQ-0688: `183 -NE-> 170` is one-way (Deep Canyon's only exit is Down), and arrows are only
/// ever OUTGOING — an arrow on a room border is that room's own exit. The arrival used to stamp
/// a side-derived `▶` on Deep Canyon's SW corner, which read as "east leads out of here" when
/// nothing is known to leave that way. The corner now carries NO arrow at all: the departure
/// corner on North-South Passage wears the `↗`, and the line ends bare on the canyon.
#[test]
fn the_one_way_ne_passage_puts_no_arrow_on_deep_canyons_corner() {
    let m = underground();
    let (buf, hits) = draw(&m);
    let canyon = rect_of(&hits, DEEP_CANYON);
    let ns = rect_of(&hits, NS_PASSAGE);

    // The departure corner (North-South Passage's top-right) wears the travel diagonal.
    let ne_corner = (ns.x + ns.width - 1, ns.y);
    assert_eq!(sym(&buf, ne_corner.0, ne_corner.1), "\u{2197}", "the departure corner wears ↗");

    // Deep Canyon's outline carries no arrow that is not its own exit: no ▶ (the old side-arrow
    // arrival), no ↗ (an arrival is not an exit). Its genuine Down exit keeps its ↓.
    for y in canyon.y..canyon.y + canyon.height {
        for x in canyon.x..canyon.x + canyon.width {
            let s = sym(&buf, x, y);
            assert_ne!(s, "\u{25b6}", "no ▶ on Deep Canyon at ({x},{y})");
            assert_ne!(s, "\u{2197}", "no ↗ on Deep Canyon at ({x},{y})");
        }
    }
    let bottom = canyon.y + canyon.height - 1;
    let downs = (canyon.x..canyon.x + canyon.width)
        .filter(|&x| sym(&buf, x, bottom) == "\u{2193}")
        .count();
    assert_eq!(downs, 1, "the canyon's own Down exit still shows its ↓");
}

/// SQ-0689: `112 -Up-> 136` is the Chasm's known return to the East-West Passage, collapsed by
/// the router into the passage's own `N` connector as a secondary. It used to draw NOTHING — the
/// map showed a plain corridor and a player reading it had no way to know the Chasm led anywhere
/// but down its S exit. The staircase now stamps `↑` on the Chasm's bottom border, beside the
/// shared line it travels.
#[test]
fn the_chasms_collapsed_up_return_is_stamped_on_its_border() {
    let m = underground();
    let (buf, hits) = draw(&m);
    let chasm = rect_of(&hits, CHASM);
    let ew = rect_of(&hits, EW_PASSAGE);

    // The marker sits between the two rooms it connects — on the border facing the passage.
    assert!(ew.y > chasm.y, "fixture sanity: the passage is drawn south of the Chasm");
    let bottom = chasm.y + chasm.height - 1;
    let ups: Vec<u16> = (chasm.x..chasm.x + chasm.width)
        .filter(|&x| sym(&buf, x, bottom) == "\u{2191}")
        .collect();
    assert_eq!(ups.len(), 1, "exactly one ↑ on the Chasm's bottom border");
    // …and ON the line it travels: the cell directly below the ↑ is the shared lane itself,
    // not a neighbouring column (SQ-0689 follow-up — the marker is aligned with its path).
    assert_eq!(
        sym(&buf, ups[0], bottom + 1),
        "\u{2502}",
        "the ↑ lines up with the corridor it rides"
    );
}
