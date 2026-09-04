//! Noticing that a set of rooms wants to be a layer of its own — and remembering the answer
//! (SQ-0439).
//!
//! Layers are still created and destroyed ONLY by an explicit [`crate::layer::move_region`]. This
//! module never moves anything: it watches the player walk and, twice in a game, says "those rooms
//! look like they belong apart". A prompt renders it; the player decides.
//!
//! Two independent triggers, one prompt, one memory:
//!
//! * **Structural** — a set of rooms that hangs off one portal and nothing else. There are two
//!   moments it can be noticed at, because there are two shapes of region: on the way back OUT of
//!   it ([`structural_trigger`]), or, when there is no way back out, while the player is still
//!   down there and the region has just grown into a floor plan ([`descent_trigger`], SQ-0853).
//! * **Semantic** — you just walked into a room called "Maze".
//!
//! And a memory that keeps a declined suggestion declined, because a prompt that reappears on the
//! next step is worse than no prompt at all: it trains the player to dismiss it blind.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::direction::{grid_offset, is_portal, Direction};
use crate::graph::{MapGraph, RoomId};
use crate::layer::{is_whole_layer, planar_region, region_at_arrival, LayerId, MoveTarget, Region};

/// The smallest region worth interrupting the player about. Below it a suggestion is noise: a
/// two-room cupboard behind a door is not a floor plan, and drawing it in place is what the
/// in-plane default with dotted portal stubs is FOR.
pub const STRUCTURAL_FLOOR: usize = 4;

/// The passage a suggestion was about, and the key its answer is remembered under.
///
/// It is always the crossing the player will make AGAIN, because re-arming has to hit the same key
/// that firing did — which is the way OUT for [`structural_trigger`] (it fires as you leave), and
/// the way IN for [`name_trigger`] and [`descent_trigger`] (they fire while you are still inside).
/// For a descent that is the portal the region hangs off, not whatever step happened to be walked
/// when the region grew big enough to mention.
///
/// **Which way it points is readable from the suggestion itself**, and nothing else has to be told
/// (SQ-0858): `from` is a room INSIDE [`LayerSuggestion::region`] exactly when the seam is a way
/// out, and outside it exactly when the seam is a way in. Both triggers guarantee it by
/// construction — [`structural_trigger`] walks the region FROM `from`, so it contains it, while
/// [`entry_seam`] takes only origins the region does not contain. A prompt that reads the two
/// apart this way cannot fall out of step with whichever trigger fired, because it is reading the
/// data the trigger produced rather than a second label about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeamKey {
    pub from: RoomId,
    pub dir: Direction,
}

// [`Direction`] is deliberately not `Ord` (see `Room::tried`), so the ordering that makes this a
// `BTreeMap` key is spelled out here instead of derived: room first, then the direction's short
// tag, which is distinct for all thirteen.
impl Ord for SeamKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.from.cmp(&other.from).then_with(|| {
            crate::direction::short_label(self.dir).cmp(crate::direction::short_label(other.dir))
        })
    }
}

impl PartialOrd for SeamKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// What the player has already said about a seam — the whole of the prompt's memory.
///
/// "Mark for later" is not a list, a badge or a command: it is [`SeamDecision::Deferred`], which
/// simply re-arms. The three prompt outcomes are then a gradient over one mechanism — separate it
/// now / ask me again next time I come through / never ask about this passage again — and only the
/// last one silences anything.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SeamDecision {
    /// Never asked about. The state every seam starts in.
    #[default]
    Armed,
    /// Asked, and put off — ask again on the next crossing.
    Deferred,
    /// Asked, and declined for good.
    Ignored,
}

/// Which of the two triggers fired. They want different things of an accepted suggestion, so the
/// prompt has to be able to tell them apart: accepting a NAME-triggered move also sets the target
/// layer's maze flag (the player confirmed it is a maze by accepting a prompt that said so),
/// while a structural one sets nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// A region reachable only through portals — noticed on the way back out, or while still being
    /// explored when there is no way back out.
    Structural,
    /// A room whose name contains the word "maze", noticed on the way in.
    Name,
}

/// A layer a region might already belong to, and the evidence: how many COMPASS edges tie the two
/// together. See [`home_candidates`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Home {
    pub layer: LayerId,
    pub edges: usize,
}

/// What the map has to say. The prompt renders this; accepting it means calling
/// [`crate::layer::move_region`] with `region` and one of `destinations`.
///
/// `region` is computed by the same walkers the move itself uses, so a suggestion cannot disagree
/// with what accepting it would actually do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerSuggestion {
    pub trigger: Trigger,
    /// The crossing that fired it, and the key its answer is remembered under.
    pub seam: SeamKey,
    /// The rooms that would move.
    pub region: Region,
    /// Where they could go, best first: [`MoveTarget::New`] when carving a fresh layer is legal,
    /// then any existing layer the region already has compass edges into, most edges first. Never
    /// empty — a suggestion with nowhere to go is not made at all.
    pub destinations: Vec<MoveTarget>,
}

/// True when `name` contains the WORD "maze", case-insensitively — `\bmaze\b`, hand-rolled because
/// `mapper` is not carrying a regex crate for four letters.
///
/// The word boundaries are the entire point. "Amazement" and "Amazed" contain the letters and mean
/// nothing of the sort, and a room called "A Maze of Twisty Little Passages" means exactly it.
pub fn mentions_maze(name: &str) -> bool {
    const WORD: [char; 4] = ['m', 'a', 'z', 'e'];
    let chars: Vec<char> = name.chars().collect();
    let is_word_char = |c: char| c.is_alphanumeric() || c == '_';
    for i in 0..chars.len().saturating_sub(WORD.len() - 1) {
        if !(0..WORD.len()).all(|k| chars[i + k].to_ascii_lowercase() == WORD[k]) {
            continue;
        }
        let starts = i == 0 || !is_word_char(chars[i - 1]);
        let ends = i + WORD.len() >= chars.len() || !is_word_char(chars[i + WORD.len()]);
        if starts && ends {
            return true;
        }
    }
    false
}

/// The layers `region` might already belong to, best first (SQ-0439).
///
/// A region is at home on a layer it has COMPASS edges into. That case is not hypothetical: new
/// rooms are always discovered on the ACTIVE layer (`layout::incremental` inherits the layer from
/// the room you came from), so peel a cellar, keep exploring, and rooms that are in-plane
/// neighbours of that cellar land on whatever layer happened to be active — structurally part of a
/// layer they are not on.
///
/// **Portal edges never nominate a home.** They are what separates layers in the first place, so
/// the `down` that reaches a cellar must not be read as evidence that the cellar belongs upstairs;
/// if it were, every peel would immediately suggest undoing itself.
///
/// Grid adjacency is deliberately not weighed either. Layout position is derived by the router and
/// can shift, so a suggestion built on it could change without the map changing. Evidence has to be
/// topological.
pub fn home_candidates(graph: &MapGraph, region: &Region) -> Vec<Home> {
    let src = graph.layer_of(region.anchor);
    let mut counts: BTreeMap<LayerId, usize> = BTreeMap::new();
    for c in graph.connections() {
        if grid_offset(c.dir).is_none() || c.is_self_loop() {
            continue; // a portal is a boundary, not a bond; a self-loop touches nothing
        }
        let other = match (region.rooms.contains(&c.origin), region.rooms.contains(&c.dest)) {
            (true, false) => c.dest,
            (false, true) => c.origin,
            _ => continue, // wholly inside or wholly outside: says nothing about a home
        };
        let layer = graph.layer_of(other);
        if layer != src {
            *counts.entry(layer).or_default() += 1;
        }
    }
    let mut out: Vec<Home> =
        counts.into_iter().map(|(layer, edges)| Home { layer, edges }).collect();
    // Most evidence first; ties by layer id, so the ranking is stable rather than map-order.
    out.sort_by(|a, b| b.edges.cmp(&a.edges).then(a.layer.cmp(&b.layer)));
    out
}

/// Everywhere `region` could legally go, best first: a fresh layer when that is not merely a
/// rename, then each [`home_candidates`] layer.
///
/// This reproduces three of [`crate::layer::move_region`]'s four refusals rather than calling it,
/// so nothing in this list can be turned down by the move it describes — **for this caller**
/// (SQ-1065). The sentence used to claim it mirrors them all; it omits `EmptiesMain`
/// (`crate::layer::layer.rs`'s check that a move must not empty the main layer), which is
/// currently unreachable only because all three of this function's triggers structurally exclude
/// a whole-layer region. `destinations` is `pub` with one caller, and the promise breaks the
/// moment it gains a second — so add the fourth refusal, or call `move_region`, before doing that.
pub fn destinations(graph: &MapGraph, region: &Region) -> Vec<MoveTarget> {
    let mut out = Vec::new();
    if !is_whole_layer(graph, region) {
        out.push(MoveTarget::New);
    }
    out.extend(home_candidates(graph, region).into_iter().map(|h| MoveTarget::Existing(h.layer)));
    out
}

/// Look at the move the player just made and decide whether the map has anything to say about it.
///
/// `from -dir-> to` is the crossing as walked, and `newly_seen` says whether `to` is a room the map
/// had never seen before this move. Only the caller knows that: the graph has already been updated
/// by the time this runs, and "the newest room in the graph" is not the same thing — walk into a
/// dead end and back out and the dead end is still the newest room, several moves later. It is what
/// [`descent_trigger`] fires on, so getting it from the graph instead would re-fire on every step
/// back and forth inside a region (SQ-0853).
///
/// All three triggers are evaluated here and the semantic one wins, because it is the stronger
/// evidence: a room called "Maze" says what it is on first contact, while structural evidence is
/// only ever circumstantial. The two structural ones cannot both fire — one wants a step back in
/// discovery order and the other a step forward — so their order between themselves says nothing.
///
/// Cheap tests come first on purpose — this runs on the interpreter's turn, once per move, and
/// everything expensive is behind either "the passage was a portal" or "the room is called maze".
pub fn on_arrival(
    graph: &MapGraph,
    from: RoomId,
    dir: Direction,
    to: RoomId,
    newly_seen: bool,
) -> Option<LayerSuggestion> {
    // "Never for this story" (SQ-1298): the player has said the whole prompt is unwelcome on this
    // map, so neither trigger below gets a turn. Checked before the per-seam decision — cheaper,
    // and it is the single choke point every route into `name_trigger`/`structural_trigger`/
    // `descent_trigger` passes through, so nothing downstream needs its own copy of this check.
    if graph.suggestions_disabled() {
        return None;
    }
    let seam = SeamKey { from, dir };
    if graph.seam_decision(seam) == SeamDecision::Ignored {
        return None;
    }
    // A crossing that already spans two layers has nothing left to separate — which is also what
    // makes accepting a suggestion self-silencing, with no bookkeeping to do it.
    if graph.layer_of(from) != graph.layer_of(to) {
        return None;
    }
    name_trigger(graph, from, dir, to, seam)
        .or_else(|| structural_trigger(graph, from, dir, to, seam))
        .or_else(|| descent_trigger(graph, from, dir, to, newly_seen))
}

/// The semantic trigger: the room just entered is called "maze" and the room left is not.
///
/// It fires on the TRANSITION, so there is one suggestion per entrance and silence inside. Zork's
/// maze is fifteen rooms all called "Maze"; asking in each is precisely the nagging this whole
/// design exists to avoid.
///
/// It bypasses the structural floor and the return-crossing timing, because both of those exist to
/// compensate for weak evidence — and the name IS the evidence, available at the doorway.
fn name_trigger(
    graph: &MapGraph,
    from: RoomId,
    dir: Direction,
    to: RoomId,
    seam: SeamKey,
) -> Option<LayerSuggestion> {
    if !mentions_maze(graph.room(to)?.label()) || mentions_maze(graph.room(from)?.label()) {
        return None;
    }
    // The passage walked in through IS an inbound seam at `to`, so the region it cuts is already
    // exactly what a move would take. When it is not a seam at all — a portal, or a passage whose
    // two ends stay connected another way — the portal-bounded walk is the right region instead.
    let region = region_at_arrival(graph, from, dir).unwrap_or_else(|_| planar_region(graph, to));
    if region.rooms.contains(&from) {
        return None; // nothing to separate the maze FROM
    }
    finish(Trigger::Name, seam, region, graph)
}

/// The structural trigger: a region reachable only through portals, noticed on the way back out.
///
/// Three things have to hold at once, and each is a case the naive "more than two rooms reached by
/// up/down/in/out" heuristic gets wrong:
///
/// * **Portal-bounded.** The far side must have no in-plane path back. A cellar you can only climb
///   down to qualifies; a balcony you can also walk round to does not, and the walk proves it —
///   [`planar_region`] is closed under compass edges, so an in-plane way back puts the room you
///   just walked INTO inside the region.
/// * **Four rooms.** A floor, not a cupboard.
/// * **A step BACK.** The crossing has to be a return — the room entered was on the map before the
///   room left. Discovery order is what says so, and without it the very same portal fires on the
///   way DOWN, where the region being left is the whole of the map you came from, and cheerfully
///   offers to peel your starting town off it.
///
/// A layer the player has already flagged as a maze is exempt outright — the point of flagging it
/// was to keep the whole maze together.
///
/// The step-back rule is why this trigger alone is not enough (SQ-0853): Zork's trapdoor is barred
/// behind you, so the return crossing it waits for never happens, and the player explores the whole
/// underground in silence. [`descent_trigger`] is the other half — the same evidence, read from
/// inside the region instead of from the doorway on the way out.
fn structural_trigger(
    graph: &MapGraph,
    from: RoomId,
    dir: Direction,
    to: RoomId,
    seam: SeamKey,
) -> Option<LayerSuggestion> {
    if !is_portal(dir) || graph.layer_is_maze(graph.layer_of(from)) {
        return None;
    }
    if graph.room(to)?.seq >= graph.room(from)?.seq {
        return None; // still heading outward, so the map is not finished being drawn
    }
    let region = planar_region(graph, from);
    if region.rooms.contains(&to) || region.rooms.len() < STRUCTURAL_FLOOR {
        return None;
    }
    finish(Trigger::Structural, seam, region, graph)
}

/// The portal a region HANGS OFF, and the key a suggestion about it is remembered under (SQ-0853).
///
/// The region's oldest room is the one the map saw first, so it is the room the region was entered
/// at; the seam is then a portal into it from a room OUTSIDE the region that is older still. That
/// last clause is the whole safety of firing forwards, and it is the same evidence the return
/// crossing used, asked of the region instead of of the step: a region whose every room postdates
/// the room it hangs off is an appendage of that room, whichever way the player happens to be
/// walking. The starting room predates everything, so no region containing it can ever have an
/// entry seam, and the starting town can never be what a suggestion offers to peel.
///
/// Ties break on the OLDEST outside room, which is the one choice that cannot change as the map
/// grows: a portal found later can only have a higher `seq`, so a key once chosen stays chosen —
/// and a memory whose key drifted would silently lose the player's answer.
///
/// Portals only ([`is_portal`]), matching [`structural_trigger`]: an `Unknown` edge is a cut for
/// [`planar_region`]'s purposes but it is not a passage anyone can point at.
fn entry_seam(graph: &MapGraph, region: &Region) -> Option<SeamKey> {
    let layer = graph.layer_of(region.anchor);
    let entry = *region.rooms.iter().min_by_key(|&&id| graph.room(id).map_or(u64::MAX, |r| r.seq))?;
    let entry_seq = graph.room(entry)?.seq;
    graph
        .connections()
        .iter()
        .filter(|c| {
            c.dest == entry
                && is_portal(c.dir)
                && !region.rooms.contains(&c.origin)
                && graph.layer_of(c.origin) == layer
                && graph.room(c.origin).is_some_and(|r| r.seq < entry_seq)
        })
        .min_by_key(|c| {
            (graph.room(c.origin).map_or(u64::MAX, |r| r.seq), crate::direction::short_label(c.dir))
        })
        .map(|c| SeamKey { from: c.origin, dir: c.dir })
}

/// The structural trigger read forwards: a region behind a portal, noticed while the player is
/// still inside it (SQ-0853).
///
/// [`structural_trigger`] waits for the way back out, and Zork I is the game that never provides
/// one — the trapdoor is barred behind you, so a player can explore the entire underground and
/// never be asked anything. This fires instead from within, and the reason it can do so safely is
/// that it never looks at the region BEHIND the player: [`planar_region`] is walked from the room
/// arrived in, so the rooms on offer are always the ones beyond the seam.
///
/// Two moments, and only two:
///
/// * **The region crosses the floor.** `to` is a room the map has never seen, so it brought exactly
///   one room with it and `== STRUCTURAL_FLOOR` is the step that turned a cupboard into a floor
///   plan. Firing on `>=` instead would ask again for every room added after it, which is the nag
///   the whole design exists to avoid; the map says its piece once and then lets the player explore.
/// * **The seam is crossed again.** That is what [`SeamDecision::Deferred`] means — "ask me next
///   time I come through" — so the way IN has to be able to re-raise it, exactly as the way out
///   does for a region you can walk out of.
///
/// A layer already flagged as a maze is exempt, for the same reason as in [`structural_trigger`].
fn descent_trigger(
    graph: &MapGraph,
    from: RoomId,
    dir: Direction,
    to: RoomId,
    newly_seen: bool,
) -> Option<LayerSuggestion> {
    if graph.layer_is_maze(graph.layer_of(to)) {
        return None;
    }
    // The ARRIVAL side, always. `region_at_arrival` cannot serve here: the crossing that grows a
    // region is usually an ordinary passage inside it, and cutting there names a handful of rooms
    // rather than the region the portal bounds.
    let region = planar_region(graph, to);
    let seam = entry_seam(graph, &region)?;
    if graph.seam_decision(seam) == SeamDecision::Ignored {
        return None;
    }
    let fires = if newly_seen {
        region.rooms.len() == STRUCTURAL_FLOOR
    } else {
        seam == SeamKey { from, dir } && region.rooms.len() >= STRUCTURAL_FLOOR
    };
    if !fires {
        return None;
    }
    finish(Trigger::Structural, seam, region, graph)
}

/// Attach the destinations, and drop the suggestion when there are none — an offer the move would
/// refuse is not an offer.
fn finish(
    trigger: Trigger,
    seam: SeamKey,
    region: Region,
    graph: &MapGraph,
) -> Option<LayerSuggestion> {
    let destinations = destinations(graph, &region);
    (!destinations.is_empty()).then_some(LayerSuggestion { trigger, seam, region, destinations })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{move_region, MAIN_LAYER};
    use crate::mapper::Mapper;

    /// A manor with two four-room halves. Upstairs: Hall(1) —E— Study(2) —E— Parlour(9), and
    /// Kitchen(7) north of the Hall. Downstairs, through the Hall's trapdoor: Cellar(3) —E— Wine
    /// Cellar(4) —E— Vault(5) —E— Crypt(6). The player ends up in the Crypt.
    ///
    /// Discovery order is the observe order, which is exactly what the timing rule reads.
    ///
    /// The Crypt is the cellar's fourth room, so the descent trigger has already spoken by the time
    /// this returns (SQ-0853); every caller either walks on — which clears it — or takes it.
    fn manor() -> Mapper {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Study", Some(Direction::E));
        m.observe(9, "Parlour", Some(Direction::E));
        m.observe(2, "Study", Some(Direction::W));
        m.observe(1, "Hall", Some(Direction::W));
        m.observe(7, "Kitchen", Some(Direction::N));
        m.observe(1, "Hall", Some(Direction::S));
        m.observe(3, "Cellar", Some(Direction::Down));
        m.observe(4, "Wine Cellar", Some(Direction::E));
        m.observe(5, "Vault", Some(Direction::E));
        m.observe(6, "Crypt", Some(Direction::E));
        m
    }

    /// Walk the Crypt back to the foot of the stairs. Nothing about walking around inside a region
    /// can fire anything, so any suggestion left over is discarded.
    fn back_to_the_stairs(m: &mut Mapper) {
        m.observe(5, "Vault", Some(Direction::W));
        m.observe(4, "Wine Cellar", Some(Direction::W));
        m.observe(3, "Cellar", Some(Direction::W));
        assert_eq!(m.take_suggestion(), None, "walking about inside the cellar says nothing");
    }

    /// Four rooms behind one `down`, and the map says so on the way UP — not before.
    #[test]
    fn climbing_out_of_a_portal_bounded_cellar_suggests_a_layer() {
        let mut m = manor();
        back_to_the_stairs(&mut m);
        m.observe(1, "Hall", Some(Direction::Up));
        let s = m.take_suggestion().expect("climbing back out is when the map has something to say");
        assert_eq!(s.trigger, Trigger::Structural);
        assert_eq!(s.seam, SeamKey { from: 3, dir: Direction::Up });
        assert_eq!(s.region.rooms, [3, 4, 5, 6].into_iter().collect());
        assert_eq!(s.destinations, vec![MoveTarget::New], "a fresh cellar has no other home");
    }

    /// The GATE. A balcony you can also walk round to is not behind a boundary at all, whatever the
    /// `up` says — and the in-plane way back is what proves it.
    #[test]
    fn a_region_with_an_in_plane_path_back_never_suggests() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(6, "Cupboard", Some(Direction::In)); // so the layer is more than the balcony
        m.observe(1, "Hall", Some(Direction::Out));
        m.observe(2, "Yard", Some(Direction::N));
        m.observe(3, "Balcony", Some(Direction::Up)); // the portal
        m.observe(4, "Gallery", Some(Direction::E));
        m.observe(5, "Landing", Some(Direction::E));
        m.observe(2, "Yard", Some(Direction::S)); // ...and a walk-round straight back to the Yard
        m.observe(3, "Balcony", Some(Direction::Up));
        m.take_suggestion();
        let region = planar_region(&m.graph, 3);
        assert!(
            region.rooms.len() >= STRUCTURAL_FLOOR && !is_whole_layer(&m.graph, &region),
            "big enough and not the whole layer, so neither of those is what stops this: {region:?}"
        );

        m.observe(2, "Yard", Some(Direction::Down));
        assert_eq!(
            m.take_suggestion(),
            None,
            "the Yard is inside the balcony's own planar region, so nothing is behind a boundary"
        );
    }

    /// The FLOOR. Three rooms behind a trapdoor is a cupboard, and drawing it in place with a
    /// dotted portal stub is the right answer.
    #[test]
    fn a_region_under_the_floor_never_suggests() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Study", Some(Direction::E));
        m.observe(3, "Pantry", Some(Direction::Down));
        m.observe(4, "Cold Store", Some(Direction::E));
        m.observe(5, "Shelf", Some(Direction::E));
        m.observe(4, "Cold Store", Some(Direction::W));
        m.observe(3, "Pantry", Some(Direction::W));
        m.take_suggestion();
        assert_eq!(planar_region(&m.graph, 3).rooms.len(), 3, "the pantry is three rooms");

        m.observe(1, "Hall", Some(Direction::Up));
        assert_eq!(m.take_suggestion(), None, "under STRUCTURAL_FLOOR, so it stays quiet");
    }

    /// The TIMING. On the way DOWN, the region being left is the map you came from; offering to
    /// peel your starting town off it is the naive heuristic's worst failure.
    #[test]
    fn descending_into_a_new_region_never_suggests() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Study", Some(Direction::E));
        m.observe(9, "Parlour", Some(Direction::E));
        m.observe(2, "Study", Some(Direction::W));
        m.observe(1, "Hall", Some(Direction::W));
        m.observe(7, "Kitchen", Some(Direction::N));
        m.observe(1, "Hall", Some(Direction::S));
        m.take_suggestion();
        assert_eq!(planar_region(&m.graph, 1).rooms.len(), 4, "a four-room upper floor");

        m.observe(3, "Cellar", Some(Direction::Down)); // the descent, off that four-room floor
        assert_eq!(
            m.take_suggestion(),
            None,
            "the upstairs was on the map before the cellar, so it is not the appendage"
        );
    }

    // ── SQ-0853: the descent trigger — a one-way region noticed from inside ──

    /// Zork I's shape, which is the whole of the report: five surface rooms, a trapdoor that bars
    /// itself behind you, and four rooms below with no way back up. [`structural_trigger`] waits
    /// for a return crossing that never comes, so before this the player explored the entire
    /// underground in silence.
    ///
    /// Stops one step short of the cellar so each test below walks its own way down.
    fn barred_trapdoor() -> Mapper {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "North of House", Some(Direction::N));
        m.observe(3, "Behind House", Some(Direction::E));
        m.observe(4, "Kitchen", Some(Direction::W));
        m.observe(5, "Living Room", Some(Direction::W));
        m
    }

    /// Walk the cellar as the player did, one room at a time, and the map says its piece on the
    /// fourth — while they are still down there, with no way back up to be noticed on.
    #[test]
    fn a_one_way_descent_suggests_once_the_region_becomes_a_floor_plan() {
        let mut m = barred_trapdoor();
        m.observe(6, "Cellar", Some(Direction::Down));
        assert_eq!(m.take_suggestion(), None, "one room is a doorway, not a floor plan");
        m.observe(7, "East of Chasm", Some(Direction::S));
        assert_eq!(m.take_suggestion(), None, "two");
        m.observe(8, "Gallery", Some(Direction::E));
        assert_eq!(m.take_suggestion(), None, "three, still under the floor");

        m.observe(9, "Studio", Some(Direction::N));
        let s = m.take_suggestion().expect("the fourth room is when the cellar is worth a layer");
        assert_eq!(s.trigger, Trigger::Structural);
        assert_eq!(
            s.seam,
            SeamKey { from: 5, dir: Direction::Down },
            "remembered under the trapdoor the region hangs off, not the step that grew it"
        );
        assert_eq!(s.region.rooms, [6, 7, 8, 9].into_iter().collect());
        assert_eq!(s.destinations, vec![MoveTarget::New]);
    }

    /// The INVARIANT the prompt's two structural sentences are read apart by (SQ-0858). Both
    /// shapes report `Trigger::Structural`, and the only thing that tells them apart is where the
    /// seam's outside end sits: inside the region when it is the way out, outside it when it is
    /// the way in. Break this and the descent prompt goes back to describing itself as a return.
    #[test]
    fn a_return_seam_is_inside_the_region_and_a_descent_seam_is_outside() {
        // Climbing out: the seam is `Cellar -up-> Hall`, and the Cellar is one of the rooms that
        // would move.
        let mut m = manor();
        back_to_the_stairs(&mut m);
        m.observe(1, "Hall", Some(Direction::Up));
        let out = m.take_suggestion().expect("the climb out speaks");
        assert_eq!(out.trigger, Trigger::Structural);
        assert!(
            out.region.rooms.contains(&out.seam.from),
            "a way OUT starts inside: {:?} in {:?}",
            out.seam,
            out.region.rooms
        );

        // Descending: the seam is `Living Room -down-> Cellar`, and the Living Room stays put.
        let mut m = barred_trapdoor();
        for (id, name, dir) in [
            (6, "Cellar", Direction::Down),
            (7, "East of Chasm", Direction::S),
            (8, "Gallery", Direction::E),
            (9, "Studio", Direction::N),
        ] {
            m.observe(id, name, Some(dir));
        }
        let down = m.take_suggestion().expect("the fourth room speaks");
        assert_eq!(down.trigger, Trigger::Structural, "the SAME trigger, and a different reading");
        assert!(
            !down.region.rooms.contains(&down.seam.from),
            "a way IN starts outside: {:?} in {:?}",
            down.seam,
            down.region.rooms
        );
    }

    /// **The guard the forward direction exists to earn.** On the way down, the region BEHIND the
    /// player is the whole of the map they came from, and offering to peel that is the naive
    /// heuristic's worst failure. The rooms named are the ones beyond the seam, always.
    #[test]
    fn a_descent_never_offers_the_world_above() {
        let mut m = barred_trapdoor();
        assert_eq!(
            planar_region(&m.graph, 1).rooms.len(),
            5,
            "the surface is big enough and portal-bounded enough to be offered by mistake"
        );
        for (id, name, dir) in [
            (6, "Cellar", Direction::Down),
            (7, "East of Chasm", Direction::S),
            (8, "Gallery", Direction::E),
            (9, "Studio", Direction::N),
        ] {
            m.observe(id, name, Some(dir));
            if let Some(s) = m.take_suggestion() {
                for above in [1, 2, 3, 4, 5] {
                    assert!(
                        !s.region.rooms.contains(&above),
                        "the surface must never be what is offered: {:?}",
                        s.region.rooms
                    );
                }
                assert_eq!(s.region.rooms, [6, 7, 8, 9].into_iter().collect());
            }
        }
    }

    /// The same guard from the other side, and the case that decides how a seam is chosen: a
    /// portal that leads INTO the starting room from somewhere discovered later is not a way in to
    /// the town, it is a way out of it. Without the discovery-order test on the seam's outside end,
    /// the fourth street would offer to peel the town off its own cellar.
    #[test]
    fn a_portal_into_an_older_room_never_makes_it_a_region() {
        let mut m = Mapper::default();
        m.observe(1, "Town Square", None);
        m.observe(2, "Cellar", Some(Direction::Down));
        m.observe(1, "Town Square", Some(Direction::Up)); // `2 -Up-> 1`: a portal INTO the start
        m.observe(3, "North Road", Some(Direction::N));
        m.observe(4, "East Road", Some(Direction::E));
        m.observe(5, "South Road", Some(Direction::S));

        assert_eq!(planar_region(&m.graph, 1).rooms.len(), STRUCTURAL_FLOOR, "big enough to fire");
        assert_eq!(
            m.take_suggestion(),
            None,
            "the town predates the cellar, so it is not the cellar's appendage"
        );
        assert_eq!(entry_seam(&m.graph, &planar_region(&m.graph, 1)), None, "…and has no seam");
    }

    /// The FLOOR, forwards. A two-room cupboard behind a door is not a floor plan, whichever way
    /// the player is walking when it is counted.
    #[test]
    fn a_two_room_cupboard_behind_a_door_never_suggests_on_the_way_in() {
        let mut m = barred_trapdoor();
        m.observe(6, "Cupboard", Some(Direction::In));
        assert_eq!(m.take_suggestion(), None);
        m.observe(7, "Shelf", Some(Direction::N));
        assert_eq!(m.take_suggestion(), None, "two rooms is a cupboard, and it stays quiet");
        assert_eq!(planar_region(&m.graph, 6).rooms.len(), 2);
    }

    /// It must not NAG. The map says its piece once, on the step that turned the region into a
    /// floor plan, and then lets the player explore: walking about says nothing, and neither does
    /// every room found after it.
    #[test]
    fn a_descent_suggestion_is_made_once_and_then_stays_quiet() {
        let mut m = barred_trapdoor();
        m.observe(6, "Cellar", Some(Direction::Down));
        m.observe(7, "East of Chasm", Some(Direction::S));
        m.observe(8, "Gallery", Some(Direction::E));
        m.observe(9, "Studio", Some(Direction::N));
        m.take_suggestion().expect("it fires exactly here");

        m.observe(8, "Gallery", Some(Direction::S));
        assert_eq!(m.take_suggestion(), None, "walking back over known ground says nothing");
        m.observe(9, "Studio", Some(Direction::N));
        assert_eq!(m.take_suggestion(), None, "…and neither does walking forward over it again");
        m.observe(10, "Gallery Annexe", Some(Direction::E));
        assert_eq!(m.take_suggestion(), None, "a fifth room does not re-open a settled question");
        m.observe(11, "Cold Store", Some(Direction::E));
        assert_eq!(m.take_suggestion(), None, "nor a sixth");
    }

    /// "Not now" means "ask me next time I come through", and for a region entered by a portal the
    /// crossing the player makes again is the way IN. So the descent has to be able to re-raise it.
    #[test]
    fn a_deferred_descent_asks_again_on_the_way_back_in() {
        let mut m = manor();
        let seam = SeamKey { from: 1, dir: Direction::Down };
        let s = m.take_suggestion().expect("the manor's fourth cellar room speaks on the way down");
        assert_eq!(s.seam, seam, "the trapdoor, not the passage that happened to grow the region");
        m.graph.set_seam_decision(seam, SeamDecision::Deferred);

        back_to_the_stairs(&mut m);
        m.observe(1, "Hall", Some(Direction::Up));
        m.take_suggestion(); // the climb out has its own trigger; not what this is about
        m.observe(3, "Cellar", Some(Direction::Down));
        let s = m.take_suggestion().expect("a deferred trapdoor asks again on the next descent");
        assert_eq!(s.seam, seam);
        assert_eq!(s.region.rooms, [3, 4, 5, 6].into_iter().collect(), "still the rooms below");
    }

    /// …and "never" silences that descent for good.
    #[test]
    fn an_ignored_descent_stays_silent_on_the_way_back_in() {
        let mut m = manor();
        m.graph.set_seam_decision(SeamKey { from: 1, dir: Direction::Down }, SeamDecision::Ignored);
        back_to_the_stairs(&mut m);
        m.observe(1, "Hall", Some(Direction::Up));
        m.take_suggestion();
        m.observe(3, "Cellar", Some(Direction::Down));
        assert_eq!(m.take_suggestion(), None, "declined for good means declined for good");
    }

    /// The name still wins where both could fire: the fourth room of a portal-bounded region is
    /// also called "Maze", and the prompt that comes up is the one that can say why.
    #[test]
    fn the_name_trigger_still_wins_over_a_descent() {
        let mut m = barred_trapdoor();
        m.observe(6, "Cellar", Some(Direction::Down));
        m.observe(7, "East of Chasm", Some(Direction::S));
        m.observe(8, "Gallery", Some(Direction::E));
        m.observe(9, "Maze", Some(Direction::N));
        let s = m.take_suggestion().expect("both could fire here");
        assert_eq!(s.trigger, Trigger::Name, "the name is the stronger evidence");
        assert_eq!(s.seam, SeamKey { from: 8, dir: Direction::N }, "keyed on the way into the maze");
        assert_eq!(s.region.rooms, [9].into_iter().collect(), "and it takes the maze, not the cellar");
    }

    /// The EXEMPTION holds forwards too: a layer the player has flagged as a maze is kept whole.
    #[test]
    fn a_maze_flagged_layer_gets_no_descent_suggestion() {
        let mut m = barred_trapdoor();
        m.observe(6, "Cellar", Some(Direction::Down));
        m.observe(7, "East of Chasm", Some(Direction::S));
        m.observe(8, "Gallery", Some(Direction::E));
        m.graph.set_layer_maze(MAIN_LAYER, true);
        m.observe(9, "Studio", Some(Direction::N));
        assert_eq!(m.take_suggestion(), None, "the maze flag exempts the layer outright");
    }

    /// A descent answer survives the map file, exactly as a climb-out answer does — and the
    /// crossing after the reload is the test, not the reload.
    #[test]
    fn an_ignored_descent_is_still_ignored_after_a_save_and_reload() {
        let mut m = manor();
        let seam = SeamKey { from: 1, dir: Direction::Down };
        m.graph.set_seam_decision(seam, SeamDecision::Ignored);
        back_to_the_stairs(&mut m);

        let mut m2 = crate::persist::from_json(&crate::persist::to_json(&m)).unwrap();
        assert_eq!(m2.graph.seam_decision(seam), SeamDecision::Ignored);
        m2.observe(1, "Hall", Some(Direction::Up));
        m2.take_suggestion();
        m2.observe(3, "Cellar", Some(Direction::Down));
        assert_eq!(m2.take_suggestion(), None, "a restore does not un-decline a descent");
    }

    /// The EXEMPTION. Once the player has said a layer is a maze, we keep the entire maze together
    /// — no structural prompt may pick at it again.
    #[test]
    fn a_maze_flagged_layer_gets_no_structural_suggestion() {
        let mut m = manor();
        back_to_the_stairs(&mut m);
        m.graph.set_layer_maze(MAIN_LAYER, true);
        m.observe(1, "Hall", Some(Direction::Up));
        assert_eq!(m.take_suggestion(), None, "the maze flag exempts the layer outright");
    }

    /// The word boundary, which is the whole of the name rule. "Amazement" is not a maze.
    #[test]
    fn maze_is_matched_as_a_word_not_a_substring() {
        assert!(mentions_maze("Maze"));
        assert!(mentions_maze("maze"));
        assert!(mentions_maze("A Maze of Twisty Little Passages, All Alike"));
        assert!(mentions_maze("Twisty maze"));
        assert!(!mentions_maze("Amazement"));
        assert!(!mentions_maze("Amazed"));
        assert!(!mentions_maze("Mazes"));
        assert!(!mentions_maze("Hall of Mirrors"));
        assert!(!mentions_maze(""));
    }

    /// Walking into a room called "Maze" says everything at the doorway: no floor, no waiting for
    /// the player to come back out.
    #[test]
    fn walking_into_a_maze_suggests_immediately() {
        let mut m = Mapper::default();
        m.observe(1, "Troll Room", None);
        m.observe(2, "Maze", Some(Direction::W));
        let s = m.take_suggestion().expect("the name is evidence enough on first contact");
        assert_eq!(s.trigger, Trigger::Name);
        assert_eq!(s.seam, SeamKey { from: 1, dir: Direction::W });
        assert_eq!(s.region.rooms, [2].into_iter().collect(), "one room so far, and that is fine");
        assert_eq!(s.destinations, vec![MoveTarget::New]);
    }

    /// One suggestion per entrance, silence inside. Zork's maze is fifteen rooms all called
    /// "Maze"; asking in each is the nag this design exists to avoid.
    #[test]
    fn moving_between_maze_rooms_says_nothing() {
        let mut m = Mapper::default();
        m.observe(1, "Troll Room", None);
        m.observe(2, "Maze", Some(Direction::W));
        m.take_suggestion().expect("the entrance fires");
        m.observe(3, "Maze", Some(Direction::N));
        assert_eq!(m.take_suggestion(), None, "maze → maze is not a transition into a maze");
        m.observe(4, "Maze", Some(Direction::S));
        assert_eq!(m.take_suggestion(), None);
    }

    /// An "Amazement Park" reached by an ordinary passage trips nothing at all.
    #[test]
    fn a_room_that_merely_contains_the_letters_never_fires() {
        let mut m = Mapper::default();
        m.observe(1, "Fairground", None);
        m.observe(2, "Amazement Park", Some(Direction::W));
        assert_eq!(m.take_suggestion(), None);
    }

    /// The HOME rule, and the one thing most likely to be got backwards: the `down` that reaches a
    /// cellar must never nominate the room above it as the cellar's home.
    #[test]
    fn a_portal_edge_never_nominates_a_home() {
        let mut m = manor();
        // Carve the cellar off, so Main and the cellar layer are joined by the down/up pair only.
        let region = planar_region(&m.graph, 3);
        let cellar = move_region(&mut m.graph, &region, MoveTarget::New).unwrap();
        let region = planar_region(&m.graph, 3);
        assert_eq!(
            home_candidates(&m.graph, &region),
            vec![],
            "the up/down portal is a boundary, not evidence that the cellar belongs upstairs"
        );
        // Now give the cellar a COMPASS edge into Main and the same region does have a home.
        m.graph.add_edge(6, Direction::E, 2);
        m.graph.add_edge(2, Direction::W, 6);
        let region = planar_region(&m.graph, 3);
        assert_eq!(
            home_candidates(&m.graph, &region),
            vec![Home { layer: MAIN_LAYER, edges: 2 }],
            "a compass edge is topological evidence; a portal is not"
        );
        assert_eq!(m.graph.layer_of(3), cellar, "and none of this moved anything");
    }

    /// Candidates rank by edge count, most evidence first — and a region that IS its whole layer is
    /// never offered a new one, because that would only rename it.
    #[test]
    fn home_candidates_rank_by_edge_count() {
        let mut m = manor();
        let region = planar_region(&m.graph, 3);
        move_region(&mut m.graph, &region, MoveTarget::New).unwrap();
        let attic = m.graph.new_layer(Some(MAIN_LAYER), "Attic".into());
        m.graph.upsert_room(20, "Loft".into());
        m.graph.set_room_layer(20, attic);
        m.graph.add_edge(6, Direction::N, 20); // one edge to the attic
        m.graph.add_edge(6, Direction::E, 2); // two to Main
        m.graph.add_edge(2, Direction::W, 6);
        let region = planar_region(&m.graph, 3);
        assert_eq!(
            home_candidates(&m.graph, &region),
            vec![Home { layer: MAIN_LAYER, edges: 2 }, Home { layer: attic, edges: 1 }]
        );
        assert!(is_whole_layer(&m.graph, &region));
        assert_eq!(
            destinations(&m.graph, &region),
            vec![MoveTarget::Existing(MAIN_LAYER), MoveTarget::Existing(attic)],
            "no `new`: the region is already its own whole layer"
        );
    }

    /// A whole layer with nowhere to go is not offered anything, and a suggestion with no
    /// destination is never made at all.
    #[test]
    fn a_region_with_no_legal_destination_is_never_suggested() {
        let mut m = manor();
        let region = planar_region(&m.graph, 3);
        move_region(&mut m.graph, &region, MoveTarget::New).unwrap();
        let region = planar_region(&m.graph, 3);
        assert_eq!(destinations(&m.graph, &region), vec![], "nowhere legal to go");
        let seam = SeamKey { from: 3, dir: Direction::Up };
        assert_eq!(finish(Trigger::Structural, seam, region, &m.graph), None);
    }

    /// The case the home rule exists for, end to end. Peel a cellar, keep exploring, and the rooms
    /// you find next land on whatever layer was ACTIVE — so a sub-basement dug out from the Kitchen
    /// ends up on Main while sharing a wall with the Crypt. The suggestion offers the cellar layer
    /// alongside a fresh one, on the strength of that compass edge and nothing else.
    #[test]
    fn a_region_with_compass_edges_into_another_layer_is_offered_that_layer() {
        let mut m = manor();
        back_to_the_stairs(&mut m);
        m.observe(1, "Hall", Some(Direction::Up));
        let s = m.take_suggestion().expect("the cellar suggests itself");
        let cellar = move_region(&mut m.graph, &s.region, MoveTarget::New).unwrap();

        m.observe(7, "Kitchen", Some(Direction::N));
        m.observe(10, "Coal Chute", Some(Direction::Down)); // new rooms join the ACTIVE layer: Main
        m.observe(11, "Coal Store", Some(Direction::E));
        m.observe(12, "Furnace", Some(Direction::E));
        m.observe(13, "Boiler Room", Some(Direction::E));
        assert_eq!(m.graph.layer_of(13), MAIN_LAYER, "on Main, though it belongs downstairs");
        m.graph.add_edge(13, Direction::E, 6); // ...and it shares a wall with the Crypt
        m.graph.add_edge(6, Direction::W, 13);

        m.observe(12, "Furnace", Some(Direction::W));
        m.observe(11, "Coal Store", Some(Direction::W));
        m.observe(10, "Coal Chute", Some(Direction::W));
        m.take_suggestion();
        m.observe(7, "Kitchen", Some(Direction::Up));

        let s = m.take_suggestion().expect("four rooms behind the coal chute");
        assert_eq!(s.trigger, Trigger::Structural);
        assert_eq!(s.region.rooms, [10, 11, 12, 13].into_iter().collect());
        assert_eq!(
            s.destinations,
            vec![MoveTarget::New, MoveTarget::Existing(cellar)],
            "a layer of its own, or the cellar it is actually attached to"
        );
    }

    /// The MEMORY. An ignored seam stays ignored, and the crossing that would otherwise fire is
    /// simply silent.
    #[test]
    fn an_ignored_seam_stays_silent() {
        let mut m = manor();
        back_to_the_stairs(&mut m);
        m.graph.set_seam_decision(SeamKey { from: 3, dir: Direction::Up }, SeamDecision::Ignored);
        m.observe(1, "Hall", Some(Direction::Up));
        assert_eq!(m.take_suggestion(), None, "declined for good means declined for good");
    }

    /// Deferring is not ignoring: "ask me again next crossing" re-arms, which is the whole of what
    /// "mark for later" means, and the whole reason it is not a list or a badge.
    #[test]
    fn a_deferred_seam_asks_again_on_the_next_crossing() {
        let mut m = manor();
        back_to_the_stairs(&mut m);
        m.graph.set_seam_decision(SeamKey { from: 3, dir: Direction::Up }, SeamDecision::Deferred);
        m.observe(1, "Hall", Some(Direction::Up));
        let s = m.take_suggestion().expect("a deferred seam is re-armed, not silenced");
        assert_eq!(s.seam, SeamKey { from: 3, dir: Direction::Up });
    }

    /// Setting a seam back to armed forgets it outright, so the map carries only answers the player
    /// actually gave.
    #[test]
    fn re_arming_a_seam_forgets_it() {
        let mut m = Mapper::default();
        let key = SeamKey { from: 3, dir: Direction::Up };
        m.graph.set_seam_decision(key, SeamDecision::Ignored);
        assert_eq!(m.graph.seam_decisions().len(), 1);
        m.graph.set_seam_decision(key, SeamDecision::Armed);
        assert!(m.graph.seam_decisions().is_empty());
        assert_eq!(m.graph.seam_decision(key), SeamDecision::Armed);
    }

    /// Folding a layer back in is an answer too — the strongest one there is. The player has said
    /// these rooms belong together, so the boundary they used to have is never suggested again.
    #[test]
    fn a_region_merged_back_is_never_re_suggested() {
        let mut m = manor();
        back_to_the_stairs(&mut m);
        m.observe(1, "Hall", Some(Direction::Up));
        let s = m.take_suggestion().expect("it fires the first time");
        let cellar = move_region(&mut m.graph, &s.region, MoveTarget::New).unwrap();
        let region = crate::layer::layer_region(&m.graph, cellar).unwrap();
        move_region(&mut m.graph, &region, MoveTarget::Existing(MAIN_LAYER)).unwrap();
        assert_eq!(
            m.graph.seam_decision(SeamKey { from: 3, dir: Direction::Up }),
            SeamDecision::Ignored,
            "both orientations of the folded-shut passage are recorded"
        );
        assert_eq!(
            m.graph.seam_decision(SeamKey { from: 1, dir: Direction::Down }),
            SeamDecision::Ignored
        );

        m.observe(3, "Cellar", Some(Direction::Down));
        assert_eq!(m.take_suggestion(), None);
        m.observe(1, "Hall", Some(Direction::Up));
        assert_eq!(m.take_suggestion(), None, "a merge is a decision, and it sticks");
    }

    /// Accepting a suggestion silences it with no bookkeeping at all: the seam now spans two
    /// layers, and a crossing that already spans two layers has nothing left to separate.
    #[test]
    fn accepting_a_suggestion_silences_it_without_a_memory_entry() {
        let mut m = manor();
        back_to_the_stairs(&mut m);
        m.observe(1, "Hall", Some(Direction::Up));
        let s = m.take_suggestion().expect("fires once");
        move_region(&mut m.graph, &s.region, MoveTarget::New).unwrap();
        assert!(m.graph.seam_decisions().is_empty(), "nothing had to be remembered");

        m.observe(3, "Cellar", Some(Direction::Down));
        assert_eq!(m.take_suggestion(), None);
        m.observe(1, "Hall", Some(Direction::Up));
        assert_eq!(m.take_suggestion(), None, "the seam is cross-layer now, so it cannot fire");
    }

    /// A relocation is not a walked passage, so it can never fire either trigger — the same reason
    /// it leaves no `arrived_via` behind.
    #[test]
    fn an_involuntary_relocation_suggests_nothing() {
        let mut m = manor();
        back_to_the_stairs(&mut m);
        m.observe_relocation(1, "Hall");
        assert_eq!(m.take_suggestion(), None);
    }

    /// The memory survives the map file, and a restored game does not re-ask about a passage
    /// already declined. Restoring is not the test — the CROSSING after it is.
    #[test]
    fn an_ignored_seam_is_still_ignored_after_a_save_and_reload() {
        let mut m = manor();
        back_to_the_stairs(&mut m);
        m.graph.set_seam_decision(SeamKey { from: 3, dir: Direction::Up }, SeamDecision::Ignored);
        m.graph.set_seam_decision(SeamKey { from: 9, dir: Direction::In }, SeamDecision::Deferred);

        let mut m2 = crate::persist::from_json(&crate::persist::to_json(&m)).unwrap();
        assert_eq!(
            m2.graph.seam_decision(SeamKey { from: 3, dir: Direction::Up }),
            SeamDecision::Ignored
        );
        assert_eq!(
            m2.graph.seam_decision(SeamKey { from: 9, dir: Direction::In }),
            SeamDecision::Deferred,
            "a deferral is an answer too, and it survives"
        );

        // Perturb, THEN assert: the crossing that would have fired must still be silent. Asserting
        // on the restored map alone is the moment everything still looks correct.
        assert_eq!(m2.graph.current(), Some(3), "the reload put the player back at the stairs");
        m2.observe(1, "Hall", Some(Direction::Up));
        assert_eq!(m2.take_suggestion(), None, "a restore does not un-decline a seam");
    }

    /// "Never for this story" (SQ-1298) is story-wide, not per-seam: once set, neither the
    /// structural trigger nor the name trigger fires again, on a passage the flag was never asked
    /// about either.
    #[test]
    fn suggestions_disabled_silences_both_triggers() {
        let mut m = manor();
        back_to_the_stairs(&mut m);
        m.graph.set_suggestions_disabled(true);
        m.observe(1, "Hall", Some(Direction::Up));
        assert_eq!(m.take_suggestion(), None, "the structural trigger is silenced story-wide");

        let mut m2 = Mapper::default();
        m2.observe(1, "Troll Room", None);
        m2.graph.set_suggestions_disabled(true);
        m2.observe(2, "Maze", Some(Direction::W));
        assert_eq!(m2.take_suggestion(), None, "the name trigger is silenced too, not just structural");
    }

    /// The flag survives the map file, and a restored game stays quiet — the same shape as
    /// [`an_ignored_seam_is_still_ignored_after_a_save_and_reload`], perturbed rather than merely
    /// asserted on the restored map alone.
    #[test]
    fn suggestions_disabled_is_still_disabled_after_a_save_and_reload() {
        let mut m = manor();
        back_to_the_stairs(&mut m);
        m.graph.set_suggestions_disabled(true);

        let mut m2 = crate::persist::from_json(&crate::persist::to_json(&m)).unwrap();
        assert!(m2.graph.suggestions_disabled(), "the flag itself survives");
        m2.observe(1, "Hall", Some(Direction::Up));
        assert_eq!(m2.take_suggestion(), None, "a restore does not un-set the story-wide never");
    }

    /// A seam answer naming a room the map no longer has is dropped on load, exactly like a
    /// dangling connection or `current` — a phantom key would silence a passage that cannot exist.
    #[test]
    fn a_dangling_seam_answer_is_dropped_on_load() {
        let json = r#"{"version":1,
            "rooms":[{"id":1,"name":"A","label_override":null,"notes":"","pos":[0,0]}],
            "connections":[],"current":1,
            "seams":[{"from":1,"dir":"Up","decision":"Ignored"},
                     {"from":99,"dir":"Down","decision":"Ignored"}]}"#;
        let m = crate::persist::from_json(json).unwrap();
        assert_eq!(m.graph.seam_decisions().len(), 1, "only the entry naming a real room survives");
        assert_eq!(
            m.graph.seam_decision(SeamKey { from: 1, dir: Direction::Up }),
            SeamDecision::Ignored
        );
    }
}
