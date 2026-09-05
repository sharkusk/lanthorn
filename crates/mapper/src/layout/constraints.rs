//! Build axis-separated separation constraints from compass edges, dropping the
//! minimal set that would otherwise make an axis's precedence graph cyclic
//! (a geometric contradiction). Dropped connections feed the `distorted` flag.

use std::collections::{BTreeMap, BTreeSet};

use crate::direction::{grid_offset, layout_offset, opposite};
use crate::graph::{MapGraph, RoomId};

use super::vpsc::Constraint;

/// One of a room's own bearings: which neighbour it named, and which way it said that
/// neighbour lay. See [`positionally_unreliable`].
type Bearing = (RoomId, (i32, i32));

/// Rooms whose own compass evidence cannot all be true at once, so no separation
/// constraint touching them is worth solving for (SQ-1289).
///
/// A room qualifies on **two** counts, both needed:
///
/// 1. **Its own observations disagree.** Two of the room's OUTGOING compass edges name
///    the same neighbour on opposing sides of an axis — "the valley is west of me" and
///    "the valley is east of me". One room cannot be on both sides of another, so at
///    least one of those walks did not start where the map thinks it did. That is the
///    fingerprint of a room the story SCATTERS the player into: Adventure's second
///    `In Forest` (SQ-1264) collects exits recorded from whichever forest the player
///    was actually standing in. Note OUTGOING only — a mutual `A E B` / `B E A`
///    contradiction is two rooms each making one coherent claim, and the acyclic pass
///    below already resolves that by dropping one leg.
/// 2. **Nothing better to fall back on.** The room has no reciprocated compass pair —
///    no passage walked from both ends, the strongest evidence the map has (SQ-1287).
///    A room that has one is anchored by it and keeps its constraints even if some
///    other neighbour's bearing is muddled.
///
/// Self-loops and non-compass directions (`Up`/`Down`/`In`/`Out`/`Unknown`) are not
/// evidence about where anything is, so neither test looks at them.
///
/// Such a room still lands on the map — the stress solve places it by graph distance
/// among its neighbours and the pack stage gives it a free cell LAST, after every
/// reliable room has claimed one — it simply stops pushing the rooms whose geometry IS
/// known apart to make room for a position it does not have.
pub fn positionally_unreliable(graph: &MapGraph) -> BTreeSet<RoomId> {
    let conns = graph.connections();
    let mut claims: BTreeMap<RoomId, Vec<Bearing>> = BTreeMap::new();
    let mut anchored: BTreeSet<RoomId> = BTreeSet::new();
    for c in conns {
        if c.is_self_loop() {
            continue;
        }
        let Some(off) = grid_offset(c.dir) else {
            continue;
        };
        claims.entry(c.origin).or_default().push((c.dest, off));
        if conns
            .iter()
            .any(|o| o.origin == c.dest && o.dest == c.origin && o.dir == opposite(c.dir))
        {
            anchored.insert(c.origin);
            anchored.insert(c.dest);
        }
    }
    claims
        .into_iter()
        .filter(|(id, _)| !anchored.contains(id))
        .filter(|(_, seen)| {
            seen.iter().enumerate().any(|(i, &(a, (ax, ay)))| {
                seen[i + 1..]
                    .iter()
                    .any(|&(b, (bx, by))| a == b && (ax * bx < 0 || ay * by < 0))
            })
        })
        .map(|(id, _)| id)
        .collect()
}

/// Separation constraints split by axis, plus the global connection indices whose
/// direction had to be dropped to keep each axis acyclic.
pub struct AxisConstraints {
    pub x: Vec<Constraint>,
    pub y: Vec<Constraint>,
    pub dropped: BTreeSet<usize>,
}

/// Is `a` reachable from `b` in the precedence graph `adj`? If so, adding the
/// edge `a → b` (a must be left of b) would close a cycle.
fn creates_cycle(adj: &[Vec<usize>], a: usize, b: usize) -> bool {
    let mut seen = vec![false; adj.len()];
    let mut stack = vec![b];
    seen[b] = true;
    while let Some(u) = stack.pop() {
        if u == a {
            return true;
        }
        for &v in &adj[u] {
            if !seen[v] {
                seen[v] = true;
                stack.push(v);
            }
        }
    }
    false
}

/// Build axis separation constraints for the component whose rooms are `ids`
/// (local index = position in `ids`). Connections in array order; a constraint
/// that would close a cycle on its axis is skipped and its connection index
/// recorded in `dropped`.
pub fn build_axis_constraints(graph: &MapGraph, ids: &[RoomId], gap: f64) -> AxisConstraints {
    let index: std::collections::HashMap<RoomId, usize> =
        ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    let n = ids.len();
    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut x_adj = vec![Vec::new(); n];
    let mut y_adj = vec![Vec::new(); n];
    let mut dropped = BTreeSet::new();

    // Chain equalities: reciprocal E/W chains share a row (equality on Y); reciprocal N/S
    // chains share a column (equality on X). Equality coord[a]==coord[b] is BOTH a≤b and
    // b≤a with gap 0 — block-merge collapses them to one coordinate when either is violated.
    // Both legs are added UNCONDITIONALLY: a gap-0 two-leg cycle is always feasible. They go
    // into *_adj so a later DIRECTIONAL constraint contradicting the equality is the one
    // creates_cycle drops (→ distorted). Added before the directional loop.
    let unreliable = positionally_unreliable(graph);
    let chains = super::chains::detect_chains(graph);
    fn add_equality(a: usize, b: usize, adj: &mut [Vec<usize>], out: &mut Vec<Constraint>) {
        adj[a].push(b);
        adj[b].push(a);
        out.push(Constraint { left: a, right: b, gap: 0.0 });
        out.push(Constraint { left: b, right: a, gap: 0.0 });
    }
    for members in &chains.ew_members {
        for w in members.windows(2) {
            if let (Some(&a), Some(&b)) = (index.get(&w[0]), index.get(&w[1])) {
                add_equality(a, b, &mut y_adj, &mut y); // E/W chain → equal Y
            }
        }
    }
    for members in &chains.ns_members {
        for w in members.windows(2) {
            if let (Some(&a), Some(&b)) = (index.get(&w[0]), index.get(&w[1])) {
                add_equality(a, b, &mut x_adj, &mut x); // N/S chain → equal X
            }
        }
    }

    // Directional constraints, STRONGEST EVIDENCE FIRST (SQ-1287/SQ-1291) rather than in the
    // order the player happened to mint them. Four tiers, insertion order breaking ties inside
    // each so the pass stays deterministic — EXCEPT that within a tier, a cardinal (N/S/E/W)
    // edge is tried before a diagonal one (SQ-1309):
    //
    //   0. A **reciprocated compass pair** — the passage walked from both ends, the two
    //      observations agreeing — is the best evidence about geometry the map has, so it claims
    //      its axis first and a contradicting edge is the one `creates_cycle` drops (SQ-1287).
    //   1. A **one-way compass edge**: one observation, but still the game's own word for where
    //      the room lies.
    //   2. **Up/Down**, reciprocated or not (SQ-1291). North-for-up is a DRAWING CONVENTION —
    //      `layout_offset` invents it, `grid_offset` does not — so a stairwell must never outrank
    //      a room that was actually described as lying north or south of somewhere. Zork I's
    //      `East-West Passage` and `Chasm` are joined by a reciprocated `Down`/`Up` pair AND by
    //      the chasm's one-way `SW` back to the passage; ranking the stairwell as a reciprocated
    //      pair pinned the chasm a row south and dropped the compass bearing, so the map
    //      contradicted both the game's prose ("a stairway leading down at the north end") and
    //      the player's own return walk.
    //   3. A **SOFT edge** ranks below every one of those (SQ-1312) — below `Up`/`Down` as well,
    //      so it is the FIRST constraint dropped when a cycle closes. A soft edge is a passage
    //      the story gates (see `crate::graph::Connection::soft`): the author put it there
    //      because the fiction wanted a way through, not because the two rooms are neighbours,
    //      and it is routinely the edge that makes a planar arrangement impossible. Zork I's
    //      kitchen WINDOW is the case — it makes `Behind House` the east end of the Kitchen's
    //      row while the outdoor ring needs it as its own east corner, and no grid holds a room
    //      in two places. The gated passage is the one to bend.
    //
    // Losing an Up/Down constraint costs nothing on screen: `mark_distorted` gates on
    // `grid_offset`, so a stairwell is never drawn distorted whatever happens here.
    //
    // **Cardinal before diagonal, within a tier** (SQ-1309). A cycle on the grid can always be
    // broken by stretching a diagonal instead of severing a cardinal pair: a diagonal edge only
    // pins its endpoint to the right QUADRANT (`mark_distorted`/`edge_is_satisfied` are sign-based
    // per axis, not exact-offset — see their doc comments), so a diagonal room landing one cell
    // further out than a unit step still satisfies its own bearing and draws as a slightly wider
    // corner, never as a broken door. A cardinal pair has no such slack: E/W or N/S mean exactly
    // one row or column apart, so severing one is a door the player can no longer see. Zork I's
    // house ring closes exactly this cycle — West of House → North of House → Behind House by
    // three reciprocated diagonals, then Behind House → Kitchen → Living Room → West of House by
    // three reciprocated CARDINALS back to the room the ring started from — and insertion order
    // alone (the diagonals happened to be minted first) sacrificed all three cardinal doors
    // instead of stretching one diagonal corner.
    let conns = graph.connections();
    let mut order: Vec<usize> = (0..conns.len()).collect();
    order.sort_by_key(|&ci| {
        let c = &conns[ci];
        let tier = if c.soft {
            3 // a passage the story gates: bend this one first
        } else if c.is_self_loop() || grid_offset(c.dir).is_none() {
            2 // Up/Down (and anything else with no compass bearing, which makes no constraint)
        } else if conns.iter().any(|o| {
            o.origin == c.dest
                && o.dest == c.origin
                && o.dir == crate::direction::opposite(c.dir)
                && !o.soft
        }) {
            0 // reciprocated compass pair, hard both ways
        } else {
            1 // one-way compass edge
        };
        let is_diagonal = matches!(
            c.dir,
            crate::direction::Direction::NE
                | crate::direction::Direction::NW
                | crate::direction::Direction::SE
                | crate::direction::Direction::SW
        );
        (tier, is_diagonal, ci)
    });
    for ci in order {
        let conn = &conns[ci];
        if conn.is_self_loop() {
            continue; // "x < x" is not a solvable constraint (SQ-0666)
        }
        if unreliable.contains(&conn.origin) || unreliable.contains(&conn.dest) {
            continue; // SQ-1289: a bearing to or from a scattered room places nobody
        }
        let (Some(&o), Some(&d)) = (index.get(&conn.origin), index.get(&conn.dest)) else {
            continue;
        };
        let Some((dx, dy)) = layout_offset(conn.dir) else {
            continue;
        };
        let mut this_dropped = false;

        // X: positive dx = dest east of origin = larger x; precedence left → right.
        if dx != 0 {
            let (left, right) = if dx > 0 { (o, d) } else { (d, o) };
            if creates_cycle(&x_adj, left, right) {
                this_dropped = true;
            } else {
                x_adj[left].push(right);
                x.push(Constraint { left, right, gap });
            }
        }
        // Y: north = smaller y. dy < 0 (north) ⇒ dest has smaller y ⇒ dest is "left".
        if dy != 0 {
            let (left, right) = if dy > 0 { (o, d) } else { (d, o) };
            if creates_cycle(&y_adj, left, right) {
                this_dropped = true;
            } else {
                y_adj[left].push(right);
                y.push(Constraint { left, right, gap });
            }
        }
        if this_dropped {
            dropped.insert(ci);
        }
    }

    AxisConstraints { x, y, dropped }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    fn two_rooms() -> MapGraph {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g
    }

    #[test]
    fn east_makes_x_constraint_origin_left() {
        let mut g = two_rooms();
        g.add_edge(1, Direction::E, 2); // B east of A → x[B] >= x[A] + gap
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        assert_eq!(ac.x.len(), 1);
        assert_eq!((ac.x[0].left, ac.x[0].right), (0, 1)); // local idx: A=0 left, B=1 right
        assert!(ac.y.is_empty());
        assert!(ac.dropped.is_empty());
    }

    #[test]
    fn north_makes_y_constraint_dest_left() {
        let mut g = two_rooms();
        g.add_edge(1, Direction::N, 2); // B north of A → y[B] <= y[A] → B is "left" on y
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        assert_eq!(ac.y.len(), 1);
        assert_eq!((ac.y[0].left, ac.y[0].right), (1, 0)); // B(idx1) left, A(idx0) right
        assert!(ac.x.is_empty());
    }

    #[test]
    fn diagonal_constrains_both_axes() {
        let mut g = two_rooms();
        g.add_edge(1, Direction::NE, 2); // B north-east of A
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        assert_eq!(ac.x.len(), 1, "NE has an east component");
        assert_eq!(ac.y.len(), 1, "NE has a north component");
        assert!(ac.dropped.is_empty());
    }

    /// SQ-0365: doors on two axes into the same room must COMPOSE, not cancel.
    ///
    /// `N` constrains only Y and `E` constrains only X, so together they say "above and to the
    /// right" — northeast — and nothing needs dropping. What broke it was the chain equalities:
    /// the pair counted as both an E/W chain (equal Y) and an N/S chain (equal X), and each
    /// equality made the OTHER's direction cycle, so BOTH were dropped and the room was left free
    /// to drift anywhere. On the real map it drifted north-WEST, satisfying neither door.
    #[test]
    fn doors_on_two_axes_into_one_room_compose_into_a_diagonal() {
        // Zork's Dam Lobby (1) has doors north AND east into the Maintenance Room (2).
        let mut g = two_rooms();
        g.add_edge(1, Direction::N, 2);
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::S, 1);
        g.add_edge(2, Direction::W, 1);
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        assert!(
            ac.dropped.is_empty(),
            "the two doors constrain different axes, so neither has to give: {:?}",
            ac.dropped
        );
        assert!(!ac.x.is_empty(), "east survives: the Maintenance Room is to the right");
        assert!(!ac.y.is_empty(), "north survives: and above — i.e. north-east");
    }

    #[test]
    fn contradiction_drops_one_constraint() {
        // A→N→B and B→N→A: both want the other north → cycle on the y axis.
        let mut g = two_rooms();
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::N, 1);
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        // First N kept, second N dropped (would close a cycle).
        assert_eq!(ac.y.len(), 1, "exactly one y constraint survives");
        assert_eq!(ac.dropped.len(), 1, "the cycle-closing connection is dropped");
    }

    #[test]
    fn updown_edge_makes_a_y_constraint_like_north() {
        // Up now counts as a weight-1 N/S layout hint (layout_offset), so it produces the
        // same y constraint a plain N edge would — unlike a truly non-compass edge (Unknown),
        // which still makes no constraints at all.
        let mut g = two_rooms();
        g.add_edge(1, Direction::Up, 2); // B "north" of A → y[B] <= y[A] → B is "left" on y
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        assert_eq!(ac.y.len(), 1, "Up produces a y constraint, same as N");
        assert_eq!((ac.y[0].left, ac.y[0].right), (1, 0)); // B(idx1) left, A(idx0) right
        assert!(ac.x.is_empty());
        assert!(ac.dropped.is_empty());
    }

    // ── SQ-1291: Up/Down rank below every compass edge ───────────────────────────

    /// The Zork I shape. `East-West Passage` and `Chasm` are joined by a stairwell walked from
    /// BOTH ends (`136 Down 112` / `112 Up 136`) and, separately, by the chasm's ONE-WAY
    /// `southwest` back to the passage. Under SQ-1287's two tiers the stairwell counted as a
    /// reciprocated pair, claimed the Y axis first, and pinned the chasm a row SOUTH; the
    /// compass bearing then closed a cycle and was dropped. North-for-up is a drawing
    /// convention, so the one-way compass word must win instead.
    #[test]
    fn a_one_way_compass_bearing_outranks_a_reciprocated_stairwell() {
        let mut g = two_rooms(); // 1 = the passage, 2 = the chasm
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);
        g.add_edge(2, Direction::SW, 1); // the passage is south-WEST of the chasm
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        // SW's y leg: dy > 0, so origin(the chasm, local idx 1) is "left" — the smaller y, i.e.
        // the chasm to the NORTH. Exactly one y constraint survives, and it is that one.
        assert_eq!(ac.y.len(), 1, "one y constraint survives the contest: {:?}", ac.y);
        assert_eq!((ac.y[0].left, ac.y[0].right), (1, 0), "the chasm sits north of the passage");
        assert_eq!(ac.x.len(), 1, "and SW's west leg places the passage on the x axis too");
        assert_eq!((ac.x[0].left, ac.x[0].right), (0, 1), "the passage is west of the chasm");
        let dropped: Vec<_> = ac.dropped.iter().map(|&i| g.connections()[i].dir).collect();
        assert!(
            dropped.contains(&Direction::Down) || dropped.contains(&Direction::Up),
            "a leg of the stairwell is what gives way, not the bearing: {dropped:?}"
        );
        assert!(!dropped.contains(&Direction::SW), "the compass bearing survives: {dropped:?}");
    }

    /// …and the stairwell pays nothing on screen for losing: `mark_distorted` gates on
    /// `grid_offset`, which is `None` for Up/Down, so a dropped stairwell constraint never
    /// draws a passage as distorted.
    #[test]
    fn a_dropped_stairwell_constraint_never_draws_distorted() {
        let mut g = two_rooms();
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);
        g.add_edge(2, Direction::SW, 1);
        super::super::relayout_auto(&mut g);
        for c in g.connections() {
            assert!(!c.distorted, "nothing here may draw distorted: {:?}", c.dir);
        }
    }

    /// A one-way compass edge still yields to a RECIPROCATED one (SQ-1287's tier, unchanged):
    /// the three tiers are ordered, not flattened into two.
    #[test]
    fn a_reciprocated_compass_pair_still_outranks_a_one_way_compass_edge() {
        let mut g = two_rooms();
        g.upsert_room(3, "C".into());
        g.add_edge(1, Direction::N, 3); // one-way, minted FIRST
        g.add_edge(1, Direction::S, 3); // …and reciprocated, minted second
        g.add_edge(3, Direction::N, 1);
        let ac = build_axis_constraints(&g, &[1, 3], 1.0);
        assert!(
            ac.y.iter().all(|c| (c.left, c.right) == (0, 1)),
            "both legs of the reciprocated pair agree that C is south: {:?}",
            ac.y
        );
        assert_eq!(ac.dropped.len(), 1, "and the one-way N edge is the one dropped");
        assert_eq!(
            g.connections()[*ac.dropped.iter().next().unwrap()].dir,
            Direction::N,
            "specifically the one-way north edge"
        );
    }

    #[test]
    fn unknown_edges_make_no_constraints() {
        let mut g = two_rooms();
        g.add_edge(1, Direction::Unknown, 2);
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        assert!(ac.x.is_empty() && ac.y.is_empty() && ac.dropped.is_empty());
    }

    // ── SQ-1289: rooms whose own compass claims contradict each other ────────────

    /// The Adventure shape (SQ-1264/SQ-1287): a room that says the SAME neighbour is both
    /// west and east of it. One room cannot be on two sides of another, so at least one of
    /// those walks did not start where the map thinks it did — the fingerprint of a room the
    /// story scatters the player into.
    #[test]
    fn a_room_that_puts_one_neighbour_both_east_and_west_of_itself_is_unreliable() {
        let mut g = two_rooms();
        g.upsert_room(3, "C".into());
        g.add_edge(2, Direction::S, 3);
        g.add_edge(3, Direction::N, 2); // B/C reciprocated: they are the anchored skeleton
        g.add_edge(1, Direction::W, 2); // A: "B is west of me"
        g.add_edge(1, Direction::E, 2); // A: "…and east of me"
        assert_eq!(
            positionally_unreliable(&g).into_iter().collect::<Vec<_>>(),
            vec![1],
            "only A's own claims disagree; B and C are walked from both ends"
        );
    }

    /// Condition 2 on its own. The same contradiction, but the room ALSO has a passage walked
    /// from both ends — the strongest evidence the map has (SQ-1287) — so it keeps a position
    /// and keeps its constraints; one muddled bearing does not unmoor an anchored room.
    #[test]
    fn a_contradicting_room_with_a_reciprocated_pair_keeps_its_geometry() {
        let mut g = two_rooms();
        g.upsert_room(3, "C".into());
        g.add_edge(1, Direction::W, 2);
        g.add_edge(1, Direction::E, 2);
        g.add_edge(1, Direction::N, 3);
        g.add_edge(3, Direction::S, 1); // A ↔ C walked both ways
        assert!(positionally_unreliable(&g).is_empty(), "the reciprocated pair anchors A");
    }

    /// OUTGOING only. `A E B` and `B E A` is two rooms each making ONE coherent claim that
    /// happen to disagree with each other — the acyclic pass already resolves that by dropping
    /// one leg (see `contradiction_drops_one_constraint`), and unmooring BOTH rooms instead
    /// would throw away the half of the evidence that is fine.
    #[test]
    fn a_mutual_two_room_contradiction_unmoors_neither_room() {
        let mut g = two_rooms();
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::E, 1);
        assert!(positionally_unreliable(&g).is_empty());
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        assert_eq!(ac.x.len(), 1, "one leg still survives");
        assert_eq!(ac.dropped.len(), 1, "and the cycle-closing one is still dropped");
    }

    /// The commonest shape on any half-explored map: one one-way step into a room nobody has
    /// walked back out of. It must keep its constraint, or every freshly-found room would
    /// float off the passage that found it.
    #[test]
    fn a_room_reached_by_a_single_one_way_step_is_not_unreliable() {
        let mut g = two_rooms();
        g.add_edge(1, Direction::N, 2);
        assert!(positionally_unreliable(&g).is_empty());
        assert_eq!(build_axis_constraints(&g, &[1, 2], 1.0).y.len(), 1, "north still places B");
    }

    /// Up/Down/In/Out are not bearings, so they are not evidence either way: a room reached
    /// both by going north and by going down is an ordinary shortcut, not a contradiction.
    /// (`grid_offset` is the same gate `mark_distorted` and SQ-1287's reciprocation test use.)
    #[test]
    fn an_up_or_down_shortcut_beside_a_compass_edge_is_not_a_contradiction() {
        let mut g = two_rooms();
        g.add_edge(1, Direction::N, 2);
        g.add_edge(1, Direction::Down, 2);
        assert!(positionally_unreliable(&g).is_empty());
    }

    /// Self-loops say nothing about where anything is (SQ-0666), so a room whose compass
    /// exits all lead back to itself is not thereby contradicting anything.
    #[test]
    fn self_loops_are_never_evidence_of_a_contradiction() {
        let mut g = two_rooms();
        g.add_self_loop(1, Direction::E);
        g.add_self_loop(1, Direction::W);
        g.add_edge(1, Direction::N, 2);
        assert!(positionally_unreliable(&g).is_empty());
    }

    /// The consequence for the solver: EVERY edge touching an unreliable room is left out,
    /// inbound ones included — a bearing INTO a scattered room places the room it left just
    /// as badly as one out of it. Here `HILL S FOREST` is what pushed Adventure's valley a
    /// second row down (SQ-1289).
    #[test]
    fn no_separation_constraint_survives_that_touches_an_unreliable_room() {
        let mut g = two_rooms();
        g.upsert_room(3, "C".into());
        g.add_edge(2, Direction::S, 3);
        g.add_edge(3, Direction::N, 2);
        g.add_edge(1, Direction::W, 2);
        g.add_edge(1, Direction::E, 2);
        g.add_edge(3, Direction::S, 1); // inbound: C says the muddled room is south of it
        let ac = build_axis_constraints(&g, &[1, 2, 3], 1.0);
        assert!(positionally_unreliable(&g).contains(&1));
        // Only the B/C pair is left: its N/S equality on x, and one y separation per leg.
        assert!(ac.x.iter().all(|c| c.gap == 0.0), "the only x constraints are B/C's equality");
        assert_eq!(ac.y.len(), 2, "just the two legs of the B/C pair: {:?}", ac.y);
    }

    #[test]
    fn reciprocal_ew_pair_emits_y_equality() {
        // Reciprocal E/W: room 1 says "2 is east of me", room 2 says "1 is west of me".
        // build_axis_constraints must emit TWO gap-0 Y constraints (the equality pair)
        // in addition to the directional X constraint. Without the chain-equality block
        // (detect_chains / add_equality), ac.y would be empty and this test fails RED.
        let mut g = two_rooms();
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);

        // The directional edges produce X constraints only; Y must come from the equality block.
        let y_zeros: Vec<_> = ac.y.iter().filter(|c| c.gap == 0.0).collect();
        assert!(
            y_zeros.len() >= 2,
            "expected >=2 gap-0 Y constraints from chain equality, got {}",
            y_zeros.len()
        );
        let has_01 = ac.y.iter().any(|c| c.left == 0 && c.right == 1 && c.gap == 0.0);
        let has_10 = ac.y.iter().any(|c| c.left == 1 && c.right == 0 && c.gap == 0.0);
        assert!(has_01, "missing Y equality leg (0,1,0.0)");
        assert!(has_10, "missing Y equality leg (1,0,0.0)");
    }

    #[test]
    fn reciprocal_ns_pair_emits_x_equality() {
        // Reciprocal N/S: the column-sharing analogue of the E/W test above. The equality
        // must land on X (shared column). Without the chain-equality block ac.x would hold
        // only the directional gap-1 constraints, so this fails RED.
        let mut g = two_rooms();
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        let has_01 = ac.x.iter().any(|c| c.left == 0 && c.right == 1 && c.gap == 0.0);
        let has_10 = ac.x.iter().any(|c| c.left == 1 && c.right == 0 && c.gap == 0.0);
        assert!(has_01, "missing X equality leg (0,1,0.0)");
        assert!(has_10, "missing X equality leg (1,0,0.0)");
    }

    // ── SQ-1309: a cardinal reciprocal outranks a diagonal reciprocal on a shared tier ──

    /// A direct, minimal version of the conflict SQ-1309's tiering exists for: a diagonal
    /// reciprocal pair and a cardinal reciprocal pair that flatly disagree about which of two
    /// rooms is west of the other, so exactly one must be dropped as the cycle-closer. `1 NE 2`
    /// (reciprocated by `2 SW 1`) says room 1 is west of room 2; `1 W 2` (reciprocated by `2 E 1`)
    /// says the opposite. Both pairs share tier 0 (SQ-1287: both reciprocated), so before
    /// SQ-1309 raw insertion order broke the tie — here the diagonal is minted first, so it would
    /// win and the cardinal would be the one `creates_cycle` drops. Falsify by removing the
    /// `is_diagonal` term from the sort key in `build_axis_constraints`: the cardinal legs (indices
    /// 2 and 3, minted after the diagonal) are then the ones in `ac.dropped`.
    #[test]
    fn a_cardinal_reciprocal_beats_a_directly_conflicting_diagonal_reciprocal() {
        let mut g = two_rooms();
        g.add_edge(1, Direction::NE, 2); // ci=0: diagonal, minted FIRST — says 1 west of 2
        g.add_edge(2, Direction::SW, 1); // ci=1: its reciprocal partner
        g.add_edge(1, Direction::W, 2); // ci=2: cardinal, minted SECOND — says 2 west of 1
        g.add_edge(2, Direction::E, 1); // ci=3: its reciprocal partner
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        // Both x constraints agree (the reciprocal cardinal pair is redundant with itself) and
        // both are the CARDINAL's claim: local idx 1 (room 2) west of idx 0 (room 1).
        assert!(ac.x.iter().all(|c| (c.left, c.right) == (1, 0)), "the cardinal's claim (2 west of 1) wins: {:?}", ac.x);
        // The diagonal pair (ci 0 and 1) is what gives way, not the cardinal (ci 2 and 3).
        assert!(ac.dropped.contains(&0) || ac.dropped.contains(&1), "a diagonal leg must be dropped: {:?}", ac.dropped);
        assert!(!ac.dropped.contains(&2) && !ac.dropped.contains(&3), "the cardinal pair must survive: {:?}", ac.dropped);
    }

    /// The exact shape around Zork I's white house: a diagonal ring — West of House ↔ North of
    /// House ↔ Behind House ↔ South of House ↔ West of House, each leg a reciprocated diagonal —
    /// plus the game's own one-way N/S/E/W bearings between those same four rooms, plus the front
    /// door itself: Behind House ↔ Kitchen ↔ Living Room, each leg a reciprocated CARDINAL.
    ///
    /// On the real map (SQ-1309) this shape's cardinal doors were dropped, but NOT by a raw
    /// `creates_cycle` tie here — on this isolated 6-room graph, nothing conflicts and this test
    /// passes unchanged with or without the `is_diagonal` tiering above (verified: reverting it
    /// leaves this test green). The real cause was `app::mapgen` running one shared relayout over
    /// EVERY layer at once, so the house's chain competed with a hundred unrelated rooms from
    /// other layers for cells and `contiguify` (`mod.rs`) evicted the house's own chain members as
    /// if they were foreign interlopers. This test stays as an end-to-end regression on the shape
    /// itself — see `crates/app/tests/suites/sq1308_mapgen_layers.rs` and the real-game assertions
    /// in `sq1306_mapgen`/`sq1308_mapgen_layers` for the tests that actually falsify against the
    /// per-layer and contiguify fixes.
    #[test]
    fn a_cardinal_reciprocal_outranks_a_diagonal_reciprocal_closing_the_same_cycle() {
        use Direction::*;
        let mut g = MapGraph::new();
        for (id, name) in [
            (68u32, "West of House"),
            (143, "North of House"),
            (217, "South of House"),
            (89, "Behind House"),
            (28, "Kitchen"),
            (79, "Living Room"),
        ] {
            g.upsert_room(id, name.into());
        }
        // Diagonal ring: three reciprocated diagonal pairs.
        g.add_edge(68, NE, 143);
        g.add_edge(143, SW, 68);
        g.add_edge(143, SE, 89);
        g.add_edge(89, NW, 143);
        g.add_edge(217, NE, 89);
        g.add_edge(89, SW, 217);
        // The game's own one-way N/S/E/W bearings among the ring rooms (not reciprocated).
        g.add_edge(68, N, 143);
        g.add_edge(68, S, 217);
        g.add_edge(217, W, 68);
        g.add_edge(89, N, 143);
        g.add_edge(143, E, 89);
        g.add_edge(89, S, 217);
        g.add_edge(217, E, 89);
        // The front door: two reciprocated cardinal pairs closing the cycle back toward West of House.
        g.add_edge(89, W, 28);
        g.add_edge(28, E, 89);
        g.add_edge(28, W, 79);
        g.add_edge(79, E, 28);

        super::super::relayout_auto(&mut g);

        let pos = |id: u32| g.room(id).unwrap().pos.unwrap();
        let (p_kitchen, p_behind, p_living, p_west) = (pos(28), pos(89), pos(79), pos(68));

        // Kitchen, Behind House and Living Room all sit on one row, in that order — and,
        // critically, no OTHER room in the graph rounds onto the row strictly between the
        // Kitchen/Behind and Kitchen/Living pairs (the shape SQ-1309's `contiguify` protection
        // in `mod.rs` also guards, though nothing here should trigger it — this graph has no
        // room outside the chain to interlope).
        assert_eq!(p_kitchen.1, p_behind.1, "Kitchen and Behind House share a row: {p_kitchen:?} {p_behind:?}");
        assert_eq!(p_kitchen.1, p_living.1, "Kitchen and Living Room share a row: {p_kitchen:?} {p_living:?}");
        let (lo, hi) = (p_kitchen.0.min(p_behind.0), p_kitchen.0.max(p_behind.0));
        assert!(
            !g.rooms().any(|r| r.id != 28
                && r.id != 89
                && r.pos.is_some_and(|p| p.1 == p_kitchen.1 && p.0 > lo && p.0 < hi)),
            "no foreign room interleaves Kitchen and Behind House on their row"
        );
        let (lo, hi) = (p_kitchen.0.min(p_living.0), p_kitchen.0.max(p_living.0));
        assert!(
            !g.rooms().any(|r| r.id != 28
                && r.id != 79
                && r.pos.is_some_and(|p| p.1 == p_kitchen.1 && p.0 > lo && p.0 < hi)),
            "no foreign room interleaves Kitchen and Living Room on their row"
        );
        assert_ne!(p_living, p_west, "Living Room must not land on West of House's own cell");

        let distorted = |o: RoomId, d: Direction, dest: RoomId| {
            g.connections().iter().find(|c| c.origin == o && c.dir == d && c.dest == dest).unwrap().distorted
        };
        assert!(!distorted(89, W, 28), "Behind House→W→Kitchen must survive");
        assert!(!distorted(28, E, 89), "Kitchen→E→Behind House must survive");
        assert!(!distorted(28, W, 79), "Kitchen→W→Living Room must survive");
        assert!(!distorted(79, E, 28), "Living Room→E→Kitchen must survive");
    }
}
