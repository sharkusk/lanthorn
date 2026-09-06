//! Seating a LATE-ARRIVING room beside the room it hangs off (SQ-1356).
//!
//! Two callers, one question. A cross-layer **ghost** (`crate::render::render_layer`) stands for
//! a room on another layer and is placed after every real room on this one; a **portal-only
//! room** — a room whose every passage is Up/Down/In/Out, Zork I's `Attic` and `Cellar` being
//! the specimens — has no compass bearing for the stress solve to seat it by, so it too arrives
//! after everything with a bearing. Both want the same thing: the cell adjacent to their anchor
//! in the passage's own direction, and a map that OPENS to make room for it rather than parking
//! them wherever a free cell happened to be left over.
//!
//! The gap is opened the way `open_gated_holes_for_hubs` opens one — by sliding a whole side of
//! the map away by a cell — but along a LINE rather than along a run: every room at or beyond
//! the wanted cell's row (or column) moves one cell further out, which inserts a blank line
//! there. Every pair of rooms on the same side of that cut keeps its exact offset, so the only
//! links that can stretch are the ones that STRADDLE the cut.
//!
//! **And a straddling CARDINAL RECIPROCAL is what vetoes the whole shift.** "Exactly one cell
//! apart" is what a reciprocal pair means (see the module docs on `layout`), so a line whose
//! opening would pull one apart is not opened at all. A passage that is one-way, diagonal, gated
//! or already stretched may lengthen — none of those claims a cell count.
//!
//! **The word CARDINAL in that sentence went unimplemented for four quests** (SQ-1375).
//! [`adjacent_reciprocals`] asked `grid_offset`, which answers `Some` for all eight compass
//! points, so a DIAGONAL pair was as binding here as a cardinal one — against this module's own
//! rule above, against `layout::edge_is_satisfied`'s (SQ-1364) and against `mark_distorted`'s,
//! all three of which let a diagonal stretch to any distance inside its quadrant. Zork I's
//! `Attic` paid for it: it hangs off the `Kitchen` by a staircase and wants the cell above, and
//! the row that would have opened for it was vetoed by `North of House` sitting one diagonal step
//! from `Behind House` — a diagonal the LAYOUT itself had written, as the repair for a walked `E`
//! that came out distorted. So the `Attic` walked four cells up a column of its own, past
//! `North of House`, `Forest Path` and `Clearing`, to reach open ground. With the diagonal free to
//! stretch, the row opens, every cardinal pair on the map keeps its cell, and the `Attic` is drawn
//! on the doorstep.
//!
//! **A whole-side slide is all-or-nothing, though, and that was not enough** (SQ-1358). Zork I's
//! house: the Studio's stairs arrive from below the `Kitchen`, whose doorstep `South of House`
//! holds. Opening the row would have separated a `Clearing` from the `Forest` directly below it —
//! a walked north/south pair far away on the same cut, with nothing to do with the house — so the
//! slide was vetoed and the ghost was parked BELOW `South of House`, its line looping around a
//! room the crossing never touches. But `South of House` itself is joined to the house only by
//! distorted passages: it is half of no adjacent pair at all, and could have stepped down a cell
//! on its own. So a blocked bearing now tries a **chain push** as well — move the blocker one
//! cell further along the bearing, and whatever IT would then displace with it, as a rigid chain
//! — under exactly the same veto, room by room instead of side by side.
//!
//! **And a pushed room takes what HANGS OFF it with it** (SQ-1363). The same Zork I map, one room
//! further on: `South of House` stepped down correctly, and the `Forest` its own `S` exit leads to
//! stayed where it was, so the two ended level and the map stopped saying the forest was south of
//! the house at all. A push is only legible if the rooms that depend on the pushed room travel
//! with it, so the chain is extended with its **dependants** — see [`hangs_off`] — before anything
//! moves.
//!
//! **But that rule may not CANCEL a push** (SQ-1367). On Zork I's Maze the dependant closure is
//! transitive through a tangle of passages that point every which way, so pushing one room up a
//! cell gathered nine more, outgrew [`MAX_PUSH_SET`] and refused the whole push — and the
//! `Clearing` ghost, whose `Down` wanted exactly that cell, was parked east of `Grating Room`
//! instead, four turns of line between two adjacent boxes. The dependants say how a push travels;
//! a party that is oversized or vetoed falls back to the bare column ([`push_chain`]), under the
//! same veto, rather than giving up the cell.
//!
//! **And a KEPT ADJACENCY is another way a push travels, not a reason to refuse one** (SQ-1375).
//! SQ-1363 swept in the rooms downwind of the party and left the ones beside it standing — so a
//! blocker with a cardinal neighbour due east of it could not be pushed north at all, because the
//! neighbour would have been stranded and [`apply_push`] refuses exactly that. The same fact reads
//! two ways, and the user's rule settles which: when a room gets pushed, the rooms that depend on
//! it get pushed the same way, and the neighbour whose adjacency the map is promising to keep is
//! as dependent as the room downwind. [`stranded_partners`] brings it along, transitively and
//! together with whatever it displaces, under the same [`MAX_PUSH_SET`] and the same fallback to
//! the bare column. Zork I's house again: `Forest Path` cannot step north without `Forest`, due
//! west of it, coming too. The one room that can never join is the ANCHOR — a party containing it
//! lands on the very cell the newcomer is being seated in, which [`apply_push`] already refuses.

use std::collections::{BTreeMap, BTreeSet};

use crate::direction::{grid_offset, layout_offset, Direction};
use crate::graph::{MapGraph, RoomId};

/// The grid step a passage travelling `dir` wants its far end to sit at, for SEATING purposes.
///
/// [`layout_offset`]'s answer where it has one (so Up seats north and Down south, exactly as the
/// solve already lays them out), and the horizontal pair the drawn and vector maps already give
/// In and Out — `export_svg::side_for_travel` puts both on the right-hand side, and a seated room
/// needs the two to differ so an In and an Out from one room do not want the same cell.
/// `Unknown` has no bearing at all and is never seated.
pub fn seat_offset(dir: Direction) -> Option<(i32, i32)> {
    match dir {
        Direction::In => Some((1, 0)),
        Direction::Out => Some((-1, 0)),
        Direction::Unknown => None,
        d => layout_offset(d),
    }
}

/// Every pair of rooms in `positions` joined by a CARDINAL RECIPROCAL passage that currently sits
/// exactly one cell apart, in the direction the passage claims.
///
/// The one invariant [`seat_adjacent`] will not trade away. Keyed by the ordered pair so the set
/// is comparable across a trial shift; a pair whose rooms are already further apart than the
/// passage claims is not in here at all, and so cannot be made worse by a shift either.
///
/// **CARDINAL means cardinal: a DIAGONAL claims no cell count** (SQ-1375). `N` names the room in
/// the next cell up; `NW` only ever pinned its endpoint to a QUADRANT, and stretching one is the
/// layout's ordinary currency — which is exactly what `layout::edge_is_satisfied` decided at
/// SQ-1364 (`Some((dx, dy)) if dx == 0 || dy == 0`), what `mark_distorted` reads, and what this
/// module's own docs have said since SQ-1356 ("a passage that is one-way, diagonal, gated or
/// already stretched may lengthen"). The filter below is the line that had been missing: without
/// it `grid_offset` answers `Some` for all eight compass points, so a diagonal was silently as
/// binding here as a cardinal and vetoed shifts the rest of the mapper considers free.
///
/// Zork I's `Attic` is the specimen. It hangs off the `Kitchen` by a staircase and wanted the cell
/// directly above it, where `North of House` stands. Opening that row would have moved
/// `North of House` off the DIAGONAL it sits on from `Behind House` — the repair edge the layout
/// itself wrote when the walked `E` came out distorted — and that one diagonal vetoed the whole
/// slide, so the `Attic` walked four cells out of the house and was drawn past `North of House`,
/// `Forest Path` and `Clearing`, in a column of its own. With the diagonal free to stretch the row
/// opens, every cardinal pair on the map keeps its cell, and the `Attic` sits on the doorstep.
fn adjacent_reciprocals(
    graph: &MapGraph,
    positions: &BTreeMap<RoomId, (i32, i32)>,
) -> BTreeSet<(RoomId, RoomId)> {
    let conns = graph.connections();
    let mut out = BTreeSet::new();
    for c in conns {
        if c.is_self_loop() {
            continue;
        }
        let Some(off) = grid_offset(c.dir).filter(|&(dx, dy)| dx == 0 || dy == 0) else { continue };
        let (Some(&a), Some(&b)) = (positions.get(&c.origin), positions.get(&c.dest)) else {
            continue;
        };
        if (b.0 - a.0, b.1 - a.1) != off {
            continue;
        }
        if !conns.iter().any(|o| o.origin == c.dest && o.dest == c.origin) {
            continue; // one-way: no cell-count claim
        }
        out.insert((c.origin, c.dest));
    }
    out
}

/// Slide every room at or beyond `line` on one axis by `step`, inserting a blank row/column.
fn open_line(positions: &mut BTreeMap<RoomId, (i32, i32)>, horizontal: bool, line: i32, step: i32) {
    for p in positions.values_mut() {
        let v = if horizontal { p.0 } else { p.1 };
        let beyond = if step > 0 { v >= line } else { v <= line };
        if beyond {
            if horizontal {
                p.0 += step;
            } else {
                p.1 += step;
            }
        }
    }
}

/// The most cells a chain push will move rooms out of (SQ-1358).
///
/// A late arrival is worth a nudge to the rooms standing in front of it, not a migration of a
/// dense column across the whole map: past a few cells the map has plainly told you the bearing
/// is full, and the newcomer is better off beside its anchor.
const MAX_PUSH_CHAIN: usize = 4;

/// The most rooms one push may move once the DEPENDANTS are counted in (SQ-1363).
///
/// [`MAX_PUSH_CHAIN`] bounds the column the newcomer is pushing THROUGH, which is a line; this
/// bounds the whole party that ends up travelling, which a dependant closure can spread across
/// several columns and so needs a count of its own. Eight is a house and its doorstep — Zork I's
/// case moves two — and past it a late arrival is shunting a neighbourhood rather than asking a
/// room to step aside, which is the point [`MAX_PUSH_CHAIN`] already makes about depth.
const MAX_PUSH_SET: usize = 8;

/// Does a passage running `dir` say the room at its far end HANGS OFF the moved room, on the far
/// side of `bearing`? (SQ-1363)
///
/// `from_moved` says which end was walked: `true` when the moved room owns the passage
/// (`moved --dir--> candidate`, so the candidate lies `dir`-ward of it), `false` when the
/// candidate does (`candidate --dir--> moved`, so the moved room lies `dir`-ward and the
/// candidate lies back the other way).
///
/// **Read off the PASSAGE'S DIRECTION, not off reciprocity and not off the cells.** The specimen
/// is exactly why: Zork I's `South of House --S--> Forest` is answered by
/// `Forest --NW--> South of House`, so the two are not a reciprocal pair in the sense
/// [`adjacent_reciprocals`] means at all — and yet both halves say the same thing to a reader,
/// that the forest lies south of the house. A rule keyed on reciprocity drops this case, which is
/// the whole case; a rule keyed on the direction keeps it, and keeps every honest one-way exit
/// with it. A neighbour to the SIDE (`E`, `W` against a downward bearing, dot product zero) or
/// BEHIND (`N` out of the moved room) is anchored by whatever is over there, and is not a
/// dependant.
///
/// **Not a dependant is not the same as left behind** (SQ-1375). A side neighbour whose adjacency
/// the map is currently PROMISING — a cardinal reciprocal exactly one cell away — travels with the
/// party all the same, because leaving it would break that promise; [`stranded_partners`] is where
/// that half of the closure lives, and it reads the cells rather than the directions, because
/// "would this move break a kept adjacency" is a question about geometry. This function stays a
/// question about what the passages SAY, which is why a one-way `S` exit to a room two cells
/// diagonally away still drags it along.
fn hangs_off(dir: Direction, from_moved: bool, bearing: (i32, i32)) -> bool {
    let Some(v) = grid_offset(dir) else { return false };
    let (dx, dy) = if from_moved { v } else { (-v.0, -v.1) };
    dx * bearing.0 + dy * bearing.1 > 0
}

/// Grow `moving` with every placed room that [`hangs_off`] one of its members, until it stops
/// growing (SQ-1363).
///
/// Transitive: a room that hangs off a dependant hangs off the push, and a chain of two is not
/// rarer than a chain of one.
///
/// One of the two halves of the party closure; [`stranded_partners`] is the other, and
/// [`settle_party`] alternates them because each can hand the other new members.
fn extend_with_dependants(
    graph: &MapGraph,
    positions: &BTreeMap<RoomId, (i32, i32)>,
    moving: &mut BTreeSet<RoomId>,
    bearing: (i32, i32),
) {
    let conns = graph.connections();
    loop {
        let mut found: Vec<RoomId> = Vec::new();
        for c in conns {
            if c.is_self_loop() {
                continue;
            }
            let (other, from_moved) = match (moving.contains(&c.origin), moving.contains(&c.dest)) {
                (true, false) => (c.dest, true),
                (false, true) => (c.origin, false),
                _ => continue,
            };
            if positions.contains_key(&other) && hangs_off(c.dir, from_moved, bearing) {
                found.push(other);
            }
        }
        let before = moving.len();
        moving.extend(found);
        if moving.len() == before {
            return;
        }
    }
}

/// The COLUMN a push moves through: whatever stands on `want`, whatever stands behind THAT along
/// the bearing, and so on until the bearing reaches open ground (SQ-1358).
///
/// `None` when it does not reach open ground inside [`MAX_PUSH_CHAIN`] cells — the bearing is
/// plainly full and nobody is stepping aside.
fn push_column(
    positions: &BTreeMap<RoomId, (i32, i32)>,
    want: (i32, i32),
    offset: (i32, i32),
) -> Option<BTreeSet<RoomId>> {
    let mut cell = want;
    let mut column: BTreeSet<RoomId> = BTreeSet::new();
    for _ in 0..MAX_PUSH_CHAIN {
        let here: Vec<RoomId> =
            positions.iter().filter(|(_, &p)| p == cell).map(|(&r, _)| r).collect();
        if here.is_empty() {
            return Some(column);
        }
        column.extend(here);
        cell = (cell.0 + offset.0, cell.1 + offset.1);
    }
    None
}

/// Every room in `positions` that is half of a `keep` pair whose OTHER half is already moving
/// (SQ-1375).
///
/// A push moves its party as a rigid body, so a kept adjacency survives it exactly when both ends
/// travel. [`apply_push`] reads that as a veto — the partner was left behind, so refuse — and this
/// reads the same fact as an instruction: bring the partner. Which of the two applies is settled
/// by whether the partner CAN come, and [`apply_push`] still has the last word on that.
fn stranded_partners(
    positions: &BTreeMap<RoomId, (i32, i32)>,
    keep: &BTreeSet<(RoomId, RoomId)>,
    moving: &BTreeSet<RoomId>,
) -> Vec<RoomId> {
    keep.iter()
        .filter_map(|&(a, b)| match (moving.contains(&a), moving.contains(&b)) {
            (true, false) => Some(b),
            (false, true) => Some(a),
            _ => None,
        })
        .filter(|r| positions.contains_key(r))
        .collect()
}

/// The whole party `column` ends up dragging: its dependants ([`extend_with_dependants`]), the
/// partners its kept adjacencies would otherwise strand ([`stranded_partners`]), whatever ALL of
/// them would displace, and their dependants and partners in turn — repeated until no pass adds
/// anybody (SQ-1363, SQ-1375).
///
/// **A kept adjacency is a reason to bring a room, not a reason to stop** (SQ-1375). SQ-1363 swept
/// in the rooms that hang off the party along the bearing and left the ones to the SIDE of it
/// where they stood — and a side neighbour joined by a cardinal reciprocal is precisely the shape
/// [`apply_push`] refuses, so the push died on a room that would have been perfectly happy to step
/// across with everybody else. That is the user's rule stated once: when a room gets pushed, the
/// rooms that depend on it get pushed the same way, and "depends on" covers the neighbour whose
/// adjacency the map is promising to keep as much as it covers the room downwind. Zork I's house
/// is the specimen — `Forest Path` cannot be nudged north without `Forest`, due west of it,
/// coming along.
///
/// The anchor is the one room that can never join: it is where the newcomer is being seated
/// FROM. Nothing here says so, because nothing here has to — the anchor stands one cell back
/// along the bearing, so a party containing it lands on `want`, and [`apply_push`]'s check that
/// `want` comes out free refuses that party without a special case for it (see [`push_chain`]).
///
/// `None` when the party outgrows [`MAX_PUSH_SET`] rooms, at which point the push is shunting a
/// neighbourhood rather than asking a room to step aside.
fn settle_party(
    graph: &MapGraph,
    positions: &BTreeMap<RoomId, (i32, i32)>,
    keep: &BTreeSet<(RoomId, RoomId)>,
    column: &BTreeSet<RoomId>,
    offset: (i32, i32),
) -> Option<BTreeSet<RoomId>> {
    let mut moving = column.clone();
    loop {
        if moving.len() > MAX_PUSH_SET {
            return None;
        }
        let before = moving.len();
        extend_with_dependants(graph, positions, &mut moving, offset);
        let partners = stranded_partners(positions, keep, &moving);
        moving.extend(partners);
        let ahead: BTreeSet<(i32, i32)> = moving
            .iter()
            .filter_map(|r| positions.get(r))
            .map(|p| (p.0 + offset.0, p.1 + offset.1))
            .collect();
        let displaced: Vec<RoomId> = positions
            .iter()
            .filter(|(r, &p)| !moving.contains(r) && ahead.contains(&p))
            .map(|(&r, _)| r)
            .collect();
        moving.extend(displaced);
        if moving.len() == before {
            return Some(moving);
        }
    }
}

/// Move `moving` one cell along `offset` as a rigid body, if the result is legal.
///
/// `Some` with the pushed positions when every cardinal-reciprocal pair in `keep` (the map's set
/// BEFORE the push) is still adjacent after, so a partner either travelled with the party or was
/// never adjacent to begin with; `None` when a partner would be left behind, or when the party
/// lands back on `want` — the very cell the newcomer is being seated in.
fn apply_push(
    graph: &MapGraph,
    positions: &BTreeMap<RoomId, (i32, i32)>,
    keep: &BTreeSet<(RoomId, RoomId)>,
    moving: &BTreeSet<RoomId>,
    want: (i32, i32),
    offset: (i32, i32),
) -> Option<BTreeMap<RoomId, (i32, i32)>> {
    let mut trial = positions.clone();
    for (r, p) in trial.iter_mut() {
        if moving.contains(r) {
            *p = (p.0 + offset.0, p.1 + offset.1);
        }
    }
    if trial.values().any(|&p| p == want) {
        return None; // the party filled the very cell the newcomer is being seated in
    }
    keep.is_subset(&adjacent_reciprocals(graph, &trial)).then_some(trial)
}

/// Push whatever stands on `want` — whatever THAT would displace, whatever hangs off any of them,
/// and whatever kept adjacency they would otherwise strand — one cell further along `offset`, as a
/// rigid chain (SQ-1358, SQ-1363, SQ-1367, SQ-1375).
///
/// [`push_column`] walks the bearing to open ground, [`settle_party`] grows that column into the
/// party which must travel with it, and [`apply_push`] moves the party and judges the result. The
/// whole party moves by `offset` together, so each moved room keeps its exact offset from every
/// other moved room.
///
/// **The dependants are how a push TRAVELS, never a reason not to push** (SQ-1367). A party that
/// is oversized or vetoed falls back to the bare COLUMN — exactly the push SQ-1358 shipped —
/// rather than abandoning the seating, because the alternative to pushing is not "nothing moves":
/// it is the newcomer seated somewhere else entirely, its line looping around a room the passage
/// never touches. Zork I's Maze is the specimen. One room, `Maze #169`, stood on the cell the
/// `Clearing` ghost's `Down` wanted, and asking it to step one cell up was legal and correct — but
/// a maze's passages point every which way, so the transitive dependant closure swallowed nine
/// more rooms, blew [`MAX_PUSH_SET`] and refused the push outright. The ghost fell to the
/// perpendicular side, and the two adjacent boxes that portal joins were drawn with four turns of
/// line between them. Both attempts face the same veto, so the fallback can no more pull an
/// adjacent reciprocal pair apart than the widened party could.
///
/// **Two rungs, not three** (SQ-1375). The partner closure only ever adds a room whose kept
/// adjacency the smaller party would have broken — and a party that breaks a kept adjacency is a
/// party [`apply_push`] refuses. So wherever the widened party differs from SQ-1363's, SQ-1363's
/// was already vetoed, and an intermediate rung between the two would be dead code. The fallback
/// that matters is still the bare COLUMN.
///
/// `None` when neither party is legal, or when the column never reaches open ground.
///
/// The anchor stands one cell BACK from `want`, against the bearing — but a DISTORTED passage may
/// claim a bearing its cells do not bear out, so the anchor can be swept into the party by a rule
/// that reads directions rather than positions, and since SQ-1375 by one that reads the kept
/// adjacencies too: the anchor's own eastern neighbour joining the party drags the anchor in
/// behind it. [`apply_push`]'s check that `want` ends up free says so rather than assuming it, and
/// is the only guard the anchor needs — the party lands ON `want` the moment the anchor is in it.
/// Zork I's `Studio` ghost takes that route every run: `Behind House` is a cardinal neighbour of
/// the `Kitchen` it is being seated against, so the closure reaches the anchor, the party is
/// refused, and the bare column — `South of House` alone — is what actually steps aside.
fn push_chain(
    graph: &MapGraph,
    positions: &BTreeMap<RoomId, (i32, i32)>,
    keep: &BTreeSet<(RoomId, RoomId)>,
    want: (i32, i32),
    offset: (i32, i32),
) -> Option<BTreeMap<RoomId, (i32, i32)>> {
    let column = push_column(positions, want, offset)?;
    settle_party(graph, positions, keep, &column, offset)
        .and_then(|party| apply_push(graph, positions, keep, &party, want, offset))
        .or_else(|| apply_push(graph, positions, keep, &column, want, offset))
}

/// Where a late-arriving room seats itself relative to `anchor`, and what the map had to do to
/// let it (SQ-1356).
///
/// The three travel together because a caller that reads the cell without knowing whether the
/// map moved under it would draw the newcomer against stale neighbours: `positions` is mutated
/// in place when `moved_map` is set, and every cell in it — the anchor's included — must be
/// re-read afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seat {
    /// The cell the newcomer takes. Free in `positions` on return.
    pub cell: (i32, i32),
    /// True when the wanted cell was already free and nothing moved.
    pub direct: bool,
    /// True when the map had to move to make room — a blank line opened, or the blocker pushed
    /// along the bearing (SQ-1358). Every other room may have shifted.
    pub moved_map: bool,
}

/// Seat a late-arriving room in the cell adjacent to `anchor` along `offset` (SQ-1356).
///
/// `positions` is every room already standing on this plane — one layer's rooms, plus whatever
/// earlier newcomers have already been seated — and the newcomer itself must NOT be in it. On
/// return the chosen cell is free in `positions`; the caller inserts the newcomer there.
///
/// Five outcomes, in order:
///
/// 1. the wanted cell is free and is taken;
/// 2. it is not, and a blank line is opened at it by sliding that whole side of the map one cell
///    further out — accepted only when every cardinal-reciprocal pair that was exactly adjacent
///    still is (see the module docs). A diagonal `offset` may open EITHER of its two lines, so
///    both are tried, the horizontal one first;
/// 3. the slide is vetoed, so the blocker alone is asked to step aside: it moves one cell further
///    along the bearing, and whatever it would then displace — plus everything that HANGS OFF any
///    of them, so a pushed room does not leave its own southern neighbours behind (SQ-1363), plus
///    every SIDE neighbour whose kept adjacency the move would otherwise strand (SQ-1375) —
///    moves with it as a rigid chain, up to [`MAX_PUSH_CHAIN`] cells deep and [`MAX_PUSH_SET`]
///    rooms wide — under the same veto, and the newcomer still gets the
///    cell it wanted. This is what a slide cannot do, because a slide is all-or-nothing: one
///    faraway pair straddling the cut vetoes it for the whole side, however free the blocker
///    itself is (SQ-1358 — Zork I's `South of House`, see the module docs);
/// 4. neither, and the newcomer takes a free cell still ADJACENT to the anchor, off the bearing:
///    the two sides perpendicular to it first, then the side opposite. That is what a real room
///    arriving to find its slot taken gets — a neighbour with a one-bend line — and it beats
///    step 5 by a distance: a newcomer pushed PAST the room blocking its bearing lands on the
///    far side of it, so the line has to be routed all the way around a room that has nothing to
///    do with the passage, and the two boxes it does join are no longer neighbours at all;
/// 5. only then does it walk out along the bearing to the first free cell, which is where a
///    genuinely boxed-in anchor ends up.
///
/// **The perpendicular sides are tried roomier-first.** Both are equally correct as geometry —
/// the passage is a portal or a crossing, so neither side is the direction anything was walked —
/// and the one with more free cells around it is the one whose line has somewhere to go and
/// whose box has room to be read. Ties keep the left-hand rotation, so the choice stays
/// deterministic.
///
/// `None` when `anchor` is not in `positions` (nothing to seat against).
pub fn seat_adjacent(
    graph: &MapGraph,
    positions: &mut BTreeMap<RoomId, (i32, i32)>,
    anchor: RoomId,
    offset: (i32, i32),
) -> Option<Seat> {
    let a = *positions.get(&anchor)?;
    let want = (a.0 + offset.0, a.1 + offset.1);
    let taken = |p: &BTreeMap<RoomId, (i32, i32)>, c: (i32, i32)| p.values().any(|&q| q == c);
    if !taken(positions, want) {
        return Some(Seat { cell: want, direct: true, moved_map: false });
    }

    let keep = adjacent_reciprocals(graph, positions);
    for (horizontal, step) in [(true, offset.0), (false, offset.1)] {
        if step == 0 {
            continue;
        }
        let line = if horizontal { want.0 } else { want.1 };
        let mut trial = positions.clone();
        open_line(&mut trial, horizontal, line, step);
        // The anchor sits one cell back from `line` on the stationary side, so it never moves and
        // the cell it wanted is now empty — but say so rather than assume it.
        if trial.get(&anchor) != Some(&a) || taken(&trial, want) {
            continue;
        }
        if keep.is_subset(&adjacent_reciprocals(graph, &trial)) {
            *positions = trial;
            return Some(Seat { cell: want, direct: false, moved_map: true });
        }
    }

    // No line may open, but the blocker itself may still be free to step aside: push it — and
    // whatever it would displace — one cell along the bearing, under the same veto.
    if let Some(trial) = push_chain(graph, positions, &keep, want, offset) {
        *positions = trial;
        return Some(Seat { cell: want, direct: false, moved_map: true });
    }

    // Nothing legal to move: stay ADJACENT to the anchor instead of going round the blocker.
    // Perpendicular sides first, roomier one first; then the side opposite the bearing.
    let free_neighbours = |c: (i32, i32)| {
        [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .into_iter()
            .filter(|(dx, dy)| !taken(positions, (c.0 + dx, c.1 + dy)))
            .count()
    };
    let mut sides = vec![(-offset.1, offset.0), (offset.1, -offset.0)];
    sides.sort_by_key(|&d| std::cmp::Reverse(free_neighbours((a.0 + d.0, a.1 + d.1))));
    sides.push((-offset.0, -offset.1));
    for d in sides {
        let c = (a.0 + d.0, a.1 + d.1);
        if c != want && !taken(positions, c) {
            return Some(Seat { cell: c, direct: false, moved_map: false });
        }
    }

    // Boxed in on every side: walk out along the bearing. Each step lands on a distinct cell, so
    // one more step than there are rooms is guaranteed to find a free one.
    let mut c = want;
    for _ in 0..=positions.len() {
        c = (c.0 + offset.0, c.1 + offset.1);
        if !taken(positions, c) {
            return Some(Seat { cell: c, direct: false, moved_map: false });
        }
    }
    Some(Seat { cell: c, direct: false, moved_map: false })
}

/// Re-seat every PORTAL-ONLY room onto its anchor's doorstep (SQ-1356).
///
/// A room whose every passage is Up/Down/In/Out has no compass bearing for the solve to place it
/// by, so it falls out of the stress stage wherever the collision pass could fit it — Zork I's
/// `Cellar` hanging off the `Living Room`'s `Down` came out across the map from it. This pass
/// runs last, after every room WITH a bearing has settled, and asks [`seat_adjacent`] for the
/// cell the portal itself points at.
///
/// Only rooms that are not already there move, and only within their own layer: a portal that
/// crosses a layer is that layer's ghost's business (`crate::render::render_layer`), not a claim
/// on a cell here.
pub fn seat_portal_leaves(graph: &MapGraph, final_pos: &mut BTreeMap<RoomId, (i32, i32)>) {
    let conns = graph.connections();
    // The rooms to seat, and the anchor + offset each wants: the lowest-id partner it shares a
    // portal with, so the pass is deterministic whatever order the passages were walked in.
    let mut wanted: Vec<(RoomId, RoomId, (i32, i32))> = Vec::new();
    for room in graph.rooms() {
        let mine: Vec<_> = conns
            .iter()
            .filter(|c| !c.is_self_loop() && (c.origin == room.id || c.dest == room.id))
            .collect();
        if mine.is_empty() || mine.iter().any(|c| grid_offset(c.dir).is_some()) {
            continue; // no passages at all, or at least one real compass bearing
        }
        let mut cands: Vec<(RoomId, (i32, i32))> = mine
            .iter()
            .filter_map(|c| {
                let (partner, out) = if c.origin == room.id {
                    // `room --dir--> partner`: the partner sits `dir`-ward, so `room` sits back.
                    (c.dest, seat_offset(crate::direction::opposite(c.dir))?)
                } else {
                    (c.origin, seat_offset(c.dir)?)
                };
                (graph.layer_of(partner) == room.layer && partner != room.id)
                    .then_some((partner, out))
            })
            .collect();
        cands.sort();
        if let Some(&(anchor, off)) = cands.first() {
            wanted.push((room.id, anchor, off));
        }
    }
    wanted.sort();

    for (id, anchor, off) in wanted {
        let layer = graph.layer_of(id);
        let mut plane: BTreeMap<RoomId, (i32, i32)> = final_pos
            .iter()
            .filter(|(&r, _)| r != id && graph.layer_of(r) == layer)
            .map(|(&r, &p)| (r, p))
            .collect();
        let Some(&a) = plane.get(&anchor) else { continue };
        if final_pos.get(&id) == Some(&(a.0 + off.0, a.1 + off.1)) {
            continue; // already on the doorstep: leave the map exactly as it was
        }
        let Some(seat) = seat_adjacent(graph, &mut plane, anchor, off) else { continue };
        for (r, p) in plane {
            final_pos.insert(r, p);
        }
        final_pos.insert(id, seat.cell);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;

    fn g_with(rooms: &[(RoomId, &str)], edges: &[(RoomId, Direction, RoomId)]) -> MapGraph {
        let mut g = MapGraph::new();
        for &(id, n) in rooms {
            g.upsert_room(id, n.into());
        }
        for &(a, d, b) in edges {
            g.add_edge(a, d, b);
        }
        g
    }

    #[test]
    fn a_free_doorstep_is_taken_and_nothing_moves() {
        let g = g_with(&[(1, "Hall"), (2, "Study")], &[(1, Direction::E, 2), (2, Direction::W, 1)]);
        let mut pos: BTreeMap<RoomId, (i32, i32)> = [(1, (0, 0)), (2, (1, 0))].into_iter().collect();
        let seat = seat_adjacent(&g, &mut pos, 1, (0, 1)).unwrap();
        assert_eq!(seat.cell, (0, 1));
        assert!(seat.direct);
        assert_eq!(pos[&2], (1, 0), "nothing moved");
    }

    /// The occupied doorstep opens: the row below slides one further down, and the newcomer takes
    /// the cell. The two rooms that straddle the cut are joined ONE WAY, so no cell count is
    /// claimed and the shift is legal.
    #[test]
    fn an_occupied_doorstep_opens_a_line() {
        let g = g_with(
            &[(1, "Hall"), (2, "Below"), (3, "Beside")],
            &[(1, Direction::Down, 9), (2, Direction::E, 3), (3, Direction::W, 2)],
        );
        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            [(1, (0, 0)), (2, (0, 1)), (3, (1, 1))].into_iter().collect();
        let seat = seat_adjacent(&g, &mut pos, 1, (0, 1)).unwrap();
        assert_eq!(seat.cell, (0, 1), "the newcomer takes the cell it wanted");
        assert!(seat.moved_map);
        assert_eq!(pos[&1], (0, 0), "the anchor never moves");
        assert_eq!((pos[&2], pos[&3]), ((0, 2), (1, 2)), "the row below slid, keeping its own shape");
    }

    /// …but not when opening it would pull a cardinal-reciprocal pair apart: `1`–`2` is walked
    /// both ways, so the line stays shut and the newcomer stays ADJACENT to its anchor, on a side
    /// perpendicular to the bearing — never past `2`, which would put the blocker between the two
    /// boxes the passage joins.
    #[test]
    fn a_blocked_bearing_falls_back_to_a_neighbour_never_past_the_blocker() {
        let g = g_with(
            &[(1, "Hall"), (2, "Below")],
            &[(1, Direction::S, 2), (2, Direction::N, 1), (1, Direction::Down, 9)],
        );
        let mut pos: BTreeMap<RoomId, (i32, i32)> = [(1, (0, 0)), (2, (0, 1))].into_iter().collect();
        let seat = seat_adjacent(&g, &mut pos, 1, (0, 1)).unwrap();
        assert_eq!(seat.cell, (-1, 0), "beside the anchor, perpendicular to the blocked bearing");
        assert_ne!(seat.cell, (0, 2), "never on the far side of the room that blocked it");
        assert!(!seat.moved_map);
        assert_eq!(pos[&2], (0, 1), "and the pair is still exactly one cell apart");
        let (ax, ay) = pos[&1];
        assert!(
            (seat.cell.0 - ax).abs() <= 1 && (seat.cell.1 - ay).abs() <= 1,
            "still on the anchor's own doorstep: {:?} vs {:?}",
            seat.cell,
            (ax, ay)
        );
    }

    /// Both perpendicular sides are free, so the ROOMIER one wins — the side whose own
    /// neighbourhood has somewhere for the line to go and room for the box to be read. Here east
    /// of the anchor is walled in by `3` and `4`, so the newcomer goes west.
    #[test]
    fn the_roomier_perpendicular_side_wins() {
        let g = g_with(
            &[(1, "Hall"), (2, "Below"), (3, "East"), (4, "Corner")],
            &[(1, Direction::S, 2), (2, Direction::N, 1), (1, Direction::Down, 9)],
        );
        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            [(1, (0, 0)), (2, (0, 1)), (3, (2, 0)), (4, (1, -1))].into_iter().collect();
        let seat = seat_adjacent(&g, &mut pos, 1, (0, 1)).unwrap();
        assert_eq!(seat.cell, (-1, 0), "west: east is hemmed in on two more sides");
    }

    // ── SQ-1358: the blocker itself steps aside ────────────────────────────────

    /// Every room of [`house`] but `South of House`, at the cells none of these cases may move it
    /// from: the whole point is that ONE room steps aside, not that the map rearranges itself.
    const HOUSE_AT_REST: [(RoomId, (i32, i32)); 6] =
        [(1, (-1, 0)), (2, (0, 0)), (3, (1, 0)), (4, (0, -1)), (6, (2, 0)), (7, (2, 1))];

    /// Zork I's house, reduced to its shape: a three-wide row (`1`–`2`–`3`), an `Attic` above the
    /// middle, `South of House` below it, and a `Clearing`/`Forest` pair further east that is
    /// walked north/south across the same row. The Studio's stairs arrive from BELOW the middle.
    ///
    /// The whole-side slide is vetoed by the Clearing/Forest pair — a pair with nothing to do with
    /// the house — but `South of House` is joined to the house only by distorted passages, so it
    /// is half of no adjacent pair and can step down a cell alone. The house grows a row.
    fn house(
        joined_to_the_row: &[(RoomId, Direction, RoomId)],
    ) -> (MapGraph, BTreeMap<RoomId, (i32, i32)>) {
        let mut edges = vec![
            (1, Direction::E, 2),
            (2, Direction::W, 1),
            (2, Direction::E, 3),
            (3, Direction::W, 2),
            // The Attic hangs off the Kitchen by a staircase: Up/Down claim no cell count.
            (2, Direction::Up, 4),
            (4, Direction::Down, 2),
            // The pair that vetoes the slide, walked both ways across the cut at y = 1.
            (6, Direction::S, 7),
            (7, Direction::N, 6),
        ];
        edges.extend_from_slice(joined_to_the_row);
        let g = g_with(
            &[
                (1, "Living Room"),
                (2, "Kitchen"),
                (3, "Behind House"),
                (4, "Attic"),
                (5, "South of House"),
                (6, "Clearing"),
                (7, "Forest"),
            ],
            &edges,
        );
        let pos = HOUSE_AT_REST.into_iter().chain([(5, (0, 1))]).collect();
        (g, pos)
    }

    #[test]
    fn a_lone_blocker_steps_aside_and_the_house_grows_a_row() {
        // Every passage `South of House` owns is distorted — it sits a cell away diagonally from
        // each of the rooms it names, so none of them is an adjacent pair.
        let (g, mut pos) = house(&[
            (5, Direction::W, 1),
            (1, Direction::S, 5),
            (5, Direction::E, 3),
            (3, Direction::S, 5),
        ]);
        let seat = seat_adjacent(&g, &mut pos, 2, (0, 1)).unwrap();
        assert_eq!(seat.cell, (0, 1), "the ghost took the doorstep it wanted");
        assert!(seat.moved_map);
        assert_eq!(pos[&5], (0, 2), "South of House stepped down a row by itself");
        for (id, cell) in HOUSE_AT_REST {
            assert_eq!(pos[&id], cell, "room {id} did not move");
        }
    }

    /// …but a blocker that is half of an adjacent walked pair is not pushed: its partner stands
    /// beside the chain rather than travelling in it, so the push would pull the two apart. The
    /// seating falls through to the old answer — past the blocker, along the bearing.
    #[test]
    fn a_blocker_with_a_walked_partner_is_not_pushed() {
        let (g, mut pos) = house(&[(2, Direction::S, 5), (5, Direction::N, 2)]);
        let seat = seat_adjacent(&g, &mut pos, 2, (0, 1)).unwrap();
        assert!(!seat.moved_map, "nothing legal to move");
        assert_eq!(seat.cell, (0, 2), "past the blocker, exactly as before SQ-1358");
        assert_eq!(pos[&5], (0, 1), "and the walked pair is still exactly one cell apart");
        for (id, cell) in HOUSE_AT_REST {
            assert_eq!(pos[&id], cell, "room {id} did not move");
        }
    }

    /// The chain is RIGID: the blocker's own blocker travels with it, so a pair inside the chain
    /// keeps its offset and only the far end of the column meets open ground.
    #[test]
    fn a_pushed_blocker_takes_the_room_behind_it_with_it() {
        let g = g_with(
            &[(1, "Hall"), (2, "Below"), (3, "Further Below"), (6, "Clearing"), (7, "Forest")],
            &[
                (2, Direction::S, 3),
                (3, Direction::N, 2),
                (6, Direction::S, 7),
                (7, Direction::N, 6),
            ],
        );
        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            [(1, (0, 0)), (2, (0, 1)), (3, (0, 2)), (6, (2, 0)), (7, (2, 1))].into_iter().collect();
        let seat = seat_adjacent(&g, &mut pos, 1, (0, 1)).unwrap();
        assert_eq!(seat.cell, (0, 1));
        assert!(seat.moved_map);
        assert_eq!((pos[&2], pos[&3]), ((0, 2), (0, 3)), "both moved, keeping their own offset");
        assert_eq!((pos[&1], pos[&6], pos[&7]), ((0, 0), (2, 0), (2, 1)), "and nothing else did");
    }

    /// A column deeper than [`MAX_PUSH_CHAIN`] is not pushed at all: past a few cells the bearing
    /// is plainly full, and the newcomer is better off beside its anchor than shunting the map.
    #[test]
    fn a_chain_longer_than_the_cap_is_refused() {
        let g = g_with(
            &[(1, "Hall"), (6, "Clearing"), (7, "Forest")],
            &[(6, Direction::S, 7), (7, Direction::N, 6)],
        );
        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            [(1, (0, 0)), (6, (2, 0)), (7, (2, 1))].into_iter().collect();
        for (n, id) in (11..).take(MAX_PUSH_CHAIN + 1).enumerate() {
            pos.insert(id, (0, n as i32 + 1));
        }
        let seat = seat_adjacent(&g, &mut pos, 1, (0, 1)).unwrap();
        assert!(!seat.moved_map, "the column stayed put");
        assert_eq!(seat.cell, (-1, 0), "beside the anchor instead");
        assert_eq!(pos[&11], (0, 1), "the blocker never budged");
    }

    // ── SQ-1363: a pushed room takes what hangs off it ─────────────────────────

    /// The reported case, reduced: the Studio's ghost pushes `South of House` down out of its way,
    /// and the `Forest` that room's own `S` exit leads to goes down with it — so the forest is
    /// still drawn below the house afterwards, which is the only thing the passage says.
    ///
    /// The pair is deliberately NOT reciprocal in the strict sense: `5 --S--> 8` is answered by
    /// `8 --NW--> 5`, exactly as Zork I walks it. A rule keyed on reciprocity would leave `8`
    /// behind; [`hangs_off`] reads the direction instead.
    #[test]
    fn a_pushed_blocker_takes_the_room_hanging_off_its_south_exit() {
        let (g, mut pos) = house(&[
            (5, Direction::W, 1),
            (1, Direction::S, 5),
            (5, Direction::E, 3),
            (3, Direction::S, 5),
            (5, Direction::S, 8),
            (8, Direction::NW, 5),
        ]);
        pos.insert(8, (2, 2));
        let seat = seat_adjacent(&g, &mut pos, 2, (0, 1)).unwrap();
        assert_eq!(seat.cell, (0, 1), "the ghost took the doorstep it wanted");
        assert!(seat.moved_map);
        assert_eq!(pos[&5], (0, 2), "South of House stepped down a row");
        assert_eq!(pos[&8], (2, 3), "and the Forest hanging off its S exit came with it");
        for (id, cell) in HOUSE_AT_REST {
            assert_eq!(pos[&id], cell, "room {id} did not move");
        }
    }

    /// Dependence is transitive: a room hanging off the dependant travels too, and the whole party
    /// keeps its own shape.
    #[test]
    fn a_chain_of_two_dependants_all_move() {
        let (g, mut pos) = house(&[
            (5, Direction::W, 1),
            (1, Direction::S, 5),
            (5, Direction::S, 8),
            (8, Direction::NW, 5),
            (8, Direction::SE, 9),
            (9, Direction::N, 8),
        ]);
        pos.insert(8, (2, 2));
        pos.insert(9, (3, 3));
        let seat = seat_adjacent(&g, &mut pos, 2, (0, 1)).unwrap();
        assert_eq!(seat.cell, (0, 1));
        assert!(seat.moved_map);
        assert_eq!(
            (pos[&5], pos[&8], pos[&9]),
            ((0, 2), (2, 3), (3, 4)),
            "the blocker and both dependants moved one row, keeping their own offsets"
        );
        for (id, cell) in HOUSE_AT_REST {
            assert_eq!(pos[&id], cell, "room {id} did not move");
        }
    }

    // ── SQ-1375: diagonals claim no cell, and a kept pair travels ──────────────

    /// **A DIAGONAL reciprocal claims no cell count, so it never vetoes a slide.** `6`/`7` sit one
    /// diagonal step apart across the cut the newcomer needs opened. Were that binding — as
    /// [`adjacent_reciprocals`] wrongly made it until SQ-1375 — the row could not open, and the
    /// newcomer would be pushed past its blocker or parked beside its anchor; that is exactly what
    /// happened to Zork I's `Attic`. A cardinal pair in the same position still vetoes, which is
    /// what [`a_blocked_bearing_falls_back_to_a_neighbour_never_past_the_blocker`] pins.
    #[test]
    fn a_diagonal_reciprocal_does_not_veto_a_slide() {
        let g = g_with(
            &[(1, "Hall"), (2, "Below"), (6, "Clearing"), (7, "Forest")],
            &[(6, Direction::SE, 7), (7, Direction::NW, 6)],
        );
        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            [(1, (0, 0)), (2, (0, 1)), (6, (2, 0)), (7, (3, 1))].into_iter().collect();
        let seat = seat_adjacent(&g, &mut pos, 1, (0, 1)).unwrap();
        assert_eq!(seat.cell, (0, 1), "the newcomer takes the cell it wanted");
        assert!(seat.moved_map, "the row opened");
        assert_eq!(pos[&1], (0, 0), "the anchor never moves");
        assert_eq!((pos[&2], pos[&7]), ((0, 2), (3, 2)), "the row below slid, diagonal and all");
        assert_eq!(pos[&6], (2, 0), "and the far half of the diagonal stayed where it was");
    }

    // ── SQ-1375: a kept adjacency travels with the party ───────────────────────

    /// **The boundary SQ-1363 drew, moved.** A neighbour due EAST of the blocker is perpendicular
    /// to the bearing, so [`hangs_off`] says nothing about it and it is not a dependant — and up
    /// to SQ-1375 that meant it stood still, the E/W pair was pulled apart, and the push was
    /// vetoed outright: the newcomer went PAST the blocker and the two boxes its passage joins
    /// stopped being neighbours.
    ///
    /// The user's rule says the opposite. A room the map is promising to keep beside a pushed room
    /// is as dependent on it as the room downwind, so the partner travels too
    /// ([`stranded_partners`]), the pair keeps its cell, and the newcomer gets the doorstep it
    /// asked for. This case is the one this quest deliberately inverted; the paragraph above is
    /// what it used to pin.
    #[test]
    fn a_perpendicular_neighbour_travels_with_the_blocker_it_is_paired_to() {
        let (g, mut pos) = house(&[
            (5, Direction::W, 1),
            (1, Direction::S, 5),
            (5, Direction::E, 8),
            (8, Direction::W, 5),
        ]);
        pos.insert(8, (1, 1)); // due east of `South of House`, exactly one cell away
        let seat = seat_adjacent(&g, &mut pos, 2, (0, 1)).unwrap();
        assert_eq!(seat.cell, (0, 1), "the ghost took the doorstep it wanted");
        assert_ne!(seat.cell, (0, 2), "never past the blocker any more");
        assert!(seat.moved_map);
        assert_eq!(pos[&5], (0, 2), "South of House stepped down a row");
        assert_eq!(pos[&8], (1, 2), "and its eastern partner stepped down with it");
        assert_eq!(pos[&8].0 - pos[&5].0, 1, "the E/W pair is still exactly one cell apart");
        assert_eq!(pos[&8].1, pos[&5].1);
        for (id, cell) in HOUSE_AT_REST {
            assert_eq!(pos[&id], cell, "room {id} did not move");
        }
    }

    /// …and the partner closure is TRANSITIVE and drags what it displaces, exactly as the
    /// dependant closure does: the partner's own partner comes, and a room standing where the
    /// party is headed is swept up rather than sat on.
    #[test]
    fn a_partner_brings_its_own_partner_and_whatever_stands_in_their_way() {
        let (g, mut pos) = house(&[
            (5, Direction::W, 1),
            (1, Direction::S, 5),
            (5, Direction::E, 8),
            (8, Direction::W, 5),
            (8, Direction::E, 9),
            (9, Direction::W, 8),
        ]);
        pos.insert(8, (1, 1));
        pos.insert(9, (2, 1)); // partner of the partner
        pos.insert(10, (2, 2)); // …and squarely in `9`'s way
        let seat = seat_adjacent(&g, &mut pos, 2, (0, 1)).unwrap();
        assert_eq!(seat.cell, (0, 1));
        assert!(seat.moved_map);
        assert_eq!(
            (pos[&5], pos[&8], pos[&9], pos[&10]),
            ((0, 2), (1, 2), (2, 2), (2, 3)),
            "blocker, partner, partner's partner and the room they displaced all moved one row"
        );
    }

    /// **The anchor never joins, however the closure reaches it.** Here the blocker's eastern
    /// partner is the ANCHOR's eastern neighbour too, so bringing the partner would bring the
    /// anchor — and a party holding the anchor lands on the very cell the newcomer wants. The
    /// party is refused, the bare column is refused with it (the pair it would strand is exactly
    /// the one that started this), and the seating falls back the way it always did. This is Zork
    /// I's `Studio` ghost in miniature: `Behind House` is cardinally beside the `Kitchen`.
    #[test]
    fn a_partner_that_would_drag_the_anchor_in_refuses_the_push() {
        // `2` is the anchor, `5` the blocker on its doorstep, `8` the blocker's eastern partner,
        // `9` the partner's northern partner — and `9` is the ANCHOR's eastern partner too, so the
        // closure walks 5 → 8 → 9 → 2. `6`/`7` shut the whole-side slide as they do in `house`.
        let g = g_with(
            &[
                (2, "Kitchen"),
                (5, "South of House"),
                (8, "Beside"),
                (9, "Behind House"),
                (6, "Clearing"),
                (7, "Forest"),
            ],
            &[
                (5, Direction::E, 8),
                (8, Direction::W, 5),
                (8, Direction::N, 9),
                (9, Direction::S, 8),
                (2, Direction::E, 9),
                (9, Direction::W, 2),
                (6, Direction::S, 7),
                (7, Direction::N, 6),
            ],
        );
        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            [(2, (0, 0)), (5, (0, 1)), (8, (1, 1)), (9, (1, 0)), (6, (3, 0)), (7, (3, 1))]
                .into_iter()
                .collect();
        let seat = seat_adjacent(&g, &mut pos, 2, (0, 1)).unwrap();
        assert!(!seat.moved_map, "the closure reached the anchor, so nothing legal to move");
        assert_eq!(seat.cell, (-1, 0), "beside the anchor, exactly as before SQ-1375");
        assert_eq!(
            (pos[&5], pos[&8], pos[&9]),
            ((0, 1), (1, 1), (1, 0)),
            "and every kept pair is still exactly one cell apart"
        );
        assert_eq!(pos[&2], (0, 0), "the anchor never moves");
    }

    /// A partner closure that blows [`MAX_PUSH_SET`] is not a reason to abandon the cell: the
    /// party is refused and the BARE COLUMN is tried under the same veto, exactly as SQ-1367
    /// arranged for an oversized DEPENDANT closure. Here the blocker itself is half of no kept
    /// pair, so the bare push is legal and the newcomer keeps its doorstep — at the cost of the
    /// dependant `4` staying put, which is precisely the trade SQ-1367 made.
    #[test]
    fn an_oversized_partner_closure_falls_back_to_the_bare_column() {
        // `4` hangs off the blocker's `S` exit (a distorted one — two cells away diagonally, so no
        // kept pair) and carries a chain of E/W partners long enough to blow the cap. `6`/`7` shut
        // the whole-side slide.
        let mut rooms: Vec<(RoomId, &str)> =
            vec![(1, "Hall"), (2, "Below"), (4, "Dependant"), (6, "Clearing"), (7, "Forest")];
        let mut edges = vec![(2, Direction::S, 4), (6, Direction::S, 7), (7, Direction::N, 6)];
        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            [(1, (0, 0)), (2, (0, 1)), (4, (2, 2)), (6, (-2, 0)), (7, (-2, 1))]
                .into_iter()
                .collect();
        let mut prev = 4;
        for k in 0..MAX_PUSH_SET + 1 {
            let id = 10 + k as RoomId;
            rooms.push((id, "Chain"));
            edges.push((prev, Direction::E, id));
            edges.push((id, Direction::W, prev));
            pos.insert(id, (k as i32 + 3, 2));
            prev = id;
        }
        let g = g_with(&rooms, &edges);
        let seat = seat_adjacent(&g, &mut pos, 1, (0, 1)).unwrap();
        assert_eq!(seat.cell, (0, 1), "the newcomer keeps the cell its bearing points at");
        assert!(seat.moved_map);
        assert_eq!(pos[&2], (0, 2), "the bare column stepped aside alone");
        assert_eq!(pos[&4], (2, 2), "its dependant stayed put, exactly as SQ-1367 allows");
        for k in 0..MAX_PUSH_SET + 1 {
            let id = 10 + k as RoomId;
            assert_eq!(pos[&id], (k as i32 + 3, 2), "partner {id} stayed where it was");
        }
    }

    /// **Zork I's `Attic`, reduced to its shape.** The anchor is the `Kitchen`; `North of House`
    /// stands on the cell the staircase points at, with `Forest Path` and `Clearing` behind it up
    /// the column, and `Forest` due WEST of `Forest Path` in a cardinal reciprocal pair. Before
    /// SQ-1375 that pair vetoed both the push and its bare-column fallback and the `Attic` walked
    /// four cells out of the house; now `Forest` travels with the column and the `Attic` takes
    /// the doorstep.
    ///
    /// The slide (step 2) is vetoed here by the `Clearing`/`Forest` pair that vetoes it in
    /// [`house`], so this measures the PUSH rather than the line-opening the real map happens to
    /// get.
    #[test]
    fn the_zork_house_column_steps_north_and_takes_the_forest_beside_it() {
        let g = g_with(
            &[
                (2, "Kitchen"),
                (143, "North of House"),
                (247, "Forest Path"),
                (167, "Clearing"),
                (91, "Forest"),
                (6, "Clearing"),
                (7, "Forest"),
            ],
            &[
                (143, Direction::N, 247),
                (247, Direction::S, 143),
                (247, Direction::N, 167),
                (167, Direction::S, 247),
                // `Forest` due west of `Forest Path`: the pair that used to veto everything.
                (91, Direction::E, 247),
                (247, Direction::W, 91),
                // …and the far-away pair that shuts the whole-side slide, as in `house`.
                (6, Direction::S, 7),
                (7, Direction::N, 6),
            ],
        );
        let mut pos: BTreeMap<RoomId, (i32, i32)> = [
            (2, (1, 3)),
            (143, (1, 2)),
            (247, (1, 1)),
            (167, (1, 0)),
            (91, (0, 1)),
            (6, (4, 2)),
            (7, (4, 3)),
        ]
        .into_iter()
        .collect();
        let seat = seat_adjacent(&g, &mut pos, 2, (0, -1)).unwrap();
        assert_eq!(seat.cell, (1, 2), "the Attic sits on the Kitchen's doorstep");
        assert_ne!(seat.cell, (1, -1), "not four cells up a column of its own");
        assert!(seat.moved_map);
        assert_eq!(
            (pos[&143], pos[&247], pos[&167], pos[&91]),
            ((1, 1), (1, 0), (1, -1), (0, 0)),
            "the whole column moved one cell north, and the Forest beside it came too"
        );
        assert_eq!(pos[&2], (1, 3), "the Kitchen never moves");
        assert_eq!((pos[&6], pos[&7]), ((4, 2), (4, 3)), "and the pair that vetoed the slide");
    }

    // ── SQ-1367: an oversized party falls back to the bare column ──────────────

    /// Zork I's Maze, reduced to its shape: `Grating Room` holds the doorstep the `Clearing`
    /// ghost's `Down` wants, one `Maze` room stands on it, and `deps` more hang NORTH off that
    /// blocker down a chain of one-way maze passages — the bearing's own side, so every one of
    /// them is a dependant. None of them is a cell apart from its neighbour, so nothing in the
    /// tangle is an adjacent reciprocal pair and the blocker is free to step aside alone.
    ///
    /// The `Clearing`/`Forest` pair straddles the cut at `y = -1`, exactly as it does in
    /// [`house`], so the whole-side slide is vetoed and the push is the only way through.
    fn maze(deps: usize) -> (MapGraph, BTreeMap<RoomId, (i32, i32)>) {
        let mut rooms: Vec<(RoomId, &str)> =
            vec![(1, "Grating Room"), (2, "Maze"), (6, "Clearing"), (7, "Forest")];
        let mut edges = vec![(6, Direction::S, 7), (7, Direction::N, 6)];
        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            [(1, (0, 0)), (2, (0, -1)), (6, (5, -1)), (7, (5, 0))].into_iter().collect();
        let mut prev = 2;
        for k in 0..deps {
            let id = 10 + k as RoomId;
            rooms.push((id, "Maze"));
            edges.push((prev, Direction::N, id));
            pos.insert(id, (3 + k as i32 * 2, -3));
            prev = id;
        }
        (g_with(&rooms, &edges), pos)
    }

    /// The reported case (SQ-1367): the dependant closure swallows the maze, blows
    /// [`MAX_PUSH_SET`] — and the push happens anyway, with the blocker alone. Before this the
    /// whole push was refused and the ghost was seated on the perpendicular side, so the portal
    /// drew four turns between two adjacent boxes.
    #[test]
    fn an_oversized_dependant_party_falls_back_to_the_bare_column() {
        let (g, mut pos) = maze(MAX_PUSH_SET + 1);
        let seat = seat_adjacent(&g, &mut pos, 1, (0, -1)).unwrap();
        assert_eq!(seat.cell, (0, -1), "the ghost keeps the cell its bearing points at");
        assert_ne!(seat.cell, (1, 0), "never beside the anchor while the straight cell is free");
        assert!(seat.moved_map);
        assert_eq!(pos[&2], (0, -2), "the blocker alone stepped aside");
        assert_eq!(pos[&1], (0, 0), "the anchor never moves");
        assert_eq!((pos[&6], pos[&7]), ((5, -1), (5, 0)), "and the pair that vetoed the slide");
        for k in 0..MAX_PUSH_SET + 1 {
            let id = 10 + k as RoomId;
            assert_eq!(pos[&id], (3 + k as i32 * 2, -3), "dependant {id} stayed where it was");
        }
    }

    /// …and a party that FITS still travels whole, so SQ-1363's rule is untouched wherever it can
    /// be honoured: the same shape with two dependants moves all three rooms together.
    #[test]
    fn a_party_within_the_cap_still_takes_its_dependants() {
        let (g, mut pos) = maze(2);
        let seat = seat_adjacent(&g, &mut pos, 1, (0, -1)).unwrap();
        assert_eq!(seat.cell, (0, -1));
        assert!(seat.moved_map);
        assert_eq!(
            (pos[&2], pos[&10], pos[&11]),
            ((0, -2), (3, -4), (5, -4)),
            "the blocker and both dependants moved one row, keeping their own offsets"
        );
    }

    /// The case the pass exists for: a portal-only room hanging off a hub in the middle of a
    /// block. It ends up directly below the hub, and the block opens to let it.
    #[test]
    fn a_portal_only_room_seats_below_its_hub_and_the_block_opens() {
        // A 3x3 block, joined into three east-west reciprocal rows (no vertical passages, so
        // nothing straddling the cut claims a cell count). `9` hangs off the centre by Down.
        let mut g = MapGraph::new();
        let mut ids = Vec::new();
        for row in 0..3 {
            for col in 0..3 {
                let id = 1 + row * 3 + col;
                g.upsert_room(id, format!("R{id}"));
                g.set_pos(id, (col as i32 - 1, row as i32 - 1));
                ids.push(id);
            }
        }
        for row in 0..3 {
            for col in 0..2 {
                let (a, b) = (1 + row * 3 + col, 1 + row * 3 + col + 1);
                g.add_edge(a, Direction::E, b);
                g.add_edge(b, Direction::W, a);
            }
        }
        g.upsert_room(9_999, "Cellar".into());
        g.set_pos(9_999, (7, 7)); // wherever the collision pass left it
        g.add_edge(5, Direction::Down, 9_999);
        g.add_edge(9_999, Direction::Up, 5);

        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            g.rooms().filter_map(|r| r.pos.map(|p| (r.id, p))).collect();
        seat_portal_leaves(&g, &mut pos);
        assert_eq!(pos[&5], (0, 0), "the hub stayed put");
        assert_eq!(pos[&9_999], (0, 1), "the cellar is directly below the hub");
        for row in [7, 8, 9] {
            assert_eq!(pos[&row].1, 2, "the bottom row opened to make space: {row} at {:?}", pos[&row]);
        }
        // Every east-west pair is still exactly adjacent.
        for row in 0..3 {
            for col in 0..2 {
                let (a, b) = (1 + row * 3 + col, 1 + row * 3 + col + 1);
                assert_eq!(pos[&b].0 - pos[&a].0, 1, "{a}-{b} stayed adjacent");
                assert_eq!(pos[&b].1, pos[&a].1);
            }
        }
    }

    #[test]
    fn seat_offset_reads_up_down_in_out() {
        assert_eq!(seat_offset(Direction::Up), Some((0, -1)));
        assert_eq!(seat_offset(Direction::Down), Some((0, 1)));
        assert_eq!(seat_offset(Direction::In), Some((1, 0)));
        assert_eq!(seat_offset(Direction::Out), Some((-1, 0)));
        assert_eq!(seat_offset(Direction::NE), Some((1, -1)));
        assert_eq!(seat_offset(Direction::Unknown), None);
    }
}
