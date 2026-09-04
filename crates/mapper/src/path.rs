//! The shortest route the map already knows how to WALK, from one room to another (SQ-0693).
//!
//! Not to be confused with `router.rs`: that decides how to draw a connector line between two room
//! boxes. This answers the player's question — "I am here, that room is over there, which way do I
//! go?" — and its answer is a list of directions to type.
//!
//! Two rules make the answer honest, and both are deliberate:
//!
//! * **Directed.** An edge `origin —dir→ dest` is walkable from `origin` only. A one-way passage
//!   really is one-way, and a route that walked one backwards would be a route the player cannot
//!   follow. `Direction::Unknown` is skipped for the same reason [`crate::matrix::entrances`] skips
//!   it: `xyzzy` is a passage, but it is not a direction you can walk by typing a compass word.
//! * **Whole-graph.** Layers are a way of PRESENTING rooms, not a wall between them. A route that
//!   dips out of the maze and back in is still a route you can walk, so the search never looks at
//!   layers at all; deciding which of the steps it can draw is the caller's problem.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::direction::Direction;
use crate::graph::{MapGraph, RoomId};
use crate::matrix::MATRIX_DIRS;

/// One step of a route: stand in `room`, leave by `dir`, arrive at `dest`.
///
/// The `room`+`dir` pair is what the matrix view highlights — the cell you leave by — so a route
/// reads top to bottom as walking instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub room: RoomId,
    pub dir: Direction,
    pub dest: RoomId,
}

/// The shortest known walk from `from` to `to`, or `None` when the map knows no way at all.
///
/// `Some(vec![])` for `from == to`: you are already there, which is a different answer from "there
/// is no route", and the caller wants to tell them apart.
///
/// Breadth-first, so the route returned is always one of the shortest. Ties are broken by
/// [`MATRIX_DIRS`] column order and then by destination id, which makes the answer a pure function
/// of the graph rather than of the order the edges happened to be minted in — a route the player
/// was shown before a save must be the same route after a load.
pub fn route(graph: &MapGraph, from: RoomId, to: RoomId) -> Option<Vec<Step>> {
    if from == to {
        return Some(Vec::new());
    }
    let col = |d: Direction| MATRIX_DIRS.iter().position(|&x| x == d).unwrap_or(usize::MAX);

    // Outgoing edges only — this is the whole of the "directed" rule. Self-loops are dropped
    // because they can never advance a search, and duplicates because the graph legitimately keeps
    // more than one record of the same passage.
    let mut out: BTreeMap<RoomId, Vec<(Direction, RoomId)>> = BTreeMap::new();
    for c in graph.connections() {
        if c.dir == Direction::Unknown || c.is_self_loop() {
            continue;
        }
        out.entry(c.origin).or_default().push((c.dir, c.dest));
    }
    for v in out.values_mut() {
        v.sort_by_key(|&(d, dest)| (col(d), dest));
        v.dedup();
    }

    let mut came: BTreeMap<RoomId, (RoomId, Direction)> = BTreeMap::new();
    let mut seen: BTreeSet<RoomId> = BTreeSet::from([from]);
    let mut queue: VecDeque<RoomId> = VecDeque::from([from]);
    while let Some(cur) = queue.pop_front() {
        for &(dir, dest) in out.get(&cur).into_iter().flatten() {
            if !seen.insert(dest) {
                continue;
            }
            came.insert(dest, (cur, dir));
            if dest == to {
                return Some(unwind(&came, from, to));
            }
            queue.push_back(dest);
        }
    }
    None
}

/// Walk the predecessor chain back from `to` and hand it out forwards.
fn unwind(came: &BTreeMap<RoomId, (RoomId, Direction)>, from: RoomId, to: RoomId) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut at = to;
    while at != from {
        let Some(&(prev, dir)) = came.get(&at) else { break };
        steps.push(Step { room: prev, dir, dest: at });
        at = prev;
    }
    steps.reverse();
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::MAIN_LAYER;

    /// The directions a route spells out — what the player would actually type.
    fn dirs(steps: &[Step]) -> Vec<Direction> {
        steps.iter().map(|s| s.dir).collect()
    }

    /// A chain of `n` rooms, `1 —E→ 2 —E→ 3 …`, with `W` coming back each time.
    fn chain(n: u16) -> MapGraph {
        let mut g = MapGraph::new();
        for i in 1..=n {
            g.upsert_room(i.into(), format!("R{i}"));
        }
        for i in 1..n {
            g.add_edge(i.into(), Direction::E, (i + 1).into());
            g.add_edge((i + 1).into(), Direction::W, i.into());
        }
        g
    }

    #[test]
    fn a_straight_chain_is_walked_end_to_end() {
        let g = chain(4);
        let steps = route(&g, 1, 4).expect("the chain connects");
        assert_eq!(dirs(&steps), vec![Direction::E; 3]);
        assert_eq!(
            steps,
            vec![
                Step { room: 1, dir: Direction::E, dest: 2 },
                Step { room: 2, dir: Direction::E, dest: 3 },
                Step { room: 3, dir: Direction::E, dest: 4 },
            ],
            "every step names the room you leave, the way out, and where you land"
        );
        assert_eq!(route(&g, 4, 1).map(|s| dirs(&s)), Some(vec![Direction::W; 3]), "and back");
        assert_eq!(route(&g, 2, 2), Some(Vec::new()), "you are already there — not `no route`");
    }

    /// The rule the whole feature rests on: a passage is walkable in the direction it was walked,
    /// and NOT the other way. Treating edges as bidirectional would hand the player a route
    /// through a wall.
    #[test]
    fn a_one_way_passage_is_walkable_forward_and_not_backward() {
        let mut g = MapGraph::new();
        for i in 1u16..=3 {
            g.upsert_room(i.into(), format!("R{i}"));
        }
        g.add_edge(1, Direction::N, 2); // one-way: nothing comes back
        g.add_edge(2, Direction::E, 3);
        g.add_edge(3, Direction::W, 2);

        assert_eq!(
            route(&g, 1, 3).map(|s| dirs(&s)),
            Some(vec![Direction::N, Direction::E]),
            "forward through the one-way is fine"
        );
        assert_eq!(route(&g, 3, 1), None, "but there is no way back through it");
        assert_eq!(route(&g, 2, 1), None, "nor from the far side of it");
    }

    /// An `Unknown` edge — `xyzzy`, `pray` — is a real passage but not a direction you can type at
    /// a compass, so it is no more walkable here than it is bold in `matrix::entrances`.
    #[test]
    fn a_non_compass_passage_is_not_a_walkable_step() {
        let mut g = MapGraph::new();
        for i in 1u16..=2 {
            g.upsert_room(i.into(), format!("R{i}"));
        }
        g.add_edge(1, Direction::Unknown, 2);
        assert_eq!(route(&g, 1, 2), None, "the magic word is not a compass direction");
    }

    #[test]
    fn an_unreachable_room_gets_no_route_at_all() {
        let mut g = chain(3);
        g.upsert_room(99, "Island".into()); // known, mapped, and joined to nothing
        assert_eq!(route(&g, 1, 99), None);
        assert_eq!(route(&g, 99, 1), None);
        assert_eq!(route(&g, 1, 12345), None, "nor to a room the map has never heard of");
    }

    /// Layers are presentation, not walkability. A route that leaves the layer and comes back is
    /// still a route the player can walk, so the search must never be confined to one layer.
    #[test]
    fn a_route_crosses_layers_freely() {
        let mut g = chain(4);
        let other = g.new_layer(Some(MAIN_LAYER), "Cellar".into());
        g.set_room_layer(2, other);
        g.set_room_layer(3, other);

        let steps = route(&g, 1, 4).expect("the layer boundary is not a wall");
        assert_eq!(dirs(&steps), vec![Direction::E; 3]);
        assert_eq!(
            steps.iter().map(|s| s.room).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "and the out-of-layer steps are reported like any other — the caller decides what to draw"
        );
    }

    /// Two ways round, and the short one wins — a route that is merely *a* route is not what the
    /// player asked for.
    #[test]
    fn the_shorter_of_two_candidate_routes_wins() {
        // The long way round: 1 → 2 → 3 → 4 → 6, four steps. Built on its own first, so the test
        // can show that the long way IS a route before the short one is added to beat it.
        let mut g = MapGraph::new();
        for i in 1u16..=6 {
            g.upsert_room(i.into(), format!("R{i}"));
        }
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::N, 3);
        g.add_edge(3, Direction::N, 4);
        g.add_edge(4, Direction::N, 6);
        assert_eq!(route(&g, 1, 6).map(|s| s.len()), Some(4), "the long way is a route");

        // The short way: 1 → 5 → 6, two steps. Added LAST, so insertion order favours the loser.
        g.add_edge(1, Direction::Down, 5);
        g.add_edge(5, Direction::Down, 6);

        let steps = route(&g, 1, 6).expect("both ways connect");
        assert_eq!(dirs(&steps), vec![Direction::Down, Direction::Down], "the two-step way wins");
        assert_eq!(steps.len(), 2);
    }

    /// Ties are broken by column order, not by the order the edges were minted in, so a route
    /// survives a save/load round trip unchanged.
    #[test]
    fn equal_length_routes_break_ties_deterministically() {
        let mut g = MapGraph::new();
        for i in 1u16..=4 {
            g.upsert_room(i.into(), format!("R{i}"));
        }
        // Two one-step-each ways to room 4, minted worst-first.
        g.add_edge(1, Direction::W, 3); // W is column 3
        g.add_edge(3, Direction::S, 4);
        g.add_edge(1, Direction::N, 2); // N is column 0 — wins the tie
        g.add_edge(2, Direction::S, 4);

        assert_eq!(route(&g, 1, 4).map(|s| dirs(&s)), Some(vec![Direction::N, Direction::S]));

        let m = crate::mapper::Mapper { graph: g, ..Default::default() };
        let m2 = crate::persist::from_json(&crate::persist::to_json(&m)).expect("round trip");
        assert_eq!(
            route(&m2.graph, 1, 4).map(|s| dirs(&s)),
            Some(vec![Direction::N, Direction::S]),
            "the same route after a reload"
        );
    }
}
