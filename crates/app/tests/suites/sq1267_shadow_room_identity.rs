//! SQ-1267: a Glulx probe shadow must key rooms exactly as the live session does.
//!
//! Reported symptom (from a real save under `~/.lanthorn/saves/advent.blb.save/`): "In A Valley"
//! showed a `?³` stub although Adventure has exactly two forest rooms. The saved destination
//! pool for hill-S, valley-E and valley-W alike held `[#42746 "In Forest", #55642 "In Forest",
//! #64051 <no such room on the map>]` — a THIRD id, `#64051`, which is
//! `app::roomid::synthetic_room_id("In Forest")`: the NAME hash `GlulxSession::heading_to_room`
//! falls back to while the room-lock (`glulx_roomlock`, SQ-0526) has not yet located the story's
//! `location` global. The two real forests are keyed by `roomid::glulx_room_id` — the room
//! object's own ADDRESS — once the lock resolves, so `#64051` cannot be either of them.
//!
//! # Why this reproduces only under a specific ordering
//!
//! `ShadowRecipe::store` already points a probe shadow at the LIVE session's own persistent
//! directory (`startup.rs`'s `game_dir.clone()`, mirrored here) so the shadow can read the
//! `room-global` sidecar `GlulxSession::remember_room_global` writes once the live lock
//! resolves — sharing that directory is the production shape, not a test-only crutch (see
//! [`sq1264_forest_randomization`](super::sq1264_forest_randomization)'s own `GPlay::advent`,
//! which shares the identical directory the identical way).
//!
//! The gap is TIMING. [`app::probe::ShadowProbe`] is one worker shared by every feature that
//! asks it anything — vocabulary vetting, the return-probe, this module's own random-exit
//! search — boots its shadow lazily on the FIRST question of the WHOLE SESSION, and never
//! re-reads the sidecar after boot: `GlulxSession::restore_state` only swaps VM memory, never
//! the host-side `room_lock` field (see `Engine::room_identity_state`'s doc comment). A shadow
//! whose first-ever question predates the live session's own room-lock therefore boots unlocked
//! and STAYS that way for the rest of the session, however long afterwards the live session goes
//! on to lock — and in the field, an ordinary vetted suggestion on turn one is exactly such a
//! question, long before the player has walked anywhere near a forest.
//!
//! `sq1264_forest_randomization.rs`'s own `GPlay` never hits this: its first-ever probe question
//! IS the forest one, asked only after `g_reach_hill`'s revisit-until-stable warmup — by which
//! point the live session has already locked and written the sidecar, so a shadow booting then
//! reads the right address on its very first try. `GPlay::advent` below reuses that exact
//! navigation (declared_exit needs the origin's own address cached — see
//! [`GlulxSession::declared_exit`]'s doc comment — which only happens once a room is VISITED
//! while locked, which is why the warmup exists at all) but spends the shadow's first-ever
//! question on something unrelated FIRST, mirroring the field ordering instead.
//!
//! # The fix under test
//!
//! [`Engine::room_identity_state`]/[`Engine::apply_room_identity_state`] carry the live
//! session's CURRENT room-keying state into the shadow on every restore (`probe::serve`), not
//! only at boot — see their doc comments. `random_exit_probe::note_disagreeing_destinations`
//! additionally admits a shadow-reported destination to the pool only when it is comparable to
//! the live map (SQ-1267's second line of defence): equal to the live destination, or already a
//! room on the map.
//!
//! Falsified: reverting the `apply_room_identity_state` call in `probe::serve` (or the
//! `room_identity_state`/`apply_room_identity_state` pair in `glulx_session.rs`) makes
//! [`phantom_name_hash_id_never_enters_the_destination_pool`] fail with `#64051` in the pool —
//! confirmed by hand: reverting `probe.rs`'s call turns the assertion red with exactly that id.

use std::path::PathBuf;
use std::sync::Arc;

use app::engine::{DeclaredExit, Engine};
use app::glulx_session::GlulxSession;
use app::probe::ShadowRecipe;
use app::random_exit_probe::arm_random_exit_search;
use app::roomid::synthetic_room_id;
use app::session::DeathWatch;
use app::state::AppState;
use mapper::direction::Direction;
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

/// SQ-1264's own trial value for this exact fixture/command sequence: reseeded right before the
/// hill's `south`, it lands the live walk in the forest OTHER than the declared one, guaranteeing
/// an immediate Phase-1 mismatch (`apply_turn` marks the direction random on the spot) and so a
/// Phase-2 probe on the very first south walk out of the hill.
const G_DISAGREEING_SEED: u32 = 2;

fn story() -> Option<Vec<u8>> {
    match std::fs::read(fixture_path("advent.blb")) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", fixture_path("advent.blb").display());
            None
        }
    }
}

fn recipe_in(bytes: &[u8], store: PathBuf) -> ShadowRecipe {
    ShadowRecipe {
        story_bytes: Arc::new(bytes.to_vec()),
        store,
        vfs_bytes: Arc::new(Vec::new()),
        honor_game_colours: true,
        interpreter_number: None,
        random_seed: None,
        acceleration: true,
        screen: (80, 24),
    }
}

struct GPlay {
    state: AppState,
    mapper: Mapper,
    session: GlulxSession,
    death: DeathWatch,
    /// The raw Phase-2 answer from the most recent `turn()` that armed a search, kept for tests
    /// that want to inspect what the SHADOW actually reported — `step.location` on every step,
    /// independent of `random_exit_probe::deliver`'s own pool-hygiene filter (SQ-1267's second
    /// line of defence) — so a test can tell the two fixes apart instead of one masking the
    /// other. `None` when no search armed on that turn.
    last_answer: Option<app::probe::Answer>,
}

impl GPlay {
    /// Boot Adventure (Glulx) with a real, story-shared store — the same `game_dir.clone()`
    /// shape `startup.rs` uses for its shadow recipe — then immediately spend the probe's
    /// FIRST-EVER question on something unrelated to the forest, before the live session has
    /// taken a single turn. That pins the shadow's boot moment to before the live room-lock has
    /// observed anything at all, which is the ordering the module docs above describe: in the
    /// field, whichever feature asks the shadow anything first (typically an early vetted
    /// suggestion) decides this, not the forest question itself.
    fn advent() -> Option<GPlay> {
        let bytes = story()?;
        let blorb = blorb::Blorb::parse(bytes.clone()).ok()?;
        let (_kind, exec) = blorb.executable().ok()?;
        let store = app::scratch_dir("sq1267-glulx-play");
        let s = GlulxSession::new_in(
            store.clone(), exec.to_vec(), 80, 24, true, false, false, false, (1, 1), None, &[],
            [[(None, None); 11]; 2], false, None,
        )
        .expect("Adventure (Glulx) boots");
        let mut state = AppState::default();
        state.probe.arm(recipe_in(&bytes, store));

        // The early, unrelated question: the live session has not moved at all yet, so its
        // room-lock is unlocked and the shared `room-global` sidecar does not exist. Whatever
        // the shadow answers here is discarded — only the BOOT ORDERING matters.
        assert!(state.probe.is_armed(), "recipe carries real story bytes");
        let token = state.probe.ask(&s, &["xyzzy".to_string()]).expect("the first-ever question sends");
        let answer = state.probe.settle().expect("the worker answers");
        assert_eq!(answer.token, token);

        Some(GPlay { state, mapper: Mapper::default(), session: s, death: DeathWatch::default(), last_answer: None })
    }

    /// A raw, UNTRACKED submit — no `apply_turn`, no mapper bookkeeping. Used only for the
    /// room-lock warmup below, exactly as `sq1264_forest_randomization.rs`'s `GPlay::raw` — a
    /// mid-warmup id REMAP (SQ-0526) never has to be replayed through the mapper at all, since
    /// tracking starts only once the id has stabilized.
    fn raw(&mut self, cmd: &str) {
        let _ = Engine::submit(&mut self.session, cmd);
    }

    /// Drive one live command through the full turn pipeline `sq1264_forest_randomization.rs`
    /// uses — `apply_turn` (Phase 1) first, then Phase 2's arm/settle when the move earns one —
    /// mirroring `turn::finish_command_turn`.
    fn turn(&mut self, cmd: &str) {
        let room_before = self.mapper.graph.current();
        let mut result = Engine::submit(&mut self.session, cmd);
        result.declared_exit = app::random_exit_probe::declared_exit_for_command(cmd, room_before, |o, d| {
            Engine::declared_exit(&self.session, o, d)
        });
        app::session::apply_turn(&mut self.mapper, cmd, &result, &mut self.death);
        // The Phase-2 gate is `random_exit_probe`'s own, in one place (SQ-1314) - this harness
        // used to restate it, and the copy drifted.
        app::random_exit_probe::arm_for_finished_turn(
            &mut self.state,
            &self.session,
            &mut self.mapper,
            cmd,
            room_before,
            result.declared_exit,
        );
        // Inlined `random_exit_probe::settle_random_exit_search`, so the RAW answer (every step
        // the shadow reported, before `deliver`'s own pool-hygiene filter runs) is kept for the
        // test to inspect directly. The ARMING above is production's; only the settle differs.
        if self.state.random_exit_search.is_some() {
            if let Some(answer) = self.state.probe.settle() {
                app::random_exit_probe::deliver(&mut self.state, &mut self.mapper, &answer);
                self.last_answer = Some(answer);
            } else {
                self.state.random_exit_search = None;
            }
        }

        self.state.random_exit_pre_move_save = Engine::rng_seed(&self.session)
            .map(|_| (self.mapper.graph.current().unwrap_or(0), Arc::new(Engine::save_state(&self.session))));
    }
}

/// Reach `At Hill In Road`, exactly as `sq1264_forest_randomization.rs`'s `g_reach_hill`: warm up
/// untracked until the room-lock's id stops changing (`declared_exit` needs the hill's own
/// address cached, which only happens on a visit made while LOCKED), then start tracking with one
/// `look`.
fn g_reach_hill(p: &mut GPlay) -> mapper::graph::RoomId {
    for cmd in ["in", "take lamp", "down", "west", "west"] {
        p.raw(cmd);
    }
    let mut prev = p.session.current_location().map(|l| l.number);
    for _ in 0..8 {
        p.raw("east");
        p.raw("west");
        let now = p.session.current_location().map(|l| l.number);
        if now == prev {
            break;
        }
        prev = now;
    }
    let loc = p.session.current_location().expect("standing at the hill");
    assert_eq!(loc.name, "At Hill In Road");
    p.turn("look");
    p.mapper.graph.current().expect("the hill is now the tracked current room")
}

/// The reported symptom, reproduced and falsified: a Glulx shadow whose first-ever question (of
/// the whole session) predates the live session's room-lock must not pollute a random-exit
/// destination pool with its own unlocked, NAME-hashed id once the live session goes on to lock
/// and probing for real begins.
///
/// With `Engine::room_identity_state`/`apply_room_identity_state` reverted (or `probe::serve`'s
/// call to the latter removed), this fails: the pool for the hill's `south` gains `#64051` —
/// `synthetic_room_id("In Forest")` — alongside a real forest, the exact shape the user's save
/// carried.
#[test]
fn phantom_name_hash_id_never_enters_the_destination_pool() {
    let Some(mut p) = GPlay::advent() else { return };
    let hill = g_reach_hill(&mut p);

    let phantom = synthetic_room_id("In Forest");
    let DeclaredExit::Room(forest1) = p.session.declared_exit(hill, Direction::S) else {
        panic!("expected a declared Room(_) south of the hill");
    };

    p.session.reseed_random(G_DISAGREEING_SEED);
    p.turn("south");
    let forest_a = p.mapper.graph.current().expect("landed in a forest");
    assert_eq!(
        p.mapper.graph.room(forest_a).map(|r| r.label().to_string()),
        Some("In Forest".to_string())
    );
    assert!(p.mapper.graph.is_random_exit(hill, Direction::S), "the disagreeing seed marks it random");

    // Inspect the RAW shadow answer directly — independent of `deliver`'s own pool-hygiene
    // filter (SQ-1267's second line of defence), so this specifically falsifies
    // `Engine::room_identity_state`/`apply_room_identity_state` (SQ-1267's first line): with
    // that carrying reverted, the shadow itself reports the phantom name-hash id here, filter
    // or no filter.
    let run = p.last_answer.as_ref().and_then(|a| a.run.as_ref()).expect("Phase 2 armed and answered");
    for step in &run.steps {
        if step.quit || step.escaped {
            continue;
        }
        assert_ne!(
            step.location,
            Some(phantom),
            "the shadow itself must never report the unlocked NAME hash of \"In Forest\" \
             once the live session has locked: {:?}",
            run.steps
        );
    }

    let pool = p.mapper.graph.random_destinations(hill, Direction::S);
    assert!(!pool.is_empty(), "the disagreeing walk is itself evidence, so the pool is never empty");
    assert!(
        !pool.contains(&phantom),
        "SQ-1267: the pool must never hold the unlocked NAME hash of \"In Forest\" (#{phantom}): {pool:?}"
    );
    // Every id actually in the pool must be one of the two REAL forests — `forest_a` (the live
    // landing) or `forest1` (the story's own declared exit, read from its compiled table, not
    // from the mapper — SQ-1261's own falsification test covers a disagreeing shadow attempt
    // naming a room the live map has never visited at all, which this pool design deliberately
    // keeps allowing, so a plain "already on the map" check would be too strict here too).
    for &id in pool {
        assert!(
            id == forest_a || id == forest1,
            "every pooled id for the forest exit must be one of the two real forests \
             (#{forest_a} or #{forest1}), got #{id}: {pool:?}"
        );
    }
    // The specific defect reported: at most the two real forests, never a third id.
    assert!(pool.len() <= 2, "at most the two real forest rooms, got {pool:?}");
}
