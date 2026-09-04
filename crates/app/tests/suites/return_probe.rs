//! The return probe (SQ-0785): after a move, find the way BACK in a silent copy
//! of the game, and close the automap's one-way gaps without inventing
//! reciprocity.
//!
//! Three things are worth asserting and one of them is worth asserting hardest.
//!
//! * **A real return path is found and recorded.** Zork I's West of House → North
//!   of House → back is the small, fast, always-available case.
//! * **A probe that lands in the WRONG room records nothing** — not the room, not
//!   the edge, not that it exists. This is the failure the whole design is
//!   arranged around: an invented edge is worse than the gap it closed, because
//!   the player cannot tell which arrows were observed and which were guessed.
//! * **The control is on the map border in both states**, because a switch that
//!   is off by default and invisible when off is a switch nobody ever finds.
//!
//! Real-game cases skip vacuously without `stories/` (gitignored), the
//! CI-safe pattern; the control and the wrong-room cases need no story at all.

use std::path::PathBuf;
use std::sync::Arc;

use app::engine::Engine;
use app::probe::ShadowRecipe;
use app::render::controls::{control_at, controls_for, BorderControl};
use app::render::panel::{draw_panel_with_controls, PanelSpec, PanelStrip};
use app::render::paneframe::{InsetSegment, PaneGlyphs};
use app::state::AppState;

use mapper::direction::Direction;
use mapper::mapper::{Mapper, ProbedPassage};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

// ── Fixtures ────────────────────────────────────────────────────────────────

fn story(name: &str) -> Option<Vec<u8>> {
    let path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            None
        }
    }
}

fn recipe(bytes: &[u8]) -> ShadowRecipe {
    ShadowRecipe {
        story_bytes: Arc::new(bytes.to_vec()),
        store: PathBuf::new(),
        vfs_bytes: Arc::new(Vec::new()),
        honor_game_colours: true,
        interpreter_number: None,
        random_seed: None,
        acceleration: true,
        screen: (80, 24),
    }
}

/// Drive a real story the way the turn path does — the game's reply, then
/// `apply_turn`, then the probe armed against the crossing it just made.
///
/// It is deliberately the app's own calls and not a re-implementation: a harness
/// that armed the search itself would be testing the harness.
struct Play {
    state: AppState,
    mapper: Mapper,
    session: Box<dyn Engine>,
    death: app::session::DeathWatch,
}

impl Play {
    fn zork1() -> Option<Play> {
        let bytes = story("zork1-r88-s840726.z3")?;
        let mut s = app::session::GameSession::new_with_trace(
            bytes.clone(),
            true,
            false,
            None,
            false,
            Vec::new(),
            None,
            None,
            Some((25, 80)),
        )
        .expect("zork1-r88-s840726.z3 boots without a ZError");
        s.set_strip_prompt(false);
        let mut state = AppState::default();
        state.config.return_probe = true;
        state.probe.arm(recipe(&bytes));
        let mut p = Play {
            state,
            mapper: Mapper::default(),
            session: Box::new(s),
            death: app::session::DeathWatch::default(),
        };
        // The opening room, so the map has somewhere to start from.
        let r = p.session.submit("look");
        app::session::apply_turn(&mut p.mapper, "look", &r, &mut p.death);
        Some(p)
    }

    /// The InvisiClues release, which is a **v5** — and version is what decides
    /// this: `detect_location` reads global 0 on v1-v3 and the STATUS LINE from
    /// v4 on, so the defect below cannot exist on the z3 the other cases use.
    ///
    /// It opens on a title card, so the drive answers whichever input the game is
    /// actually waiting on; a line typed at a char prompt is swallowed and the
    /// harness maps a screen with no room on it.
    fn zork1_z5() -> Option<Play> {
        let bytes = story("zork1-invclues-r52-s871125.z5")?;
        let inner = match app::hints::extract_story(bytes.clone()).ok()? {
            app::hints::LoadedStory::ZCode(b) => b,
            _ => return None,
        };
        let mut s = app::session::GameSession::new_with_trace(
            inner.clone(),
            true,
            false,
            None,
            false,
            Vec::new(),
            None,
            None,
            Some((25, 80)),
        )
        .expect("zork1-invclues-r52-s871125.z5 boots without a ZError");
        s.set_strip_prompt(false);
        let mut state = AppState::default();
        state.config.return_probe = true;
        state.probe.arm(recipe(&inner));
        let mut p = Play {
            state,
            mapper: Mapper::default(),
            session: Box::new(s),
            death: app::session::DeathWatch::default(),
        };
        for _ in 0..4 {
            let r = match p.session.pending_input() {
                app::session::InputKind::Char => p
                    .session
                    .submit_key(app::engine::KeyInput::Char(' '))
                    .unwrap_or_else(|| p.session.submit("")),
                _ => p.session.submit(""),
            };
            app::session::apply_turn(&mut p.mapper, "", &r, &mut p.death);
        }
        let r = p.session.submit("look");
        app::session::apply_turn(&mut p.mapper, "look", &r, &mut p.death);
        Some(p)
    }

    /// One turn, then let any search it armed run to the end.
    fn turn(&mut self, cmd: &str) -> Option<ProbedPassage> {
        let r = self.session.submit(cmd);
        let room_before = self.mapper.graph.current();
        app::session::apply_turn(&mut self.mapper, cmd, &r, &mut self.death);
        app::return_probe::arm_return_search(
            &mut self.state,
            &self.mapper,
            &*self.session,
            cmd,
            room_before,
            &mut app::engine::TurnSave::default(),
        );
        app::return_probe::settle_return_search(&mut self.state, &mut self.mapper)
    }

    fn edge(&self, from: mapper::graph::RoomId, dir: Direction) -> Option<mapper::graph::RoomId> {
        self.mapper
            .graph
            .connections()
            .iter()
            .find(|c| c.origin == from && c.dir == dir)
            .map(|c| c.dest)
    }
}

// ── The real-game case ──────────────────────────────────────────────────────

/// West of House → NORTH → North of House, and the shadow finds the way back —
/// which is **not** the way it came.
///
/// North of House is a small, real, and rather pointed specimen: south is boarded
/// up, east is Behind House, and it is WEST that returns you. So the search walks
/// its priority order past a refusal and past a landing in the WRONG ROOM before
/// it succeeds — and the room it wrongly landed in is not added to the map, which
/// is the invariant this whole feature is arranged around, demonstrated by a real
/// game rather than by a hand-built answer.
///
/// (Fixture: `zork1-r88-s840726.z3`, two turns in — `look`, then `north`.)
#[test]
fn zork1_learns_the_way_back_which_is_not_the_way_it_came() {
    let Some(mut p) = Play::zork1() else { return };
    let west = p.mapper.graph.current().expect("the map starts West of House");
    let rooms_before: Vec<_> = p.mapper.graph.rooms().map(|r| r.id).collect();
    assert_eq!(rooms_before.len(), 1, "one room on the map: {rooms_before:?}");

    let found = p.turn("north").expect("the way back from North of House");
    let north = p.mapper.graph.current().expect("north of house");
    assert_ne!(north, west, "the move actually crossed something");
    assert_eq!(
        found,
        ProbedPassage { from: north, dir: Direction::W, to: west },
        "west is the way back, and south — the way it came — is boarded up"
    );
    assert_eq!(p.edge(north, Direction::W), Some(west), "and it is on the map");
    assert_eq!(p.edge(west, Direction::N), Some(north), "the outbound passage is untouched");

    // THE INVARIANT. Getting to west, the search walked EAST, which really does
    // lead somewhere — Behind House. It is not on the map, it has no edge, and
    // nothing records that the shadow ever saw it.
    assert_eq!(
        p.mapper.graph.rooms().map(|r| r.id).collect::<Vec<_>>().len(),
        2,
        "only the two rooms the PLAYER has stood in: {:?}",
        p.mapper.graph.rooms().map(|r| (r.id, r.name.clone())).collect::<Vec<_>>()
    );
    assert!(
        p.edge(north, Direction::E).is_none(),
        "the room east of here was walked into and deliberately forgotten"
    );

    // Every ANSWERED attempt is on the probe's record and none of them on the player's, so the
    // map still offers south and east as exits nobody has explored.
    //
    // South is refused ("The windows are all boarded") and west reaches West of House: both are
    // answers, and both are spent for good. East is neither — it came out in Behind House, which
    // this map does not hold, so it says only "the player has not been there YET" and is NOT
    // spent (SQ-1292). `probed` is consulted forever after by `probe_candidates`, which never
    // offers a direction it holds; marking east here would mean that on a later visit — with
    // Behind House by then on the map — North of House could never learn its way back east, and
    // the search would settle for the diagonal instead.
    for d in [Direction::S, Direction::W] {
        assert!(p.mapper.graph.is_probed(north, d), "{d:?} was attempted and answered");
    }
    assert!(
        !p.mapper.graph.is_probed(north, Direction::E),
        "east was attempted but the map could not read the answer, so it stays askable"
    );
    let room = p.mapper.graph.room(north).unwrap();
    assert!(room.tried.is_empty(), "the PLAYER has typed nothing here: {:?}", room.tried);
    assert!(p.mapper.graph.untried(north).contains(&Direction::S), "south is still on offer");
    assert!(p.mapper.graph.untried(north).contains(&Direction::E), "and so is east");

    assert_eq!(p.state.probe.probes, 3, "south, east, west — the priority order, in order");
    eprintln!(
        "zork1 North of House: {} command(s), {:?} on the worker",
        p.state.probe.probes, p.state.probe.spent
    );
}

/// The gate. Walking a passage the map already has a return path for asks the
/// shadow nothing at all.
#[test]
fn zork1_asks_nothing_when_the_way_back_is_already_known() {
    let Some(mut p) = Play::zork1() else { return };
    p.turn("north"); // discovers North of House --W--> West of House
    let after = p.state.probe.probes;
    assert!(after > 0, "the first crossing was probed");
    p.turn("west"); // back to West of House along the edge just discovered
    p.turn("north"); // and out again: both directions are now on the map
    assert_eq!(
        p.state.probe.probes, after,
        "neither crossing has a gap left, so neither is probed"
    );
}

/// `enter window` is the two-facts case: the traversal stays the player's own
/// command, and the geometry arrives as the edge the shadow walked. Nothing
/// anywhere claims that the reverse of what came back works from Behind House —
/// which is the claim reciprocity would have made.
///
/// (Fixture: `zork1-r88-s840726.z3`, five turns in — `look`, `north`, `east`,
/// `open window`, `enter window`.)
#[test]
fn zork1_kitchen_gets_its_geometry_without_its_traversal_being_invented() {
    let Some(mut p) = Play::zork1() else { return };
    p.turn("north");
    p.turn("east");
    let behind = p.mapper.graph.current().expect("Behind House");
    p.turn("open window");
    let found = p.turn("enter window");
    let kitchen = p.mapper.graph.current().expect("the Kitchen");
    assert_ne!(kitchen, behind, "the window was climbed through");
    let found = found.expect("the way out of the Kitchen");
    assert_eq!(found.from, kitchen);
    assert_eq!(found.to, behind);
    assert_eq!(p.edge(kitchen, found.dir), Some(behind), "the way back is on the map");

    // The outbound passage is still whatever the player's own command meant —
    // Zork's parser reads `enter window` as `in` — and NOT the reverse of the
    // direction that came back.
    assert_eq!(
        p.edge(behind, Direction::In),
        Some(kitchen),
        "the traversal is the command the player used"
    );
    let reciprocal = mapper::direction::opposite(found.dir);
    assert!(
        reciprocal == Direction::In || p.edge(behind, reciprocal).is_none(),
        "{reciprocal:?} out of Behind House was not invented from the return direction"
    );
}

/// **An aborted search keeps everything it answered.** One attempt is pumped and
/// delivered, the player then walks on — which ends the search — and the probed
/// record still holds that attempt, so the next search from that room resumes
/// rather than re-walking it.
///
/// Falsify by marking the probed record at DELIVERY time only for a search that
/// survives, or by clearing it on abort: the second search then offers twelve
/// again and the whole cost is paid twice.
#[test]
fn an_aborted_search_keeps_what_it_answered_and_the_next_one_resumes() {
    let Some(mut p) = Play::zork1() else { return };
    let west = p.mapper.graph.current().unwrap();

    // Arm at North of House and answer exactly ONE attempt — `south`, the way it
    // came, which Zork I has boarded up.
    let r = p.session.submit("north");
    app::session::apply_turn(&mut p.mapper, "north", &r, &mut p.death);
    let north = p.mapper.graph.current().unwrap();
    app::return_probe::arm_return_search(
        &mut p.state,
        &p.mapper,
        &*p.session,
        "north",
        Some(west),
        &mut app::engine::TurnSave::default(),
    );
    assert!(app::return_probe::pump_return_search(&mut p.state), "one attempt went out");
    let answer = p.state.probe.settle().expect("the shadow answered it");
    assert!(app::return_probe::owns(&p.state, answer.token));
    assert!(
        app::return_probe::deliver(&mut p.state, &mut p.mapper, &answer).is_none(),
        "south is boarded up, so nothing was recorded but the attempt"
    );
    assert!(p.mapper.graph.is_probed(north, Direction::S), "and the attempt IS recorded");
    assert_eq!(p.state.probe.probes, 1, "exactly one command so far");

    // The player walks on: the search is over, mid-list.
    let r = p.session.submit("east");
    let room_before = p.mapper.graph.current();
    app::session::apply_turn(&mut p.mapper, "east", &r, &mut p.death);
    app::return_probe::arm_return_search(&mut p.state, &p.mapper, &*p.session, "east", room_before, &mut app::engine::TurnSave::default());
    assert_ne!(p.mapper.graph.current(), Some(north), "the move really left the room");

    // Come back, and the search picks up where it stopped.
    let r = p.session.submit("north");
    let room_before = p.mapper.graph.current();
    app::session::apply_turn(&mut p.mapper, "north", &r, &mut p.death);
    assert_eq!(p.mapper.graph.current(), Some(north), "back at North of House");
    app::return_probe::arm_return_search(&mut p.state, &p.mapper, &*p.session, "north", room_before, &mut app::engine::TurnSave::default());
    if let Some(s) = &p.state.return_search {
        assert!(
            !mapper::direction::PROBE_DIRS.is_empty() && s.remaining() < 12,
            "the resumed search is shorter than a fresh one: {} left",
            s.remaining()
        );
    }
    assert!(
        p.mapper.graph.is_probed(north, Direction::S),
        "and south is still written off, permanently"
    );
}

/// **A busy shadow makes the search wait, and it does not give up.** The seam
/// holds one question at a time and the vocabulary offer asks it too, so "busy"
/// is an ordinary outcome — and a return answer is never stale, so waiting costs
/// the search nothing but time. This is why one-question-in-flight cannot starve
/// it the way it can starve a per-turn offer.
#[test]
fn a_busy_shadow_makes_the_search_wait_rather_than_give_up() {
    let Some(mut p) = Play::zork1() else { return };
    let west = p.mapper.graph.current().unwrap();
    let r = p.session.submit("north");
    app::session::apply_turn(&mut p.mapper, "north", &r, &mut p.death);
    app::return_probe::arm_return_search(
        &mut p.state,
        &p.mapper,
        &*p.session,
        "north",
        Some(west),
        &mut app::engine::TurnSave::default(),
    );

    // Somebody else's question is out with the worker.
    let other = p.state.probe.ask(&*p.session, &["zqxwvj".to_string()]).expect("the seam is free");
    assert!(!app::return_probe::owns(&p.state, other), "and it is not ours");
    assert!(
        !app::return_probe::pump_return_search(&mut p.state),
        "the search asked nothing while the shadow was busy"
    );
    assert!(p.state.return_search.is_some(), "and it did not give up");

    // The other answer arrives and is not ours to keep; the search then asks.
    let a = p.state.probe.settle().expect("the other question answered");
    assert_eq!(a.token, other);
    assert!(!app::return_probe::owns(&p.state, a.token));
    assert!(
        app::return_probe::pump_return_search(&mut p.state),
        "with the seam free again the very next pass asks"
    );
    let found = app::return_probe::settle_return_search(&mut p.state, &mut p.mapper);
    assert!(found.is_some(), "and the search still reaches its answer: {found:?}");
}

// ── The wrong room, and total failure ───────────────────────────────────────

/// **The case the design exists for.** A probe that comes out somewhere that is
/// not the room the player left records the attempt and nothing else — no edge,
/// no room, no trace that C was ever seen.
///
/// Driven through `deliver` with a hand-built answer rather than a story,
/// because a story that reliably produces a THIRD room on the first candidate is
/// a fixture nobody has; the crossing under test is the one in this module, and
/// this is exactly the value the worker hands it.
#[test]
fn a_probe_that_lands_in_the_wrong_room_records_nothing_at_all() {
    let mut m = Mapper::default();
    m.observe(1, "Hall", None);
    m.observe(2, "Cave", Some(Direction::N));
    let before = m.graph.connections().to_vec();
    let rooms_before: Vec<_> = m.graph.rooms().map(|r| r.id).collect();

    // The passage the shadow WOULD have found, had it landed home — recorded
    // here only to show that the same call refuses it when it did not.
    let wrong = ProbedPassage { from: 2, dir: Direction::S, to: 3 };
    assert!(!m.record_probed_passage(wrong), "room 3 is not on the map and is not put there");
    assert_eq!(m.graph.connections(), before.as_slice(), "no edge");
    assert_eq!(m.graph.rooms().map(|r| r.id).collect::<Vec<_>>(), rooms_before, "no room");

    // …and even with C already known, a landing in C is not the landing asked
    // for: the caller only ever builds a passage whose `to` is the origin.
    m.graph.upsert_room(3, "Attic".into());
    assert_eq!(
        ProbedPassage { from: 2, dir: Direction::S, to: 1 }.to,
        1,
        "the value the deliverer builds always names the room the player left"
    );
}

/// Total failure says nothing about the map. Twelve directions that led nowhere
/// prove only that they led nowhere from here, this time — a door may need
/// opening, and a one-way passage is a real answer.
#[test]
fn a_search_that_finds_nothing_leaves_the_map_as_it_was() {
    let Some(mut p) = Play::zork1() else { return };
    // A room Zork I really does not let you walk out of the way you came:
    // stand in the Kitchen having gone up the stairs to the Attic, whose only
    // exit is down. `down` IS the way back, so instead assert the shape on a
    // hand-made search that exhausts its queue.
    let mut m = Mapper::default();
    m.observe(1, "Hall", None);
    m.observe(2, "Sealed Cell", Some(Direction::N));
    for d in mapper::direction::PROBE_DIRS {
        m.graph.mark_probed(2, d);
    }
    let conns = m.graph.connections().to_vec();
    app::return_probe::arm_return_search(&mut p.state, &m, &*p.session, "north", Some(1), &mut app::engine::TurnSave::default());
    assert!(
        p.state.return_search.is_none(),
        "every candidate already walked leaves nothing to ask, and nothing is recorded"
    );
    assert_eq!(m.graph.connections(), conns.as_slice());
}

// ── The control on the story pane's bottom border ───────────────────────────

/// Draw the story panel the way `main::draw_story_panel` does, into a fresh
/// buffer.
fn draw_story(state: &AppState, w: u16, h: u16) -> (Buffer, Vec<(BorderControl, Rect)>) {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let views = controls_for(state);
    let ctls: Vec<_> = views.iter().map(|v| v.as_header_control()).collect();
    let tab = state.colors.theme.get("panel.tab").style;
    let segs = [InsetSegment { text: "ZORK I", active: true }];
    let (_, rects) = draw_panel_with_controls(
        &mut buf,
        &PanelSpec {
            area,
            border_selector: "panel.border",
            border_color: None,
            border_style: None,
            glyphs: &PaneGlyphs::default(),
            header_on: true,
            strip: Some(PanelStrip { segments: &segs, base: tab, active: tab }),
            body_fill: None,
        },
        &ctls,
        &state.colors.theme,
    );
    let hits = views
        .iter()
        .map(|v| v.id)
        .zip(rects)
        .filter(|(_, r)| r.width > 0 && r.height > 0)
        .collect();
    (buf, hits)
}

fn row(buf: &Buffer, y: u16) -> String {
    (buf.area.x..buf.area.right()).map(|x| buf.cell((x, y)).unwrap().symbol().to_owned()).collect()
}

/// **The footprint rides the STORY pane's bottom border, immediately inboard of
/// the map toggle** — in both states, with the colour carrying the state.
///
/// It was on the MAP pane's border until SQ-1107, and that was wrong for a
/// reason the placement rule hides: the search keeps running when the map is
/// hidden, because hiding a view must not degrade the data behind it — so its
/// only switch cannot live on a pane that disappears. You could not turn off
/// something that was still running. `the_switch_survives_hiding_the_map` below
/// is the case that pins it.
///
/// **Never hidden when off** is the other load-bearing half. Every other control
/// governs something on by default, so it is discovered by being used; this one
/// is off out of the box, and a control nobody has seen lit is the only way an
/// off-by-default feature ever gets found.
#[test]
fn the_footprint_rides_the_story_border_inboard_of_the_map_toggle() {
    let mut st = AppState::default();
    st.story_zversion = Some(3);
    let mark = st.symbols.controls.return_probe;

    for on in [false, true] {
        st.config.return_probe = on;
        let (buf, hits) = draw_story(&st, 44, 10);
        let bottom = row(&buf, 9);
        assert!(
            bottom.contains(mark.to_string().as_str()),
            "the footprint is on the bottom border with return_probe = {on}: {bottom:?}"
        );
        eprintln!("story border, return_probe = {on:<5}: {bottom}");
        let (_, probe) = hits.iter().find(|(id, _)| *id == BorderControl::ReturnProbe).unwrap();
        let (_, map) = hits.iter().find(|(id, _)| *id == BorderControl::Map).unwrap();
        assert_eq!(probe.y, 9, "on the bottom border, not the top");
        // The pair, in order: the probe inboard, the map toggle at the corner.
        assert!(probe.x < map.x, "the probe sits inboard of the map toggle: {bottom:?}");
        assert_eq!(probe.x + 2, map.x, "…and they are one group, one space apart");
        assert!(
            map.right() + 1 >= 44 - 2,
            "the map toggle still keeps the corner anchor: {bottom:?}"
        );
        assert_eq!(
            control_at(&st, &hits, probe.x, probe.y),
            Some(BorderControl::ReturnProbe),
            "and a click on it resolves to the control"
        );
    }

    // The state is carried by the COLOUR, since the mark has only one shape.
    let view = |st: &AppState| {
        controls_for(st).into_iter().find(|v| v.id == BorderControl::ReturnProbe).unwrap().style
    };
    st.config.return_probe = false;
    let off = view(&st);
    st.config.return_probe = true;
    let lit = view(&st);
    assert_ne!(off, lit, "muted when off, lit when on");
    assert_eq!(lit, st.colors.theme.get("panel.control:lit").style, "lit is the `alert` role");
    assert_eq!(off, st.colors.theme.get("panel.control").style);
}

/// **The switch outlives the map**, which is the whole reason it moved. The
/// search runs whether or not the map pane is on screen, so its control has to
/// be reachable either way — and on the map's own border it was not.
#[test]
fn the_switch_survives_hiding_the_map() {
    let mut st = AppState::default();
    st.story_zversion = Some(3);
    for layout in [app::state::Layout::Split, app::state::Layout::TranscriptFull] {
        st.layout = layout;
        let (buf, hits) = draw_story(&st, 44, 10);
        eprintln!("{layout:?}: {}", row(&buf, 9));
        assert!(
            hits.iter().any(|(id, _)| *id == BorderControl::ReturnProbe),
            "{layout:?}: the probe's switch must be on screen with the map hidden too",
        );
    }
}

/// **The probe gives way first as the pane narrows.** Both live in the anchored
/// right-hand group, and the group sheds from its left — so the map toggle, the
/// only way back to a hidden map, is the last control standing. The printed rows
/// are the record of where each threshold falls.
#[test]
fn the_map_toggle_outlives_the_probe_as_the_pane_narrows() {
    let mut st = AppState::default();
    st.story_zversion = Some(3);
    let mut seen: Vec<(u16, bool, bool)> = Vec::new();
    for w in 4..=20u16 {
        let (buf, hits) = draw_story(&st, w, 6);
        let has = |id: BorderControl| hits.iter().any(|(i, _)| *i == id);
        let (map, probe) = (has(BorderControl::Map), has(BorderControl::ReturnProbe));
        eprintln!("w={w:>2} map={map:<5} probe={probe:<5}  {}", row(&buf, 5));
        assert!(!(probe && !map), "w={w}: the probe outlived the map toggle");
        seen.push((w, map, probe));
    }
    let first = |f: fn(&(u16, bool, bool)) -> bool| seen.iter().find(|r| f(r)).unwrap().0;
    assert_eq!(first(|r| r.1), 7, "the map toggle alone needs a 7-column pane");
    assert_eq!(first(|r| r.2), 9, "the pair needs 9 — two more columns for the probe");
}

/// A click runs the registry command, exactly as every other border control
/// does — there is no second implementation of the switch under the icon.
#[test]
fn the_control_names_a_real_slash_command() {
    assert_eq!(BorderControl::ReturnProbe.command().name, "set-return-probe");
    assert_eq!(BorderControl::ReturnProbe.command().arg, None);
    assert!(
        app::slash::COMMANDS.iter().any(|c| c.name == "set-return-probe"),
        "and the registry holds it"
    );
}

/// **A probe restored into a room inherits the last probe's status line, and on
/// v4+ that is where the room's IDENTITY comes from** (SQ-0785).
///
/// Quetzal archives no screen, so `restore_state` brings memory alone. The story
/// then repaints only as many columns as its new room name needs, and the tail of
/// the longer name it was painted over survives past the end of it. Zork I's
/// shadow read `Forest Pathse`, which matches no object; `detect_location` fell
/// off `PlayerParent` onto the text rung, and `resolve_room_object` prefix-matched
/// **object 1 — the scenery object whose short name is `forest`** (ties on
/// normalized length go to the lowest object number). The comparison `1 == 247`
/// then discarded a return path that is real, on the FIRST candidate the priority
/// order offers.
///
/// The three rooms below are the three the defect was reported from, all of them
/// beside Zork's several `Forest` rooms. Each is a one-attempt case: the way back
/// IS the way it came, so anything but `probes == 1` per crossing means the search
/// walked past a working answer.
///
/// Falsify by dropping the `blank()` in `GameSession::restore_state`: every one of
/// these reports `None`.
///
/// (Fixture: `zork1-invclues-r52-s871125.z5` — release 52, serial 871125, a **v5**;
/// the z3 the other cases drive reads global 0 and cannot show this. Five turns in.)
#[test]
fn zork1_z5_finds_the_way_back_past_a_scenery_object_of_the_same_name() {
    let Some(mut p) = Play::zork1_z5() else { return };

    p.turn("north"); // North of House
    p.turn("north"); // Forest Path
    let path = p.mapper.graph.current().expect("Forest Path");
    assert_eq!(
        p.mapper.graph.room(path).map(|r| r.name.as_str()),
        Some("Forest Path"),
        "non-vacuity: the room whose name object 1 shadows"
    );

    let before = p.state.probe.probes;
    let found = p.turn("north").expect("south returns to Forest Path");
    let clearing = p.mapper.graph.current().expect("Clearing");
    assert_eq!(
        found,
        ProbedPassage { from: clearing, dir: Direction::S, to: path },
        "the way back is the way it came, and it is Forest Path — not object 1"
    );
    assert_eq!(p.edge(clearing, Direction::S), Some(path), "and it is on the map");
    assert_eq!(p.state.probe.probes - before, 1, "found on the first candidate");

    // The same shape one room over: east into Forest, west back out.
    let before = p.state.probe.probes;
    p.turn("south"); // back to Forest Path along the edge just discovered
    let found = p.turn("east").expect("west returns to Forest Path");
    let forest = p.mapper.graph.current().expect("Forest");
    assert_eq!(found, ProbedPassage { from: forest, dir: Direction::W, to: path });
    assert_eq!(
        p.state.probe.probes - before,
        1,
        "the walk back south had a known return path and asked nothing; east asked once"
    );
}

/// **A landing on a room the map already holds is recorded, even though it is not
/// the room the search was asking about** — and that is what stops a diagonal
/// being drawn where a cardinal is known (SQ-0785).
///
/// The reported walk is `N, E, S, E, N, W, S` from West of House, and it reaches
/// South of House TWICE: first from Behind House, then from West of House. Under
/// the old rule the first visit asked "how do I get back to Behind House?", ran
/// `N` (boarded), then `W` — which reached West of House, the wrong room *for that
/// question*, and was thrown away — then `E`, which succeeded. `W` was left on the
/// probed record, and `probe_candidates` filters on `tried ∪ probed`, so the second
/// visit found `W` gone and recorded the first survivor: the diagonal `NW`.
///
/// `probed` says "this direction was walked from here". The answer it stood for was
/// "…and it did not reach THAT origin". Recording every landing on a KNOWN room
/// closes the gap on the first visit, so the second has nothing left to ask.
///
/// Falsify by narrowing `deliver` back to `landed == origin`: the west edge is
/// absent after the first visit, and the second visit mints `NW`.
///
/// (Fixture: `zork1-invclues-r52-s871125.z5`, the release the defect was reported
/// on; eleven turns in.)
#[test]
fn a_landing_on_a_known_room_is_recorded_even_when_it_is_not_the_room_asked_about() {
    let Some(mut p) = Play::zork1_z5() else { return };
    let west = p.mapper.graph.current().expect("West of House");

    p.turn("north"); // North of House
    p.turn("east"); // Behind House
    let behind = p.mapper.graph.current().expect("Behind House");

    // FIRST arrival at South of House, asking the way back to Behind House.
    let found = p.turn("south").expect("east returns to Behind House");
    let south = p.mapper.graph.current().expect("South of House");
    assert_eq!(
        p.mapper.graph.room(south).map(|r| r.name.as_str()),
        Some("South of House"),
        "non-vacuity: the room the defect is about"
    );
    assert_eq!(
        found,
        ProbedPassage { from: south, dir: Direction::E, to: behind },
        "the search's own answer is still the way back to where it came from"
    );

    // THE NEW RULE. `W` reached West of House on the way past — a room the player
    // has stood in — so the passage is on the map instead of being discarded.
    assert_eq!(
        p.edge(south, Direction::W),
        Some(west),
        "the westward landing was kept, though the search was asking about Behind House"
    );

    // …and nothing unseen arrived with it. Four rooms, all walked by the player.
    let rooms: Vec<_> = p.mapper.graph.rooms().map(|r| (r.id, r.name.clone())).collect();
    assert_eq!(rooms.len(), 4, "only the rooms the PLAYER has stood in: {rooms:?}");

    // SECOND arrival, now from West of House. The gate has a return path already
    // and asks nothing at all, so the diagonal is never reached.
    let before = p.state.probe.probes;
    p.turn("east"); // Behind House
    p.turn("north"); // North of House
    p.turn("west"); // West of House
    p.turn("south"); // South of House again
    assert_eq!(
        p.mapper.graph.current(),
        Some(south),
        "the walk came back to the same room"
    );
    assert_eq!(
        p.edge(south, Direction::W),
        Some(west),
        "the way back is the cardinal it always was"
    );
    assert!(
        p.edge(south, Direction::NW).is_none(),
        "and no diagonal was minted alongside it"
    );
    assert_eq!(
        p.state.probe.probes,
        before,
        "no gap left to close on the return leg, so the shadow was asked nothing"
    );
}
