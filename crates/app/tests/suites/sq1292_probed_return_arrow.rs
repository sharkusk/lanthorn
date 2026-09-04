//! SQ-1292: the way back is drawn CONSISTENTLY, whichever direction it turns out to be.
//!
//! Reported against Zork I (`stories/zork1-invclues-r52-s871125.z5`, the release the player's
//! `default.lanthorn` save was made from): walking around the house, the return-path probe's
//! arrows "are not drawn until the direction is explored", while a diagonal (the Chasm's
//! `northeast`, answered by `southwest`) or a staircase drew its way back the moment the player
//! arrived. The player's decision was that drawing it always is fine — it just has to be the
//! same rule in every direction.
//!
//! # The drawn map was never the problem
//!
//! Nothing in the render reads `Room::tried` or `Room::probed` — grep them and there are no
//! hits under `crates/app/src/render/`. `the_far_end_glyph_appears_the_moment_the_return_is_recorded`
//! below pins that across all six shapes a return can take (a cardinal that reciprocates, a
//! cardinal that does not, a diagonal, Up/Down in both geometries, In/Out): before the return
//! edge exists the far room's border is bare, and the instant it is recorded the glyph is there.
//!
//! # What WAS direction-shaped: which returns the search is still allowed to ask for
//!
//! [`mapper::graph::MapGraph::probe_candidates`] never offers a direction already in the room's
//! `probed` record, and `return_probe::deliver` used to write that record for EVERY attempt —
//! including one whose landing the map could not name. That mark is permanent, so a direction
//! probed on a first visit, when the room beyond was still unknown, could never be asked again
//! once it WAS known. The way back then waited for the player to walk it.
//!
//! And it falls unevenly across the compass, which is exactly how it was seen. A search that
//! does not answer at once burns the cardinals first — the seed `opposite(moved)`, the two
//! perpendiculars, then the head of [`mapper::direction::PROBE_FALLBACK_DIRS`] — reaches the
//! diagonals only if it gets that far, and since SQ-1290 took portals out of that fallback list
//! never touches Up/Down/In/Out at all. So a staircase's way back was found every time, a
//! diagonal's usually, and a compass exit's not until the player walked it.
//!
//! `an_attempt_the_map_could_not_read_is_asked_again_on_a_later_visit` drives the real story
//! through the exact route that produced it and asserts the arrowhead on the drawn map.

use std::path::PathBuf;
use std::sync::Arc;

use app::engine::Engine;
use app::probe::ShadowRecipe;
use app::render::map::render_map;
use app::state::AppState;

use mapper::direction::Direction;
use mapper::graph::{MapGraph, RoomId};
use mapper::mapper::{Mapper, ProbedPassage};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

// ── Zork I room numbers, from the reported save's `map.json` ─────────────────
const WEST_OF_HOUSE: RoomId = 68;
const NORTH_OF_HOUSE: RoomId = 143;
const BEHIND_HOUSE: RoomId = 89;
const SOUTH_OF_HOUSE: RoomId = 217;

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

/// Drive the real story the way the turn path does — the game's reply, then `apply_turn`, then
/// the probe armed against the crossing it just made. The app's own calls, not a re-implementation.
struct Play {
    state: AppState,
    mapper: Mapper,
    session: Box<dyn Engine>,
    death: app::session::DeathWatch,
}

impl Play {
    /// The InvisiClues release the defect was reported on — a **v5**, so `detect_location` reads
    /// the STATUS LINE rather than global 0, which is what the shadow's room identity depends on.
    /// It opens on a title card, so the drive answers whichever input the game is waiting on.
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

    fn has_edge(&self, from: RoomId, dir: Direction, to: RoomId) -> bool {
        self.mapper
            .graph
            .connections()
            .iter()
            .any(|c| c.origin == from && c.dir == dir && c.dest == to)
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────

/// The drawn map as rows of cell symbols, exactly as `drawn_edge_honesty` reads it.
fn render(g: &MapGraph) -> Vec<String> {
    let rm = mapper::render::render(g);
    let mut st = AppState::default();
    st.scroll = rm.bounds.0;
    // Row 3 of a room box is its `#id`, and it is drawn only with this on — which is how
    // `box_of` below finds a specific room's border rather than guessing from the label.
    st.show_room_numbers = true;
    let area = Rect::new(0, 0, 100, 40);
    let mut buf = Buffer::empty(area);
    render_map(&rm, &st, area, &mut buf);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" ").to_string())
                .collect::<String>()
        })
        .collect()
}

/// The rows of the box whose id line reads `#<id>`, plus the columns it spans — enough to ask
/// "is this glyph on THIS room's border" rather than "is it anywhere on the map".
fn box_of(rows: &[String], id: RoomId) -> (usize, usize, usize, usize) {
    // Every index here is a CELL index, so the rows are walked as chars: a box side is `│`,
    // three bytes wide and one cell, and mixing the two puts the borders in the wrong place.
    let tag: Vec<char> = format!("#{id}").chars().collect();
    let grid: Vec<Vec<char>> = rows.iter().map(|r| r.chars().collect()).collect();
    let (r, c) = grid
        .iter()
        .enumerate()
        .find_map(|(y, row)| {
            row.windows(tag.len()).position(|w| w == tag.as_slice()).map(|x| (y, x))
        })
        .unwrap_or_else(|| panic!("room #{id} is not on the drawn map:\n{}", rows.join("\n")));
    // A room box is five rows tall and the `#id` line is its fourth, so the top border is three
    // rows up and the bottom one row down. The sides are found by walking out along the id row
    // rather than by assuming a width.
    let side = |ch: &char| "│┃║◀▶┌└├┐┘┤".contains(*ch);
    let row = &grid[r];
    let left = (0..c).rev().find(|&x| side(&row[x])).unwrap_or(c);
    let right = (c..row.len()).find(|&x| side(&row[x])).unwrap_or(c);
    (r.saturating_sub(3), r + 1, left, right)
}

/// True when `glyph` appears on room `id`'s own box — its border, or the inner column beside it
/// where an In/Out badge sits (a portal with no compass side is anchored on the room rather than
/// on a border cell). Scoped to the box either way, so a connector passing nearby cannot answer
/// for it.
fn glyph_on_box(rows: &[String], id: RoomId, glyph: char) -> bool {
    let (top, bottom, left, right) = box_of(rows, id);
    (top..=bottom.min(rows.len().saturating_sub(1))).any(|y| {
        let row: Vec<char> = rows[y].chars().collect();
        (left..=right.min(row.len().saturating_sub(1))).any(|x| row[x] == glyph)
    })
}

// ── The drawn map treats every direction alike ──────────────────────────────

/// Six shapes a way back can take, and the far end of every one of them gains its glyph on the
/// turn the return edge is recorded — not before it, and not only after the player walks it.
///
/// This is the invariant the report doubted, pinned directly: the render has no `tried`/`probed`
/// reading anywhere, so a probed passage and a walked one are the same passage (SQ-0785's whole
/// design), and the *class* of the direction changes nothing.
#[test]
fn the_far_end_glyph_appears_the_moment_the_return_is_recorded() {
    /// One shape a way back can take: the player walked `out` of A into B, which sits at `b_at`
    /// relative to A, and the search answers with `back` — whose glyph B must then show.
    struct Shape {
        label: &'static str,
        out: Direction,
        b_at: (i32, i32),
        back: Direction,
        glyph: char,
    }
    let sym = AppState::default().symbols;
    let shape = |label, out, b_at, back, glyph| Shape { label, out, b_at, back, glyph };
    let cases = [
        shape("cardinal, reciprocating", Direction::E, (1, 0), Direction::W, sym.arrows.west),
        shape("cardinal, NOT reciprocating", Direction::N, (0, -1), Direction::W, sym.arrows.west),
        shape("diagonal", Direction::NE, (1, -1), Direction::SW, '↙'),
        shape("staircase down-right", Direction::Down, (1, 1), Direction::Up, sym.portal.up),
        shape("staircase up-left", Direction::Down, (-1, -1), Direction::Up, sym.portal.up),
        shape("doorway", Direction::In, (1, 0), Direction::Out, sym.portal.out),
    ];
    for Shape { label, out, b_at, back, glyph } in cases {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, b_at);
        g.add_edge(1, out, 2);
        g.set_current(2);

        let before = render(&g);
        assert!(
            !glyph_on_box(&before, 2, glyph),
            "{label}: B's box must be BARE while the way back is unknown — a one-way passage \
             shows only its departure (SQ-0688):\n{}",
            before.join("\n")
        );

        // Exactly what `Mapper::record_probed_passage` mints, and indistinguishable from the
        // edge a walked crossing would leave.
        g.add_edge(2, back, 1);
        let after = render(&g);
        assert!(
            glyph_on_box(&after, 2, glyph),
            "{label}: the way back is recorded, so B's own {back:?} exit is drawn on its border \
             at once:\n{}",
            after.join("\n")
        );
    }
}

// ── The record that decides which returns can still be asked for ────────────

/// A probe whose landing the map could not name leaves NOTHING behind — the attempt included
/// (SQ-1292). One that named a room, or named none at all (a refusal), is spent for good.
///
/// The distinction is what `probed` is FOR: `probe_candidates` never offers a direction it
/// holds, so a mark written here is permanent and has to state a fact about the world.
/// "Wherever that goes, the player has not been there yet" is not one — it stops being true
/// the moment they walk in.
#[test]
fn only_an_answered_attempt_spends_a_direction() {
    // Two rooms the map holds and one it does not.
    let mut m = Mapper::default();
    m.observe(1, "A", None);
    m.observe(2, "B", Some(Direction::N));

    // Landing in a room the map holds records the passage AND spends the direction.
    assert!(m.record_probed_passage(ProbedPassage { from: 2, dir: Direction::W, to: 1 }));
    m.graph.mark_probed(2, Direction::W);
    assert!(m.graph.is_probed(2, Direction::W));
    assert!(
        !m.graph.probe_candidates(2, Some(Direction::E)).contains(&Direction::W),
        "an answered direction is never offered again"
    );

    // Landing in a room the map does NOT hold records nothing at all — so the direction is
    // still on offer, and a later visit (with that room now known) can ask it.
    assert!(
        !m.record_probed_passage(ProbedPassage { from: 2, dir: Direction::S, to: 99 }),
        "the no-leak rule: an unvisited room never arrives through a probe"
    );
    assert!(!m.graph.is_probed(2, Direction::S), "and the ATTEMPT is not spent either");
    assert!(
        m.graph.probe_candidates(2, Some(Direction::N)).contains(&Direction::S),
        "so the way back can still be asked for once the map has grown"
    );
}

// ── The reported route, on the real story ───────────────────────────────────

/// Zork I, the reported route: the way back from Behind House to South of House is found and
/// DRAWN on the turn the player arrives, on a visit where the probe had already tried that very
/// direction — against a room the map did not hold yet.
///
/// The route, five turns from `look` (fixture: `zork1-invclues-r52-s871125.z5`, release 52,
/// serial 871125):
///
/// | turn | crossing | what the search does |
/// |---|---|---|
/// | 1 | `north`, West of House → North of House | `south` is boarded; `east` reaches Behind House, **unknown**; `west` answers |
/// | 2 | `east`, North of House → Behind House | `west` is a closed window; **`south` reaches South of House, unknown**; `north` answers |
/// | 3 | `north`, Behind House → North of House | the map already knows the way back — nothing armed |
/// | 4 | `west`, North of House → West of House | likewise |
/// | 5 | `south`, West of House → South of House | `north` is boarded; `west` answers |
/// | 6 | `east`, South of House → Behind House | the way back is `south`, tried once on turn 2 |
///
/// Turn 2's `south` is the whole case. It landed in a room that was not on the map, so it said
/// nothing — yet it used to be written to Behind House's `probed` record all the same, and
/// `probe_candidates` never offers a direction it holds. On turn 6 the way back was therefore
/// unreachable, the search ran out of candidates, and Behind House's south border stayed bare
/// until the player walked it themselves. That is the reported symptom, in a compass direction,
/// with the staircases two rooms away answering on their first attempt every time.
#[test]
fn an_attempt_the_map_could_not_read_is_asked_again_on_a_later_visit() {
    let Some(mut p) = Play::zork1_z5() else { return };
    assert_eq!(p.mapper.graph.current(), Some(WEST_OF_HOUSE), "the map starts West of House");

    for cmd in ["north", "east", "north", "west", "south"] {
        p.turn(cmd);
    }

    // Non-vacuity: the route really did reach all four rooms, and turn 2's search really did
    // spend an attempt on `south` out of Behind House. If either stops being true the case
    // below is measuring something else.
    for (id, name) in [
        (WEST_OF_HOUSE, "West of House"),
        (NORTH_OF_HOUSE, "North of House"),
        (BEHIND_HOUSE, "Behind House"),
        (SOUTH_OF_HOUSE, "South of House"),
    ] {
        assert!(p.mapper.graph.room(id).is_some(), "the route reached {name} (#{id})");
    }
    assert_eq!(p.mapper.graph.current(), Some(SOUTH_OF_HOUSE), "and ends South of House");
    assert!(
        !p.has_edge(BEHIND_HOUSE, Direction::S, SOUTH_OF_HOUSE),
        "nobody has walked or found Behind House → south → South of House yet"
    );

    let before = render(&p.mapper.graph);
    assert!(
        !glyph_on_box(&before, BEHIND_HOUSE, AppState::default().symbols.arrows.south),
        "so Behind House's south border is bare:\n{}",
        before.join("\n")
    );

    // Turn 6: east into Behind House. The way back is `south` — the direction turn 2 asked about
    // and could not read.
    p.turn("east");
    assert_eq!(p.mapper.graph.current(), Some(BEHIND_HOUSE), "the player is Behind House");
    assert!(
        p.has_edge(BEHIND_HOUSE, Direction::S, SOUTH_OF_HOUSE),
        "the search finds the way back on the turn the player arrives, not when they walk it: \
         {:?}",
        p.mapper.graph.connections()
    );

    let after = render(&p.mapper.graph);
    assert!(
        glyph_on_box(&after, BEHIND_HOUSE, AppState::default().symbols.arrows.south),
        "and it is DRAWN — Behind House's own south exit, on its own border:\n{}",
        after.join("\n")
    );
}
