//! Bidirectional cardinal chains: maximal runs of rooms joined by reciprocal E/W
//! (share a row) or reciprocal N/S (share a column) edges. A pure function of the
//! graph, used by the layout (alignment + contiguity) and the rules display.

use std::collections::{BTreeMap, BTreeSet};

use crate::direction::{grid_offset, opposite};
use crate::graph::{MapGraph, RoomId};

pub struct Chains {
    /// room → its E/W chain id (rooms sharing a row), if any.
    pub ew: BTreeMap<RoomId, usize>,
    /// room → its N/S chain id (rooms sharing a column), if any.
    pub ns: BTreeMap<RoomId, usize>,
    pub ew_members: Vec<Vec<RoomId>>,
    pub ns_members: Vec<Vec<RoomId>>,
}

pub fn detect_chains(graph: &MapGraph) -> Chains {
    let conns = graph.connections();
    let reciprocal = |a: RoomId, b: RoomId, dir| {
        conns.iter().any(|c| c.origin == b && c.dest == a && c.dir == opposite(dir))
    };
    let mut ew_pairs: Vec<(RoomId, RoomId)> = Vec::new();
    let mut ns_pairs: Vec<(RoomId, RoomId)> = Vec::new();
    for c in conns {
        if c.is_self_loop() {
            continue; // a room is not chained to itself (SQ-0666)
        }
        match grid_offset(c.dir) {
            Some((dx, dy)) if dx != 0 && dy == 0 => {
                if reciprocal(c.origin, c.dest, c.dir) {
                    ew_pairs.push((c.origin, c.dest));
                }
            }
            Some((dx, dy)) if dy != 0 && dx == 0
                && reciprocal(c.origin, c.dest, c.dir) => {
                ns_pairs.push((c.origin, c.dest));
            }
            _ => {}
        }
    }
    // A pair joined BOTH ways — Zork's Dam Lobby has doors north AND east into the one
    // Maintenance Room, with south and west back — cannot chain on either axis. Equal-Y and
    // equal-X together say the two rooms occupy one cell, and `build_axis_constraints` adds both
    // equalities before the directional pass, so each one made the OTHER's direction cycle and
    // both got dropped. The pair was then left with no constraint at all and drifted wherever the
    // solver liked — north-WEST on the real map, honouring neither door.
    //
    // Drop just the pair, not the chains it sits in: the Lobby is still genuinely north of the
    // Dam, so that leg of the N/S chain must survive. Without the equalities the two directions
    // simply compose — north gives "above", east gives "right" — and the room lands northeast.
    let key = |a: RoomId, b: RoomId| (a.min(b), a.max(b));
    let ew_keys: BTreeSet<(RoomId, RoomId)> = ew_pairs.iter().map(|&(a, b)| key(a, b)).collect();
    let ns_keys: BTreeSet<(RoomId, RoomId)> = ns_pairs.iter().map(|&(a, b)| key(a, b)).collect();
    let both: BTreeSet<(RoomId, RoomId)> = ew_keys.intersection(&ns_keys).copied().collect();
    ew_pairs.retain(|&(a, b)| !both.contains(&key(a, b)));
    ns_pairs.retain(|&(a, b)| !both.contains(&key(a, b)));

    let (ew, ew_members) = build(&ew_pairs);
    let (ns, ns_members) = build(&ns_pairs);
    Chains { ew, ns, ew_members, ns_members }
}

/// Union-find the pairs, then assign chain ids in ascending lowest-member order
/// (deterministic). Returns (room→chain id, chain id→sorted members).
fn build(pairs: &[(RoomId, RoomId)]) -> (BTreeMap<RoomId, usize>, Vec<Vec<RoomId>>) {
    // Union-find over the room ids present in `pairs`.
    let mut parent: BTreeMap<RoomId, RoomId> = BTreeMap::new();
    fn find(parent: &mut BTreeMap<RoomId, RoomId>, x: RoomId) -> RoomId {
        let p = *parent.get(&x).unwrap_or(&x);
        if p == x {
            x
        } else {
            let r = find(parent, p);
            parent.insert(x, r);
            r
        }
    }
    for &(a, b) in pairs {
        parent.entry(a).or_insert(a);
        parent.entry(b).or_insert(b);
        let ra = find(&mut parent, a);
        let rb = find(&mut parent, b);
        if ra != rb {
            // Union toward the smaller root for determinism.
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            parent.insert(hi, lo);
        }
    }
    // Group members by root.
    let members_by_root: BTreeMap<RoomId, Vec<RoomId>> = {
        let ids: Vec<RoomId> = parent.keys().copied().collect();
        let mut m: BTreeMap<RoomId, Vec<RoomId>> = BTreeMap::new();
        for id in ids {
            let r = find(&mut parent, id);
            m.entry(r).or_default().push(id);
        }
        m
    };
    // Assign chain ids in ascending root order; only keep groups of ≥2.
    let mut room_chain: BTreeMap<RoomId, usize> = BTreeMap::new();
    let mut chains: Vec<Vec<RoomId>> = Vec::new();
    for (_root, mut members) in members_by_root {
        if members.len() < 2 {
            continue;
        }
        members.sort_unstable();
        let id = chains.len();
        for &r in &members {
            room_chain.insert(r, id);
        }
        chains.push(members);
    }
    (room_chain, chains)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    #[test]
    fn reciprocal_ew_pair_is_one_chain() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1); // reciprocal E/W
        let c = detect_chains(&g);
        assert_eq!(c.ew.get(&1), c.ew.get(&2), "both in the same E/W chain");
        assert!(c.ew.contains_key(&1));
        assert!(c.ns.is_empty(), "no N/S chain");
    }

    #[test]
    fn non_reciprocal_pair_is_no_chain() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::E, 2); // one-way; no 2→W→1
        let c = detect_chains(&g);
        assert!(c.ew.is_empty(), "one-way edge forms no chain");
    }

    #[test]
    fn same_origin_n_and_s_is_no_chain() {
        // 3→N→7 and 3→S→7: same origin, not reciprocal (no 7→S→3 / 7→N→3).
        let mut g = MapGraph::new();
        g.upsert_room(3, "a".into());
        g.upsert_room(7, "b".into());
        g.add_edge(3, Direction::N, 7);
        g.add_edge(3, Direction::S, 7);
        let c = detect_chains(&g);
        assert!(c.ns.is_empty(), "same-origin N+S is not a reciprocal pair");
    }

    /// SQ-0365: Zork's Dam Lobby has doors NORTH and EAST into the one Maintenance Room (south and
    /// west back), so the pair was reciprocal on both axes and landed in an E/W chain AND an N/S
    /// chain — equal-Y and equal-X, i.e. one cell for two rooms. `build_axis_constraints` adds both
    /// equalities before the directional pass, so each made the other's direction cycle and BOTH
    /// were dropped; the pair, left unconstrained, drifted north-WEST, honouring neither door.
    #[test]
    fn a_pair_joined_on_both_axes_chains_on_neither() {
        let mut g = MapGraph::new();
        g.upsert_room(126, "Dam Lobby".into());
        g.upsert_room(233, "Maintenance Room".into());
        g.upsert_room(186, "Dam".into());
        g.add_edge(186, Direction::N, 126);
        g.add_edge(126, Direction::S, 186);
        g.add_edge(126, Direction::N, 233);
        g.add_edge(126, Direction::E, 233);
        g.add_edge(233, Direction::S, 126);
        g.add_edge(233, Direction::W, 126);

        let c = detect_chains(&g);
        assert!(c.ew.is_empty(), "the two-axis pair chains on neither axis: {:?}", c.ew_members);
        assert_eq!(c.ns.get(&126), c.ns.get(&186), "but the Dam is still genuinely below the Lobby");
        assert!(!c.ns.contains_key(&233), "and the Maintenance Room is in no column chain");
    }

    /// The exclusion is for a pair reciprocal on BOTH axes — not for any room that merely belongs
    /// to one chain of each kind, which is ordinary (see `three_room_ew_chain_and_cross_chain_room`).
    #[test]
    fn a_pair_joined_on_one_axis_still_chains() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(1, Direction::N, 2); // one-way N, with no S back: not a reciprocal N/S pair
        let c = detect_chains(&g);
        assert_eq!(c.ew.get(&1), c.ew.get(&2), "the reciprocal E/W pair still shares a row");
        assert!(c.ew.contains_key(&1));
    }

    #[test]
    fn three_room_ew_chain_and_cross_chain_room() {
        // 79↔203↔193 is one E/W chain; 74↔76 is an N/S chain; 74↔79 puts 74 in the E/W chain too.
        let mut g = MapGraph::new();
        for id in [74u16, 76, 79, 193, 203] { g.upsert_room(id.into(), "r".into()); }
        for (o, d, dst) in [
            (79, Direction::W, 203), (203, Direction::E, 79),
            (203, Direction::W, 193), (193, Direction::E, 203),
            (74, Direction::W, 79), (79, Direction::E, 74),
            (74, Direction::S, 76), (76, Direction::N, 74),
        ] { g.add_edge(o, d, dst); }
        let c = detect_chains(&g);
        // 74,79,203,193 all share one E/W chain.
        let e = c.ew.get(&74).copied();
        assert!(e.is_some());
        assert_eq!(c.ew.get(&79).copied(), e);
        assert_eq!(c.ew.get(&203).copied(), e);
        assert_eq!(c.ew.get(&193).copied(), e);
        // 74 and 76 share one N/S chain.
        assert_eq!(c.ns.get(&74), c.ns.get(&76));
        assert!(c.ns.contains_key(&74));
        // 74 is a cross-chain room: in an E/W chain AND an N/S chain.
        assert!(c.ew.contains_key(&74) && c.ns.contains_key(&74));
    }
}
