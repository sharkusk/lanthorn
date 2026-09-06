//! SQ-1345: walking between two rooms that print the SAME name is a crossing, not a self-loop.
//!
//! Reported from a real save (`~/.lanthorn/saves/zork1-invclues-r52-s871125.z5.save/`, entry
//! `map.json`): every inter-forest exit on the player's Zork I map was marked `?`, and each pool
//! named the correct other forest PLUS the room itself —
//!
//! ```text
//! room  33 "Forest" random_exits=[E] random_destinations=[[E, [175, 33]]]
//! room  91 "Forest" random_exits=[S] random_destinations=[[S, [230,  91]]]
//! room 175 "Forest" random_exits=[W] random_destinations=[[W, [ 33, 175]]]
//! room 230 "Forest" random_exits=[W] random_destinations=[[W, [ 91, 230]]]
//! ```
//!
//! — so the room card read `E ? destination varies: Forest 3, back here`. Nothing here is random:
//! `lanthorn-mapgen`'s static read of the story's own ZIL exit table calls all four plain
//! `declared` reciprocal edges (`#33 east -> #175`, `#175 west -> #33`, `#91 south -> #230`,
//! `#230 west -> #91`), and `Engine::declared_exit` answers `Room(175)` for 33-E at run time too.
//!
//! # The mechanism
//!
//! Neither the detector nor the shadow was at fault — both name the right room throughout. The
//! defect is in `session::apply_turn`'s SQ-1269 "hole 3" check, which fires in the `arrived`
//! branch: before minting an ordinary SELF-LOOP it asks whether `(origin, dir)` already carries a
//! real edge that a same-room landing would contradict. Its only disqualifier was `renamed` — the
//! room the player is standing in now prints a different name than the map holds for it — which is
//! a proxy for "you changed rooms" that holds only while room names are unique.
//!
//! Zork I ships four rooms called `Forest`. Walk east out of #33 into #175 and the heading printed
//! is the identical word the origin prints, so `renamed` is false, the check runs on a move that
//! DID change rooms, finds the edge the previous walk of that direction correctly minted, and
//! files a suspicion whose live landing is the ORIGIN (`note_random_exit_suspicion(o, d,
//! Some(old), o)` — the literal `o`, the "back here" the room card printed). The shadow then
//! reports #175 twice, honestly and correctly, and disagrees with a `live_dest` of #33 that the
//! player never landed in — so `deliver_suspicion` resolves it as random, deletes the good edge,
//! and pools the old destination (#175) and the phantom live landing (#33). That is the exact
//! `[175, 33]` in the save.
//!
//! The fix reads the fact the detector states outright — object identity, `moved_room` — instead
//! of inferring it from the printed name: `renamed || moved_room` disqualifies the check. A move
//! that DID change rooms and contradicts something is already the `existing_conflict`/`suspicious`
//! branch above, which files the suspicion with the real landing.
//!
//! Falsified: with `|| moved_room` removed,
//! [`re_walking_a_same_named_crossing_does_not_contradict_its_own_edge`] fails with
//! `re-walking #33 E marked it random; pool = [175, 33]` — the field pool, to the element.
//! Note which case that is. The FIRST crossing is clean even on the broken code, because there is
//! no edge yet for the check to contradict; the defect needs the direction walked a SECOND time,
//! which is why a suite pinned to one forest move would have reported the bug as absent.
//!
//! Skips vacuously without `stories/` (gitignored), the CI-safe pattern.

use std::path::PathBuf;
use std::sync::Arc;

use app::engine::{DeclaredExit, Engine};
use app::probe::ShadowRecipe;
use app::session::{apply_turn, DeathWatch, GameSession};
use app::state::AppState;
use mapper::direction::Direction;
use mapper::graph::RoomId;
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

/// Zork I release 52 / serial 871125 — the InvisiClues edition, a **v5** story, so
/// `zvm::location::detect_location_with` runs the v4+ status-line ladder rather than reading
/// global 0 outright. The field report is against this exact release.
const STORY: &str = "zork1-invclues-r52-s871125.z5";

/// The route the field transcript takes out of the house and into the trees, one command per
/// entry — `look` to seed the map, then West of House → Forest (#91) → Forest Path (#247) →
/// Forest (#33) → Forest (#175). The last leg is the first inter-forest crossing and the one the
/// bug was reported on; every room id below is DISCOVERED from it, never hardcoded.
const INTO_THE_FOREST: [&str; 5] = ["look", "w", "e", "e", "e"];

fn story() -> Option<Vec<u8>> {
    match std::fs::read(fixture_path(STORY)) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", fixture_path(STORY).display());
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

/// Drives `apply_turn` + a synchronously-settled Phase-2 search exactly the way
/// `turn::finish_command_turn` does — `declared_exit.rs`'s own `Play` is the precedent, and the
/// Phase-2 gate itself is `random_exit_probe::arm_for_finished_turn`'s, never restated here
/// (SQ-1314).
struct Play {
    state: AppState,
    mapper: Mapper,
    session: GameSession,
    death: DeathWatch,
    /// The raw Phase-2 answer from the most recent [`Play::turn`] that armed a search, kept so a
    /// case can inspect what the SHADOW itself reported (`step.location` per step) independently
    /// of how `random_exit_probe::deliver` judged it. `None` when no search armed that turn.
    last_answer: Option<app::probe::Answer>,
    /// The `(origin, dir, live_dest)` of the search the most recent [`Play::turn`] armed, if any.
    last_search: Option<(RoomId, Direction, RoomId)>,
}

impl Play {
    fn boot() -> Option<Play> {
        let bytes = story()?;
        let mut s = GameSession::new_with_trace(
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
        .expect("the story boots without a ZError");
        s.set_strip_prompt(false);
        let _ = s.submit("");
        let mut state = AppState::default();
        state.probe.arm(recipe(&bytes));
        Some(Play {
            state,
            mapper: Mapper::default(),
            session: s,
            death: DeathWatch::default(),
            last_answer: None,
            last_search: None,
        })
    }

    fn turn(&mut self, cmd: &str) {
        let room_before = self.mapper.graph.current();
        let mut result = self.session.submit(cmd);
        result.declared_exit =
            app::random_exit_probe::declared_exit_for_command(cmd, room_before, |o, d| {
                Engine::declared_exit(&self.session, o, d)
            });
        apply_turn(&mut self.mapper, cmd, &result, &mut self.death);
        app::random_exit_probe::arm_for_finished_turn(
            &mut self.state,
            &self.session,
            &mut self.mapper,
            cmd,
            room_before,
            result.declared_exit,
        );
        self.last_answer = None;
        self.last_search = self
            .state
            .random_exit_search
            .as_ref()
            .map(|s| (s.origin(), s.dir(), s.live_dest()));
        if self.state.random_exit_search.is_some() {
            if let Some(answer) = self.state.probe.settle() {
                if app::random_exit_probe::owns(&self.state, answer.token) {
                    app::random_exit_probe::deliver(&mut self.state, &mut self.mapper, &answer);
                }
                self.last_answer = Some(answer);
            } else {
                self.state.random_exit_search = None;
            }
        }
        self.state.random_exit_pre_move_save = self.session.rng_seed().map(|_| {
            (self.mapper.graph.current().unwrap_or(0), Arc::new(self.session.save_state()))
        });
    }

    fn edge(&self, from: RoomId, dir: Direction) -> Option<RoomId> {
        self.mapper
            .graph
            .connections()
            .iter()
            .find(|c| c.origin == from && c.dir == dir)
            .map(|c| c.dest)
    }

    fn label(&self, id: RoomId) -> String {
        self.mapper.graph.room(id).map(|r| r.label().to_string()).unwrap_or_default()
    }

    /// Every room a shadow attempt of the last armed search actually reported landing in.
    fn shadow_landings(&self) -> Vec<RoomId> {
        self.last_answer
            .as_ref()
            .and_then(|a| a.run.as_ref())
            .map(|run| {
                run.steps
                    .iter()
                    .filter(|s| !s.quit && !s.escaped)
                    .filter_map(|s| s.location)
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Walk [`INTO_THE_FOREST`] and return `(first_forest, second_forest)` — the two same-named rooms
/// the last leg crosses between. Non-vacuity: both must print `Forest`, be DIFFERENT object
/// numbers, and the origin's east must be a plain declared `Room(_)` naming the second, or this
/// suite is not looking at the shape it was written for.
fn cross_between_two_forests(p: &mut Play) -> (RoomId, RoomId) {
    for cmd in &INTO_THE_FOREST[..INTO_THE_FOREST.len() - 1] {
        p.turn(cmd);
    }
    let first = p.mapper.graph.current().expect("standing in the first forest");
    assert_eq!(p.label(first), "Forest", "the walk reached a room printing `Forest`");
    let declared = p.session.declared_exit(first, Direction::E);
    let DeclaredExit::Room(declared_dest) = declared else {
        panic!("forest #{first} east must be a plain declared Room(_) in the story's ZIL exit table, got {declared:?}");
    };
    p.turn(INTO_THE_FOREST[INTO_THE_FOREST.len() - 1]);
    let second = p.mapper.graph.current().expect("standing in the second forest");
    assert_eq!(second, declared_dest, "the crossing landed where the story's own exit table said");
    assert_ne!(first, second, "the two forests are different objects, not one room walked twice");
    assert_eq!(p.label(second), "Forest", "and the second one prints the same name as the first");
    (first, second)
}

/// The reported defect, end to end: an ordinary declared crossing between two rooms that share a
/// printed name keeps its edge, is never marked random, and — should a probe run at all — is never
/// judged against a live landing the player did not reach.
#[test]
fn forest_to_forest_crossing_keeps_its_edge_and_is_never_marked_random() {
    let Some(mut p) = Play::boot() else { return };
    let (first, second) = cross_between_two_forests(&mut p);

    // (a) The live landing is a different room from the origin.
    assert_eq!(p.mapper.graph.current(), Some(second));
    assert_ne!(second, first);

    // (b) The direction is not marked random, and the crossing's edge stands.
    assert!(
        !p.mapper.graph.is_random_exit(first, Direction::E),
        "#{first} E is marked random after an ordinary declared crossing into #{second}; pool = {:?}",
        p.mapper.graph.random_destinations(first, Direction::E)
    );
    assert_eq!(
        p.edge(first, Direction::E),
        Some(second),
        "the crossing's own edge must survive the turn that walked it"
    );

    // (c) If a probe ran at all, no shadow attempt may be counted as landing back in the origin,
    //     and no search may be judged against a live landing equal to the origin.
    if let Some((o, d, live)) = p.last_search {
        assert_ne!(
            live, o,
            "a search armed for #{o} {d:?} claims the player landed back in the room they left, \
             but they walked into #{second}"
        );
        assert!(
            !p.shadow_landings().contains(&first),
            "a shadow attempt reported landing in the origin #{first}: {:?}",
            p.shadow_landings()
        );
    }
}

/// Walk the SAME direction a second time, from the same origin. This is where the bug actually
/// bit in the field: the first crossing minted the edge, and the second one contradicted it.
#[test]
fn re_walking_a_same_named_crossing_does_not_contradict_its_own_edge() {
    let Some(mut p) = Play::boot() else { return };
    let (first, second) = cross_between_two_forests(&mut p);
    assert_eq!(p.edge(first, Direction::E), Some(second), "the first crossing minted the edge");

    // Back to the first forest and across again — a `north` out of #175 that the story's own
    // table declares straight back to #33, so no part of the return leg is in question here.
    let back = p.session.declared_exit(second, Direction::N);
    assert_eq!(back, DeclaredExit::Room(first), "the return leg is a plain declared exit too");
    p.turn("n");
    assert_eq!(p.mapper.graph.current(), Some(first), "back in the first forest");
    p.turn("e");

    assert_eq!(p.mapper.graph.current(), Some(second), "the second crossing landed in #{second}");
    assert!(
        !p.mapper.graph.is_random_exit(first, Direction::E),
        "re-walking #{first} E marked it random; pool = {:?}",
        p.mapper.graph.random_destinations(first, Direction::E)
    );
    assert_eq!(
        p.edge(first, Direction::E),
        Some(second),
        "and the edge the first crossing minted is still there"
    );
    assert!(
        !p.mapper
            .graph
            .random_destinations(first, Direction::E)
            .contains(&first),
        "the origin itself must never be pooled as a destination of a move that left it"
    );
}

/// A save written before the fix already carries the wrong marks, and walking the marked direction
/// again does NOT clear them — this pins that, so the next reader does not go looking for a repair
/// that is not there.
///
/// The Upgrade path (`random_exit_probe::deliver_upgrade`) is what would clear a stale mark, and
/// it reaches its own SQ-1269 flicker guard first: a pool that already holds two or more distinct
/// rooms outweighs a single agreeing pair of reseeded attempts, and the field pool holds exactly
/// two (`[175, 33]` — the real destination and the phantom "back here"). Both shadow attempts do
/// now agree with the live landing, so nothing is wrong with the evidence; the guard simply
/// refuses to act on it. Clearing these would need either a fresh map for the story or a repair
/// rule that can tell a phantom self-entry in a pool from a genuine one — neither is built here.
#[test]
fn a_stale_pre_fix_mark_survives_a_re_walk_and_needs_a_fresh_map() {
    let Some(mut p) = Play::boot() else { return };
    let (first, second) = cross_between_two_forests(&mut p);

    // Restate the saved map's state for this key exactly as the field `map.json` holds it: no
    // edge, marked random, pool = [the real destination, the origin itself].
    p.mapper.graph.remove_connection(first, Direction::E);
    p.mapper.graph.mark_random_exit(first, Direction::E);
    p.mapper.graph.note_random_destination(first, Direction::E, second);
    p.mapper.graph.note_random_destination(first, Direction::E, first);
    assert_eq!(p.mapper.graph.random_destinations(first, Direction::E), &[second, first]);

    p.turn("n");
    assert_eq!(p.mapper.graph.current(), Some(first), "back in the first forest");
    p.turn("e");
    assert_eq!(p.mapper.graph.current(), Some(second), "the re-walk crossed to #{second} again");

    // The Upgrade search armed, and every shadow attempt agreed with the live landing …
    let (o, d, live) = p.last_search.expect("a re-walk of a marked direction arms an Upgrade search");
    assert_eq!((o, d, live), (first, Direction::E, second));
    let landings = p.shadow_landings();
    assert!(!landings.is_empty(), "the shadow reported at least one landing");
    assert!(
        landings.iter().all(|&l| l == second),
        "every shadow attempt agreed with the live landing #{second}: {landings:?}"
    );

    // … and the mark still stands, because the two-room pool outweighs one agreeing pair.
    assert!(
        p.mapper.graph.is_random_exit(first, Direction::E),
        "SQ-1269's flicker guard holds the stale mark; a pre-fix save does not self-repair"
    );
}
