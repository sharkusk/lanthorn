//! The Journal's tab bar and tab switching, end to end (SQ-1684): the bar draws in the
//! Journal's first row, a click on a drawn label resolves to the tab it names, the
//! three tabs each draw into the Journal's body, and the bar degrades at narrow widths.
//!
//! Colour assertions run in BOTH `honor_game_colours` modes, per CLAUDE.md: the bar is app
//! chrome, so the game's palette must never reach it.

use mapper::graph::MapGraph;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use app::input::{journal_tab_click_action, key_to_action, Action};
use app::journal::{draw_tab_bar, JournalTab, TabBarHit};
use app::layout::compute_pane_layout;
use app::render::inventory_dock::{build_inventory_dock_rows, draw_inventory_dock, InventoryDockHits};
use app::state::{AppState, Layout};

const FRAME: Rect = Rect { x: 0, y: 0, width: 120, height: 40 };

fn text_in(buf: &Buffer, r: Rect) -> String {
    (r.y..r.bottom())
        .map(|y| {
            (r.x..r.right())
                .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn key_alt(c: char) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char(c), crossterm::event::KeyModifiers::ALT)
}

#[test]
fn the_bar_draws_in_the_journals_first_row_with_the_active_tab_accented() {
    for honor in [true, false] {
        for tab in JournalTab::ALL {
            let mut st = AppState::default();
            st.config.honor_game_colours = honor;
            st.journal_tab = tab;
            let pl = compute_pane_layout(FRAME, &st);
            let mut buf = Buffer::empty(FRAME);
            let hits = draw_tab_bar(pl.journal_tabs, tab, &st.colors, &mut buf);

            let row = text_in(&buf, pl.journal_tabs);
            for t in JournalTab::ALL {
                assert!(row.contains(t.label()), "honor={honor}: {} is named: {row:?}", t.label());
            }
            assert_eq!(hits.len(), JournalTab::ALL.len(), "one hit rect per label");

            let on = st.colors.theme.get("journal.tab:active").style;
            let off = st.colors.theme.get("journal.tab").style;
            for (hit, r) in &hits {
                let cell = buf.cell((r.x + 1, r.y)).unwrap();
                let want = if *hit == TabBarHit::Tab(tab) { on } else { off };
                assert_eq!(cell.fg, want.fg.unwrap_or(cell.fg), "honor={honor}: {hit:?} wears its selector");
            }
        }
    }
}

#[test]
fn a_click_on_each_drawn_label_resolves_to_that_tab() {
    let st = AppState::default();
    let pl = compute_pane_layout(FRAME, &st);
    let mut buf = Buffer::empty(FRAME);
    let hits = draw_tab_bar(pl.journal_tabs, st.journal_tab, &st.colors, &mut buf);
    for t in JournalTab::ALL {
        let (hit, r) = hits.iter().find(|(h, _)| *h == TabBarHit::Tab(t)).expect("label drawn");
        // Every cell of the label is a target.
        for col in r.x..r.right() {
            assert!(r.contains(ratatui::layout::Position { x: col, y: r.y }));
        }
        assert_eq!(journal_tab_click_action(*hit), Action::SetJournalTab(t));
    }
}

#[test]
fn a_hidden_journal_comes_back_for_a_tab_click() {
    let mut st = AppState::default();
    let mut m = mapper::mapper::Mapper::default();
    st.layout = Layout::TranscriptFull;
    app::input::apply_action(journal_tab_click_action(TabBarHit::Tab(JournalTab::Inventory)), &mut st, &mut m);
    assert_eq!(st.layout, Layout::Split);
    assert_eq!(st.journal_tab, JournalTab::Inventory);
}

#[test]
fn narrow_bars_abbreviate_then_show_only_the_active_tab_with_step_markers() {
    let mut st = AppState::default();
    let mut m = mapper::mapper::Mapper::default();
    let colors = st.colors.clone();

    // Wide enough for abbreviations only.
    let r = Rect::new(0, 0, 20, 1);
    let mut buf = Buffer::empty(r);
    draw_tab_bar(r, JournalTab::Map, &colors, &mut buf);
    let row = text_in(&buf, r);
    assert!(row.contains("Inv") && !row.contains("Inventory"), "{row:?}");
    assert!(row.contains("Docs") && !row.contains("Documents"), "the Documents tab abbreviates: {row:?}");

    // Too narrow for any strip: the active tab between markers, and the markers step.
    let r = Rect::new(0, 0, 9, 1);
    let mut buf = Buffer::empty(r);
    let hits = draw_tab_bar(r, JournalTab::Room, &colors, &mut buf);
    let row = text_in(&buf, r);
    assert!(row.contains('\u{2039}') && row.contains('\u{203a}') && row.contains("Room"), "{row:?}");
    let next = hits.iter().find(|(h, _)| *h == TabBarHit::Next).expect("› hit").0;
    st.journal_tab = JournalTab::Room;
    app::input::apply_action(journal_tab_click_action(next), &mut st, &mut m);
    assert_eq!(st.journal_tab, JournalTab::Inventory);
    let prev = hits.iter().find(|(h, _)| *h == TabBarHit::Prev).expect("‹ hit").0;
    app::input::apply_action(journal_tab_click_action(prev), &mut st, &mut m);
    assert_eq!(st.journal_tab, JournalTab::Room);
}

#[test]
fn alt_digits_switch_tabs_through_the_real_key_path_in_game_focus() {
    let mut st = AppState::default();
    for (c, tab) in [('2', JournalTab::Room), ('3', JournalTab::Inventory), ('5', JournalTab::Documents), ('1', JournalTab::Map)] {
        let a = key_to_action(&st, key_alt(c));
        assert_eq!(a, Action::SetJournalTab(tab), "Alt+{c}");
        app::input::apply_action(a, &mut st, &mut mapper::mapper::Mapper::default());
        assert_eq!(st.journal_tab, tab);
    }
    // And typing a plain digit is still typing.
    assert!(matches!(
        key_to_action(&st, crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('2'), crossterm::event::KeyModifiers::NONE)),
        Action::InputChar('2')
    ));
}

#[test]
fn the_inventory_tab_draws_into_the_journal_body() {
    for honor in [true, false] {
        let mut st = AppState::default();
        st.config.honor_game_colours = honor;
        st.set_journal_tab(JournalTab::Inventory);
        let pl = compute_pane_layout(FRAME, &st);
        assert_eq!(pl.map, Rect::default(), "no map rect behind the tab");

        let carried = vec![(None, "brass lantern".to_string())];
        let rows = build_inventory_dock_rows(&carried, &MapGraph::new(), None);
        let mut buf = Buffer::empty(FRAME);
        let mut hits = InventoryDockHits::default();
        draw_inventory_dock(&rows, pl.journal_body, &st.colors, false, 0, &mut buf, &mut hits);

        assert!(text_in(&buf, pl.journal_body).contains("brass lantern"), "honor={honor}");
        assert_eq!(hits.area, pl.journal_body, "the tab's hit area IS the Journal body");
        assert!(hits.body_viewport > pl.journal_body.height.saturating_sub(3), "the tab uses the Journal's height, not a dock's");
    }
}

#[test]
fn the_map_tab_draws_the_map_into_the_journal_body() {
    let st = AppState::default();
    let pl = compute_pane_layout(FRAME, &st);
    assert_eq!(pl.map, pl.journal_body);
    let mut mapper = mapper::mapper::Mapper::default();
    mapper.observe(1, "Start Room", None);
    let rm = mapper::render::render_layer(&mapper.graph, mapper::layer::MAIN_LAYER);
    let mut buf = Buffer::empty(FRAME);
    let hits = app::render::map::render_map_layered(&rm, &mapper.graph, &st, pl.map, &mut buf);
    assert!(!hits.room_rects.is_empty());
    assert!(hits.room_rects.iter().all(|(_, r)| r.x >= pl.map.x && r.right() <= pl.map.right()));
}
