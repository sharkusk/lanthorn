//! SQ-1304: Anchorhead's Twisting Lane, and what the room lock leaves behind when it resolves
//! mid-session.
//!
//! # The report
//!
//! On 0.4.2, against the COMMERCIAL Anchorhead (Glulx), `/export-map` drew one `"Twisting Lane"`
//! node carrying `random=[E→(4 destinations)]` and three further compass edges the layout had
//! given up on (`dropped=[…N…, …S…, …W…]`). The reporter: *"Twisting Lane is a randomized
//! 'illusion' map, but Lanthorn doesn't detect that"*, and on 0.4.3, *"Twisting Lane seems
//! unimproved"*.
//!
//! Every room id in that dump is `synthetic_room_id` of the room's own heading, so `glulx_roomlock`
//! never resolved for the whole session — which is SQ-1286, and SQ-1286 shipped IN 0.4.3
//! (`0acd243d`, an ancestor of `v0.4.3`). So 0.4.3 does change what that game does: the lock now
//! resolves. What this suite pins is what happens NEXT.
//!
//! # The fixture, and what it can and cannot show
//!
//! `stories/AnchorheadDemo.gblorb` — *Anchorhead: Special Edition Demo*, release 3, serial
//! 070202, Inform 7 build 4K41 (Glulx), and the only Anchorhead we have. NOT the commercial
//! build the report is against, which is several times its size.
//!
//! Its Twisting Lane is **one room**, not a same-named maze: walked with the lock already
//! resolved, the `location` global holds one single address however many times the lane is
//! entered and whatever direction is tried out of it (see
//! [`the_demo_twisting_lane_is_a_single_room`]). Only its EAST exit randomises — *"You make your
//! way through the dark, tangled streets…"* lands in Riverwalk, Hidden Alley or Town Square from
//! run to run — which is exactly the `random=[E→…]` the reporter's dump carries.
//!
//! So the demo cannot exhibit a same-named multi-room LANE, and **the merge half of the report is
//! not covered here yet** — several distinct rooms sharing one heading are folded into one node
//! before the lock resolves, and `MapGraph::rekey_room` can rename a node but never split one, so
//! the remap recovers the first of them and refuses the rest. That is a separate design question
//! and a separate quest; this suite covers the half the demo can reach.
//!
//! # What the demo DOES show: the remap re-keyed the MAP but not the SESSION'S OWN CACHE
//!
//! Rooms walked before the lock resolves are keyed by the hash of their heading. When the lock
//! lands, `RoomLock::take_remap` hands back `(name, real id)` for each and `turn.rs` re-keys the
//! mapper's nodes. Nobody re-keyed `GlulxSession::last_room`, the session's own cached
//! `LocationInfo`, which went on holding the pre-lock NAME hash: it is rebuilt only on a turn
//! `adopt_heading_for_room` approves, and once the lock has resolved that refuses every
//! `Movement::Unchanged` turn — a `wait`, a `take`, a refused move, a keypress. So the very next
//! heading-less turn handed `apply_turn` the id the map had just retired, the mapper minted it as
//! a room it had never seen, and the room the player was standing in was on the map twice, wired
//! to its own twin by an edge no passage explains.
//!
//! Measured on this fixture before the fix, route `west south north east | wait …` — the lock
//! resolves on the fourth turn and the remap drains on the fifth, which is the `wait`:
//!
//! ```text
//! ROOM #2666847941 "Outside the Real Estate Office"   <== the pre-lock NAME hash
//! ROOM #3546012668 "Outside the Real Estate Office"   <== the post-lock ADDRESS hash
//! EDGE #3546012668 Unknown #2666847941
//! EDGE #2666847941 W #4151728165  distorted
//! ```
//!
//! `GlulxSession::rekey_last_room_to_lock` is the session's half of the same swap, done at the one
//! moment it can be: where `finish_turn` already notices the lock resolving, beside
//! `remember_room_global`. The `distorted` tangle above is the shape the report describes, and it
//! could only arrive once SQ-1286 (`0acd243d`, in `v0.4.3`) made the commercial game's lock
//! resolve at all — in 0.4.2 there was no remap to be half-applied.
//!
//! Both cases skip vacuously without `stories/` (gitignored).

use std::path::PathBuf;

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::roomid::{glulx_room_id, synthetic_room_id};
use app::session::{apply_turn, DeathWatch};
use mapper::graph::RoomId;
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

fn story(name: &str) -> Option<Vec<u8>> {
    let path = fixture_path(name);
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            None
        }
    }
}

/// Boot the demo into play, dismissing any "press a key" splash — `sq1286_glulx_room_lock::boot`'s
/// pattern, minus the RAMSTART nothing here needs.
fn boot(tag: &str) -> Option<GlulxSession> {
    let bytes = story("AnchorheadDemo.gblorb")?;
    let blorb = blorb::Blorb::parse(bytes).ok()?;
    let (kind, exec) = blorb.executable().ok()?;
    assert_eq!(kind, blorb::ExecKind::Glulx, "AnchorheadDemo.gblorb is a Glulx blorb");
    let store: PathBuf = app::scratch_dir(tag);
    let mut s = GlulxSession::new_in(
        store,
        exec.to_vec(),
        80,
        24,
        true,
        false,
        false,
        false,
        (1, 1),
        None,
        &[],
        [[(None, None); 11]; 2],
        false,
        None,
    )
    .unwrap_or_else(|e| panic!("the Anchorhead demo boots: {e:?}"));
    for _ in 0..12 {
        if s.current_location().is_some() {
            break;
        }
        if s.pending_input() != app::session::InputKind::Char {
            break;
        }
        s.submit_key(KeyInput::Enter);
    }
    Some(s)
}

/// Natural play through the REAL pipeline — the remap drained and applied before `apply_turn`,
/// exactly as `turn::finish_command_turn` does it (`crates/app/src/turn.rs:170`). Deliberately NO
/// room-lock warmup: the warmup `sq1264_forest_randomization::GPlay` performs is what keeps a
/// mid-session remap out of the mapper, and it is not something a player does.
struct Play {
    mapper: Mapper,
    session: GlulxSession,
    death: DeathWatch,
    /// Every `(name, old id, new id, accepted)` the remap produced, for the failure message.
    remaps: Vec<(String, RoomId, RoomId, bool)>,
}

impl Play {
    fn demo(tag: &str) -> Option<Play> {
        Some(Play {
            mapper: Mapper::default(),
            session: boot(tag)?,
            death: DeathWatch::default(),
            remaps: Vec::new(),
        })
    }

    fn turn(&mut self, cmd: &str) {
        for _ in 0..4 {
            if self.session.pending_input() != app::session::InputKind::Char {
                break;
            }
            self.session.submit_key(KeyInput::Enter);
        }
        if self.session.pending_input() != app::session::InputKind::Line {
            return;
        }
        let room_before = self.mapper.graph.current();
        let mut result = Engine::submit(&mut self.session, cmd);
        let dir = mapper::direction::parse_direction(cmd);
        if let (Some(o), Some(d)) = (room_before, dir) {
            result.declared_exit = Some(self.session.declared_exit(o, d));
        }
        for (name, addr) in self.session.take_room_remap() {
            let old_id = synthetic_room_id(&name);
            let new_id = glulx_room_id(addr);
            let ok = self.mapper.rekey_room(old_id, new_id);
            self.remaps.push((name, old_id, new_id, ok));
        }
        apply_turn(&mut self.mapper, cmd, &result, &mut self.death);
        // No probe can be armed in this harness, and `turn.rs` resolves a suspicion immediately
        // when none can run.
        if let Some(susp) = self.mapper.take_random_exit_suspicion() {
            self.mapper.resolve_suspicion_as_random(susp);
        }
    }

    /// The map as the dump would show it, for a failure message that explains itself.
    fn picture(&self) -> String {
        let mut out = String::new();
        let mut rooms: Vec<_> = self.mapper.graph.rooms().collect();
        rooms.sort_by_key(|r| r.id);
        for r in rooms {
            let label = r.label().to_string();
            let tag = if r.id == synthetic_room_id(&label) { "   <== NAME-KEYED" } else { "" };
            out.push_str(&format!("  ROOM #{} {label:?}{tag}\n", r.id));
        }
        for c in self.mapper.graph.connections() {
            out.push_str(&format!(
                "  EDGE #{} {:?} #{}{}\n",
                c.origin,
                c.dir,
                c.dest,
                if c.distorted { "  distorted" } else { "" }
            ));
        }
        for (name, old, new, ok) in &self.remaps {
            out.push_str(&format!(
                "  REMAP {name:?} #{old} -> #{new}: {}\n",
                if *ok { "applied" } else { "REFUSED" }
            ));
        }
        out
    }
}

/// The route. Three rooms and a `wait`, walked in a loop: `west` to Narrow Street, `south` to
/// Twisting Lane, `north` back, `east` home, then a heading-less turn. Deliberately never `east`
/// out of the lane — that is the randomiser, and a route that takes it is a different walk every
/// run. Measured on this fixture: the lock resolves on turn 4 and the remap drains on turn 5,
/// which is the `wait`.
const ROUTE: [&str; 19] = [
    "west", "south", "north", "east", "wait", //
    "west", "south", "north", "east", "wait", //
    "west", "south", "north", "east", "wait", //
    "west", "south", "north", "east",
];

/// Characterisation, and the reason the merge half of this report is synthetic below: the demo's
/// Twisting Lane is ONE room. Walked with the lock already resolved, every arrival reports the
/// same id, so the `location` global holds one single object address.
#[test]
fn the_demo_twisting_lane_is_a_single_room() {
    let Some(mut s) = boot("sq1304-one-room") else { return };

    // Resolve the lock without entering the lane, so nothing below can be an artefact of a
    // name-keyed turn.
    for c in ["wait", "west", "east", "wait", "west", "east"] {
        let _ = Engine::submit(&mut s, c);
    }
    assert!(
        s.locked_room_global().is_some(),
        "the opening streets resolve the lock before the lane is entered"
    );

    // Now into the lane, and out of it every way there is, several times over.
    let probe = [
        "west", "south", "north", "south", "look", "north", "south", "northeast", "south",
        "northwest", "south", "southeast", "south", "southwest", "south", "up", "down",
    ];
    let mut lane_ids: Vec<RoomId> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for c in probe {
        if s.pending_input() != app::session::InputKind::Line {
            break;
        }
        let _ = Engine::submit(&mut s, c);
        if let Some(l) = s.current_location() {
            if !names.contains(&l.name) {
                names.push(l.name.clone());
            }
            if l.name == "Twisting Lane" && !lane_ids.contains(&l.number) {
                lane_ids.push(l.number);
            }
        }
    }

    // Non-vacuity: the walk really did enter the lane, and really did leave it.
    assert!(names.iter().any(|n| n == "Twisting Lane"), "the route reaches the lane, saw {names:?}");
    assert!(names.len() >= 2, "…and passes through more than the lane, saw {names:?}");
    assert_eq!(
        lane_ids.len(),
        1,
        "the demo's Twisting Lane is one object, not a same-named maze: {lane_ids:?}"
    );
    assert_ne!(
        lane_ids[0],
        synthetic_room_id("Twisting Lane"),
        "…and it is keyed by that object's address, not by its heading"
    );
}

/// **The reproduction.** Once the lock resolves and the remap re-keys the map, the SESSION still
/// answers with the pre-lock name hash on the next heading-less turn, and the room the player is
/// standing in is mapped a second time.
#[test]
fn a_room_mapped_before_the_lock_is_not_mapped_twice_after_it() {
    let Some(mut p) = Play::demo("sq1304-duplicate") else { return };
    for c in ROUTE {
        p.turn(c);
    }

    // Non-vacuity: the lock really resolved, a remap really was applied, and the walk really did
    // pass through the lane — without all three there is nothing here to fail on.
    assert!(p.session.locked_room_global().is_some(), "the route resolves the room lock");
    assert!(
        p.remaps.iter().any(|(_, _, _, ok)| *ok),
        "…and hands back rooms to re-key\n{}",
        p.picture()
    );
    let labels: Vec<String> = p.mapper.graph.rooms().map(|r| r.label().to_string()).collect();
    assert!(labels.iter().any(|l| l == "Twisting Lane"), "the route maps the lane, saw {labels:?}");

    // No room may still be keyed by the hash of its own heading once the lock has resolved: that
    // is precisely the id the remap exists to retire.
    let stale: Vec<String> = p
        .mapper
        .graph
        .rooms()
        .filter(|r| r.id == synthetic_room_id(r.label()))
        .map(|r| format!("#{} {:?}", r.id, r.label()))
        .collect();
    assert!(
        stale.is_empty(),
        "SQ-1304: with the lock resolved every room is keyed by its address, but these still \
         carry the pre-lock name hash: {stale:?}\n{}",
        p.picture()
    );

    // …and therefore no name is on the map twice.
    let mut sorted = labels.clone();
    sorted.sort();
    let mut deduped = sorted.clone();
    deduped.dedup();
    assert_eq!(
        sorted,
        deduped,
        "SQ-1304: one node per room, but a name is mapped twice\n{}",
        p.picture()
    );
}
