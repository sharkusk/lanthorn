//! The Map tab's room card (SQ-1688), driven from a real player's map
//! (`unit_tests/advent_maze_map.json`).
//!
//! The card is part of the Map tab's layout (the map rect shrinks to make room
//! for it), so most of what is asserted here goes through
//! `compute_pane_layout_with_card`, the way `main.rs` draws it. Colours are
//! asserted in both `honor_game_colours` modes: the card is app chrome.

use mapper::graph::{MapGraph, RoomId};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use app::input::{apply_action, Action};
use app::journal::JournalTab;
use app::keymap::Context;
use app::layout::{compute_pane_layout_with_card, PaneLayout};
use app::map_card::{self, CardClick, MapCardButton};
use app::room_menu::ROOM_MENU;
use app::slash::parse_in_context;
use app::state::{AppState, RoomDockView};

const FRAME: Rect = Rect { x: 0, y: 0, width: 120, height: 40 };

fn advent() -> mapper::mapper::Mapper {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../unit_tests/advent_maze_map.json");
    let json = std::fs::read_to_string(&path).expect("fixture readable");
    mapper::persist::from_json(&json).expect("valid map file")
}

fn id_of(g: &MapGraph, label: &str) -> RoomId {
    g.rooms().find(|r| r.label() == label).map(|r| r.id).expect("room")
}

fn layout(st: &AppState, g: &MapGraph, frame: Rect) -> PaneLayout {
    let base = compute_pane_layout_with_card(frame, st, 0);
    compute_pane_layout_with_card(frame, st, map_card::rows_for(g, st, base.journal_body))
}

fn draw(st: &AppState, g: &MapGraph, pl: &PaneLayout, frame: Rect) -> (Buffer, Vec<(MapCardButton, Rect)>) {
    let mut buf = Buffer::empty(frame);
    let hits = map_card::draw(g, st, pl.map_card, &mut buf);
    (buf, hits)
}

fn text(buf: &Buffer, r: Rect) -> String {
    (r.y..r.bottom())
        .map(|y| (r.x..r.right()).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

const LONG: &str = "You are standing at the end of a road before a small brick building. Around you is a forest. A small stream flows out of the building and down a gully. The road continues beyond; the stream chuckles over the rocks and a faint breeze carries the scent of pine and wet stone across the clearing, and further on there is still more to say about it all.";

fn pinned(honor: bool) -> (AppState, mapper::mapper::Mapper, RoomId) {
    let mut m = advent();
    let id = id_of(&m.graph, "Inside Building");
    m.graph.set_description(id, Some(LONG.to_string()), 12);
    let mut st = AppState::default();
    st.config.honor_game_colours = honor;
    apply_action(Action::PinRoomDock(id, RoomDockView::Info), &mut st, &mut m);
    (st, m, id)
}

#[test]
fn pinning_a_room_shows_the_card_and_shrinks_the_map_and_unpinning_hides_it() {
    for honor in [true, false] {
        let (mut st, mut m, _) = pinned(honor);
        let bare = compute_pane_layout_with_card(FRAME, &st, 0);
        let pl = layout(&st, &m.graph, FRAME);
        assert!(pl.map_card.height >= 3, "honor={honor}: {:?}", pl.map_card);
        assert_eq!(pl.map.height + pl.map_card.height, bare.map.height, "the card is carved off the map");
        assert_eq!(pl.map_card.y, pl.map.bottom(), "directly under the map");
        assert_eq!((pl.map.x, pl.map.width), (pl.map_card.x, pl.map_card.width));
        assert!(pl.map_card.height <= bare.journal_body.height / 3 + 3, "capped near a third");

        apply_action(Action::UnpinRoomDock, &mut st, &mut m);
        let pl = layout(&st, &m.graph, FRAME);
        assert_eq!(pl.map_card, Rect::default(), "honor={honor}: unpinning hides the card");
        assert_eq!(pl.map, bare.map, "and the map has its full rect back");
    }
}

#[test]
fn nothing_selected_means_no_card_even_with_a_current_room() {
    let mut m = advent();
    let here = id_of(&m.graph, "Inside Building");
    m.graph.set_current(here);
    let st = AppState::default();
    assert!(st.selected_room.is_none());
    assert_eq!(map_card::rows_for(&m.graph, &st, Rect::new(0, 0, 60, 30)), 0);
    assert_eq!(layout(&st, &m.graph, FRAME).map_card, Rect::default());
}

#[test]
fn the_card_is_only_on_the_map_tab() {
    let (mut st, m, _) = pinned(true);
    st.set_journal_tab(JournalTab::Room);
    assert_eq!(layout(&st, &m.graph, FRAME).map_card, Rect::default());
}

#[test]
fn card_shows_name_layer_description_seen_and_the_four_buttons() {
    for honor in [true, false] {
        let (st, m, id) = pinned(honor);
        let pl = layout(&st, &m.graph, FRAME);
        let (buf, hits) = draw(&st, &m.graph, &pl, FRAME);
        let t = text(&buf, pl.map_card);
        let layer = m.graph.layer_name(m.graph.layer_of(id));
        assert!(t.contains("Inside Building"), "{t}");
        assert!(t.contains(&format!("on {layer}")), "{t}");
        assert!(t.contains("You are standing"), "{t}");
        assert!(t.contains("Seen at move 12"), "{t}");
        let labels: Vec<_> = hits.iter().map(|(b, _)| *b).collect();
        assert_eq!(labels, MapCardButton::ALL.to_vec(), "{t}");
        for (b, r) in &hits {
            assert!(pl.map_card.contains(ratatui::layout::Position { x: r.x, y: r.y }));
            assert_eq!(text(&buf, *r), b.label());
        }
        // Not painted with the game's palette in either mode: the ground is the
        // theme's, identical across modes.
        let (st2, m2, _) = pinned(!honor);
        let pl2 = layout(&st2, &m2.graph, FRAME);
        let (buf2, _) = draw(&st2, &m2.graph, &pl2, FRAME);
        assert_eq!(buf, buf2, "app chrome does not depend on honor_game_colours");
    }
}

#[test]
fn a_long_description_truncates_with_an_ellipsis_inside_the_cap() {
    let (st, m, _) = pinned(true);
    let narrow = Rect::new(0, 0, 40, 30);
    let pl = layout(&st, &m.graph, narrow);
    let bare = compute_pane_layout_with_card(narrow, &st, 0);
    assert!(pl.map_card.height <= (bare.journal_body.height / 3).max(4), "{:?}", pl.map_card);
    let (buf, _) = draw(&st, &m.graph, &pl, narrow);
    let t = text(&buf, pl.map_card);
    assert!(t.contains('…'), "truncated text ends in an ellipsis: {t}");
    assert!(!t.contains("still more to say"), "the tail is cut: {t}");
    assert!(t.contains("Seen at move 12") && t.contains("Details"), "the fixed rows survive: {t}");
}

#[test]
fn no_description_renders_no_description_block() {
    let mut m = advent();
    let id = id_of(&m.graph, "Inside Building");
    let mut st = AppState::default();
    apply_action(Action::PinRoomDock(id, RoomDockView::Info), &mut st, &mut m);
    let wide = Rect::new(0, 0, 220, 40);
    let pl = layout(&st, &m.graph, wide);
    // rule + name + one row of buttons when the Journal is wide enough.
    assert_eq!(pl.map_card.height, 3, "{:?}", pl.map_card);
    let (buf, _) = draw(&st, &m.graph, &pl, wide);
    let t = text(&buf, pl.map_card);
    assert!(!t.contains("Seen at"), "{t}");
    assert_eq!(t.lines().count(), 3);
}

#[test]
fn narrow_widths_wrap_the_buttons_with_a_gap_and_never_overlap() {
    for w in [30u16, 44, 60] {
        let frame = Rect::new(0, 0, w * 2, 40);
        let (st, m, _) = pinned(true);
        let pl = layout(&st, &m.graph, frame);
        if pl.map_card.height == 0 {
            continue;
        }
        let (_, hits) = draw(&st, &m.graph, &pl, frame);
        for (i, (_, a)) in hits.iter().enumerate() {
            assert!(a.right() <= pl.map_card.right(), "w={w}: {a:?} inside {:?}", pl.map_card);
            for (_, b) in &hits[i + 1..] {
                if a.y == b.y {
                    assert!(b.x > a.right() || a.x > b.right(), "w={w}: a gap between {a:?} and {b:?}");
                }
            }
        }
    }
    // A pane this narrow wraps to more than one button row.
    let frame = Rect::new(0, 0, 60, 40);
    let (st, m, _) = pinned(true);
    let pl = layout(&st, &m.graph, frame);
    let (_, hits) = draw(&st, &m.graph, &pl, frame);
    let rows: std::collections::BTreeSet<u16> = hits.iter().map(|(_, r)| r.y).collect();
    assert!(rows.len() > 1, "{hits:?}");
}

#[test]
fn the_buttons_run_exactly_what_the_right_click_menu_runs() {
    for b in MapCardButton::ALL {
        let Some(cmd) = b.command() else { continue };
        let item = ROOM_MENU.iter().find(|it| it.command == cmd).expect("a menu item has this command");
        assert_eq!(
            parse_in_context(cmd, '/', Context::Map),
            parse_in_context(item.command, '/', Context::Map),
        );
    }
    assert_eq!(MapCardButton::Rename.command(), Some("rename-room"));
    assert_eq!(MapCardButton::Notes.command(), Some("edit-notes"));
    assert_eq!(MapCardButton::Move.command(), Some("move-region"));
}

#[test]
fn details_switches_to_the_room_tab() {
    let (mut st, mut m, id) = pinned(true);
    apply_action(MapCardButton::Details.action().expect("Details is an action"), &mut st, &mut m);
    assert_eq!(st.journal_tab, JournalTab::Room);
    assert_eq!(st.selected_room, Some(id), "the pin survives the switch");
    assert!(MapCardButton::Details.command().is_none());
}

#[test]
fn clicks_on_the_card_body_hit_nothing_and_the_map_above_is_untouched() {
    let (st, m, _) = pinned(true);
    let pl = layout(&st, &m.graph, FRAME);
    let (_, hits) = draw(&st, &m.graph, &pl, FRAME);
    // Body: the name row.
    let body = (pl.map_card.x + 1, pl.map_card.y + 1);
    assert_eq!(map_card::click_at(pl.map_card, &hits, body.0, body.1), Some(CardClick::Body));
    // The card is not inside the map rect, so the map's own hit-test never sees it.
    let p = ratatui::layout::Position { x: body.0, y: body.1 };
    assert!(!pl.map.contains(p), "{:?} vs {:?}", pl.map, pl.map_card);
    // A button.
    let (b, r) = hits[0];
    assert_eq!(map_card::click_at(pl.map_card, &hits, r.x, r.y), Some(CardClick::Button(b)));
    // A point in the map is outside the card.
    assert_eq!(map_card::click_at(pl.map_card, &hits, pl.map.x + 2, pl.map.y + 2), None);
    // The map's hit-testing above the card: its rect keeps its origin and width.
    let bare = compute_pane_layout_with_card(FRAME, &st, 0);
    assert_eq!((pl.map.x, pl.map.y, pl.map.width), (bare.map.x, bare.map.y, bare.map.width));
}

#[test]
fn a_journal_too_short_for_the_card_hides_it_rather_than_crush_the_map() {
    let (st, m, _) = pinned(true);
    let frame = Rect::new(0, 0, 120, 9);
    assert_eq!(layout(&st, &m.graph, frame).map_card, Rect::default());
}
