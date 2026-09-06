//! SQ-1264: Adventure's forest rooms, on both engines.
//!
//! Several rooms in Graham Nelson's `advent.inf` send the player at random to
//! one of two "In Forest" rooms — but NOT through a routine in the origin's
//! own exit table the way Lost Pig's gnome tunnels do (SQ-1257's Phase 1
//! target). `At_Hill_In_Road`'s `s_to` and `In_A_Valley`'s `e_to`/`w_to` all
//! name a perfectly ordinary FIXED room — `In_Forest_1` — as a plain declared
//! `Room(_)`. The randomness lives on the DESTINATION side: `In_Forest_1`
//! carries an `initial` routine that runs on every arrival and, half the
//! time, silently redirects the player on to `In_Forest_2` instead
//! (`if (random(2) == 1) PlayerTo(In_Forest_2, 1);`). See `crates/gvm/src/
//! world.rs`'s module docs for the mechanism restated for the Glulx reader.
//!
//! That shape means Phase 1's `declared_exit` mismatch check (`apply_turn`,
//! SQ-1257) already catches the FIRST divergence between what was declared and
//! where the player actually landed — but SQ-1257 Phase 2's upgrade path has a
//! statistical hole once a direction IS marked random: two reseeded shadow
//! attempts, with two possible destinations, agree with the live landing by
//! pure luck one time in four, upgrading the mark back to a confident (WRONG)
//! edge. SQ-1264's live-walk CONTRADICTION RULE in `apply_turn` closes that
//! hole: the next time the same direction lands somewhere that contradicts an
//! existing edge, the edge is removed and the direction re-marked random on
//! the spot, no Phase 2 needed.
//!
//! Both fixtures skip vacuously without `stories/` (gitignored). `RoomId`
//! spaces differ by engine and are asserted RELATIVELY, never by a hardcoded
//! number: the Z-machine's is `zvm`'s own object number, stable per compile;
//! Glulx's is `crate::roomid::glulx_room_id`'s hash of the room's object
//! ADDRESS (SQ-0526), and while that hash is itself stable per compile, this
//! module's own room-lock learner can take a variable number of revisits to
//! settle on it (a tie-break among candidate RAM words, order-sensitive) — so
//! every Glulx case discovers ids at run time and never assumes one.

use std::path::PathBuf;
use std::sync::Arc;

use app::engine::{DeclaredExit, Engine};
use app::glulx_session::GlulxSession;
use app::probe::ShadowRecipe;
use app::random_exit_probe::derived_seeds;
use app::session::{apply_turn, DeathWatch, GameSession, TurnResult};
use app::state::AppState;
use mapper::direction::Direction;
use mapper::graph::RoomId;
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

fn story(name: &str) -> Option<Vec<u8>> {
    match std::fs::read(fixture_path(name)) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", fixture_path(name).display());
            None
        }
    }
}

fn recipe(bytes: &[u8]) -> ShadowRecipe {
    recipe_in(bytes, PathBuf::new())
}

/// [`recipe`], with a store the shadow may READ (never write — [`GameStore::read_only`], applied
/// inside `probe::boot_shadow`). See [`GPlay::advent`]'s doc comment for why the Glulx cases need
/// this and the Z-machine ones do not.
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

/// Seeds found BY TRIAL (see the SQ-1264 investigation) by driving the REAL
/// `random_exit_probe::arm_random_exit_search`/`settle_random_exit_search` machinery end to end
/// (a pure `derived_seeds`-based prediction, checked against a fresh `south` walk in isolation,
/// undershoots: it cannot see the exact command/state history the real turn sequence produces
/// by the time the shadow's seeds are derived from the LIVE session's POST-move `rng_seed()`).
/// One pair per engine, because `zvm` and `gvm` do not consume `random()` draws identically for
/// the same command sequence, so a seed that upgrades the mark on one engine is not the seed
/// that does it on the other:
///
/// * `Z_LUCKY_SEED = 16`: the live walk lands in `In_Forest_1` (the declared room) and Phase 2's
///   reseeded shadow attempts agree, upgrading the random mark to a confident edge.
/// * `Z_DISAGREEING_SEED = 2`: the live walk alone lands in `In_Forest_2` — proof the direction
///   is still random.
/// * `G_LUCKY_SEED = 16` / `G_DISAGREEING_SEED = 2`: the same two roles on `advent.blb` — the
///   same numeric value as the Z-machine's lucky seed is coincidence, not a shared derivation.
const Z_LUCKY_SEED: u32 = 16;
const Z_DISAGREEING_SEED: u32 = 2;
/// SQ-1370's lucky STREAK, found the same way and for the same reason the two above were: a seed
/// under which the live walk lands in the non-declared forest AND both reseeded shadow attempts
/// agree with it, so the evidence never names a second room and the pool stays at one. That is
/// the shape the field report describes — the same forest a few times running — and it is what
/// used to upgrade the mark to a confident arrow on the very first re-walk. Seeds 19, 21, 23, 40,
/// 42, 44 and 46 behave identically on `advent.z6`; 17 is simply the lowest.
const Z_STREAK_SEED: u32 = 17;
/// The same role on `advent.blb`, again by trial and again not derivable from the Z-machine's:
/// `gvm` does not consume `random()` draws in step with `zvm`.
const G_STREAK_SEED: u32 = 17;
const G_LUCKY_SEED: u32 = 16;
const G_DISAGREEING_SEED: u32 = 2;

// ── Z-machine driver ─────────────────────────────────────────────────────

/// Drives `apply_turn` + a synchronously-settled Phase-2 search exactly the way
/// `turn::finish_command_turn` does — `declared_exit.rs`'s own `Play` is the precedent this
/// mirrors (see its doc comment).
struct ZPlay {
    state: AppState,
    mapper: Mapper,
    session: GameSession,
    death: DeathWatch,
    /// The story, kept so a case that booted with NO shadow can arm one later — see
    /// [`ZPlay::advent_without_a_shadow`].
    bytes: Vec<u8>,
}

impl ZPlay {
    fn advent() -> Option<ZPlay> {
        let mut p = ZPlay::advent_without_a_shadow()?;
        p.arm_shadow();
        Some(p)
    }

    /// [`ZPlay::advent`] with the shadow probe UNARMED (SQ-1370). Not a mere convenience: a
    /// session that cannot run a probe is how a random mark comes to hold a pool of exactly ONE
    /// room, which is the state the lucky-streak report was made from. `apply_turn` files the
    /// declared mismatch as a suspicion, `arm_for_finished_turn` finds no shadow to hand it to,
    /// and `Mapper::resolve_suspicion_as_random` marks the direction with only the live landing
    /// pooled (`old_dest` is `None` — there was no edge to contradict). Arm the shadow afterwards
    /// with [`ZPlay::arm_shadow`] and the re-walks that follow are ordinary Upgrade searches
    /// judging a one-room pool.
    fn advent_without_a_shadow() -> Option<ZPlay> {
        let bytes = story("advent.z6")?;
        let mut s = GameSession::new_with_trace(
            bytes.clone(), true, false, None, false, Vec::new(), None, None, Some((25, 80)),
        )
        .expect("advent.z6 boots without a ZError");
        s.set_strip_prompt(false);
        let _ = s.submit(""); // dismiss the V6 "[Press any key to start]" splash
        Some(ZPlay {
            state: AppState::default(),
            mapper: Mapper::default(),
            session: s,
            death: DeathWatch::default(),
            bytes,
        })
    }

    fn arm_shadow(&mut self) {
        let recipe = recipe(&self.bytes);
        self.state.probe.arm(recipe);
    }

    fn turn(&mut self, cmd: &str) {
        let room_before = self.mapper.graph.current();
        let mut result = self.session.submit(cmd);
        result.declared_exit = app::random_exit_probe::declared_exit_for_command(cmd, room_before, |o, d| {
            Engine::declared_exit(&self.session, o, d)
        });
        apply_turn(&mut self.mapper, cmd, &result, &mut self.death);
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
        app::random_exit_probe::settle_random_exit_search(&mut self.state, &mut self.mapper);

        self.state.random_exit_pre_move_save = self
            .session
            .rng_seed()
            .map(|_| (self.mapper.graph.current().unwrap_or(0), Arc::new(self.session.save_state())));
    }

    fn edge(&self, from: RoomId, dir: Direction) -> Option<RoomId> {
        self.mapper.graph.connections().iter().find(|c| c.origin == from && c.dir == dir).map(|c| c.dest)
    }
}

/// Reach `At Hill In Road`: boot, `look`, `west`.
fn z_reach_hill(p: &mut ZPlay) -> RoomId {
    p.turn("look");
    p.turn("west");
    let here = p.mapper.graph.current().expect("standing at the hill");
    assert_eq!(p.mapper.graph.room(here).map(|r| r.label().to_string()), Some("At Hill In Road".to_string()));
    here
}

/// Reach `In A Valley`: boot, `look`, `south` (a plain, non-forest passage from the start room).
fn z_reach_valley(p: &mut ZPlay) -> RoomId {
    p.turn("look");
    p.turn("south");
    let here = p.mapper.graph.current().expect("standing in the valley");
    assert_eq!(p.mapper.graph.room(here).map(|r| r.label().to_string()), Some("In A Valley".to_string()));
    here
}

/// Walk BACK to the hill from whichever forest the player is standing in, by REAL navigation
/// (through `p.turn`, so the live session and the mapper stay in agreement) — forest 1 has no
/// direct path to `At End Of Road` (only `e_to In_A_Valley`), forest 2 does (`n_to At_End_Of_Road`).
fn z_walk_back_to_hill(p: &mut ZPlay, hill: RoomId, forest1: RoomId) {
    let here = p.mapper.graph.current().expect("standing somewhere");
    if here == forest1 {
        p.turn("east"); // forest 1 -> valley
        p.turn("north"); // valley -> at end of road
    } else {
        p.turn("north"); // forest 2 -> at end of road
    }
    p.turn("west"); // -> at hill in road
    assert_eq!(p.mapper.graph.current(), Some(hill), "back at the hill");
}

/// Z-machine (`advent.z6`): the hill's and the valley's declared exits toward the forest are
/// plain `Room(_)`, never `Code`/`Absent` — the randomness is not visible from the ORIGIN's own
/// exit table at all (see the module docs).
#[test]
fn z6_declared_exits_toward_the_forest_are_plain_room_not_code() {
    let Some(mut p) = ZPlay::advent() else { return };
    let hill = z_reach_hill(&mut p);
    let hill_s = p.session.declared_exit(hill, Direction::S);
    let DeclaredExit::Room(forest1) = hill_s else {
        panic!("hill S must be a plain declared Room(_), got {hill_s:?}");
    };
    assert!(p.mapper.graph.room(forest1).is_none(), "forest 1 is not on the map yet — this is a STATIC table read");

    let mut p2 = ZPlay::advent().unwrap();
    let valley = z_reach_valley(&mut p2);
    assert_eq!(p2.session.declared_exit(valley, Direction::E), DeclaredExit::Room(forest1), "valley E");
    assert_eq!(p2.session.declared_exit(valley, Direction::W), DeclaredExit::Room(forest1), "valley W — the SAME declared room as E");
}

/// Z-machine: `advent.z6`'s own random-walk cycle, end to end through the REAL Phase-2 path —
/// the Z-machine mirror of [`blb_forest_random_walk_stays_marked_once_the_pool_holds_both_forests`],
/// at the Z-machine's own trial seeds.
///
/// # What changed here, and why (SQ-1266)
///
/// This case used to carry a note saying `advent.z6` could not exercise Phase 2 at all: that it
/// was "a V6Lib private beta test compile whose OWN init code writes a runtime-random value into
/// the header's release-number field", so Quetzal's IFhd validation refused every restore into a
/// separately-booted shadow and `Answer::run` was always `None`. The observation was real — the
/// release word genuinely differed between the live session and a fresh boot — but the CAUSE was
/// ours. `zvm`'s `supply_line` completed a `read_char` suspension as though it were a `read`,
/// and `read_char` leaves the text buffer address at zero, so `ZPlay::advent`'s own
/// `submit("")` splash dismissal wrote the typed line to absolute address 1: the release word.
/// The "random" release was the next command's letters (`0x6C6F` — `"lo"`, from `look`). See
/// `sq1266_v6_shadow_restore.rs`; the fix is `PendingInput::line_read` plus `supply_line`'s
/// early return.
///
/// So the shadow restores here now, and this case asserts the working path instead of a
/// workaround. Two concrete consequences:
///
/// * The `unmark_random_exit` call that used to stand in for "the Phase-2 upgrade this fixture
///   cannot exercise" is gone — a real reseeded shadow now runs on the very first mismatch.
/// * The pool holds BOTH forests from that first mismatch onward (`note_disagreeing_destinations`,
///   SQ-1261, pooling what the shadow itself saw), which is precisely the pool≥2 shape SQ-1269's
///   flicker fix is about. The old step 2 — a lucky-seed walk minting a confident edge — is
///   therefore no longer the behaviour under test on this story either: the mark stands, exactly
///   as it does on `advent.blb`.
#[test]
fn z6_forest_random_walk_stays_marked_once_the_pool_holds_both_forests() {
    let Some(mut p) = ZPlay::advent() else { return };
    let hill = z_reach_hill(&mut p);
    let DeclaredExit::Room(forest1) = p.session.declared_exit(hill, Direction::S) else {
        panic!("expected a declared Room(_) south of the hill");
    };

    // ── 1: first walk, reseeded to land in the OTHER forest — an immediate declared_exit
    // mismatch. `apply_turn` stashes a suspicion, `ZPlay::turn` arms the Phase-2 search for it,
    // and the shadow's own reseeded attempts add whatever they saw. ──
    p.session.reseed_random(Z_DISAGREEING_SEED);
    p.turn("south");
    let forest2 = p.mapper.graph.current().expect("landed somewhere");
    assert_ne!(forest2, forest1, "the disagreeing seed must land in the OTHER forest");
    assert!(p.mapper.graph.is_random_exit(hill, Direction::S), "marked random on the very first mismatch");
    assert_eq!(p.edge(hill, Direction::S), None, "no edge minted");
    assert!(
        p.mapper.graph.random_destinations(hill, Direction::S).contains(&forest2),
        "at least the live landing is recorded: {:?}",
        p.mapper.graph.random_destinations(hill, Direction::S)
    );

    // Back to the hill (real navigation, not mapper bookkeeping — see `z_walk_back_to_hill`).
    z_walk_back_to_hill(&mut p, hill, forest1);

    // Measured on this exact fixture/seed pair (both fixed, so this is deterministic, not
    // flaky), and the same result the Glulx build gives: the very first mismatch's own shadow
    // attempts already see BOTH forests. Before SQ-1266 the shadow could not restore at all and
    // this pool was size 1.
    assert_eq!(
        p.mapper.graph.random_destinations(hill, Direction::S).len(),
        2,
        "non-vacuity guard: the real Phase-2 shadow ran and pooled what it saw"
    );

    // ── 2: the LUCKY seed lands at the DECLARED room. Pre-SQ-1269 an agreeing pair upgraded the
    // mark to a confident edge; the flicker fix keeps it marked, because the pool alone already
    // proves the direction varies. ──
    p.session.reseed_random(Z_LUCKY_SEED);
    p.turn("south");
    assert_eq!(p.mapper.graph.current(), Some(forest1), "the lucky seed lands in forest 1");
    assert!(
        p.mapper.graph.is_random_exit(hill, Direction::S),
        "SQ-1269: the pool already held both forests, so a single agreeing pair must not upgrade it"
    );
    assert_eq!(p.edge(hill, Direction::S), None, "SQ-1269: still no edge — the mark stands");

    // Back to the hill again (forest 1 this time).
    z_walk_back_to_hill(&mut p, hill, forest1);

    // ── 3: the DISAGREEING seed again — an already-marked re-walk, landing in forest 2 and
    // adding nothing new to a pool that already names both. ──
    p.session.reseed_random(Z_DISAGREEING_SEED);
    p.turn("south");
    assert_eq!(p.mapper.graph.current(), Some(forest2), "the disagreeing seed lands in forest 2 again");
    assert!(p.mapper.graph.is_random_exit(hill, Direction::S), "still marked random");
    assert_eq!(p.edge(hill, Direction::S), None, "still no edge");
    let pool = p.mapper.graph.random_destinations(hill, Direction::S);
    assert!(pool.contains(&forest1) && pool.contains(&forest2), "both forests are in the pool: {pool:?}");
    assert_eq!(
        mapper::matrix::classify(&p.mapper.graph, hill, Direction::S),
        mapper::matrix::MatrixCell::Random { destinations: 2 },
        "the matrix reads `?²`"
    );
}

/// SQ-1370, rule 1 on the Z-machine: once the hill's south is marked with both forests pooled,
/// the FIRST walk of the valley's west — a direction the map has never seen — is marked random on
/// the spot, with the pool copied. No second, contradicting walk, and no confident arrow drawn in
/// between.
///
/// The valley's `w_to` declares `In_Forest_1` exactly as the hill's `s_to` does, so this case
/// reseeds to LAND there: a landing in the other forest would be an ordinary declared mismatch
/// (SQ-1257 Phase 1's own suspicion path, which already marks on the first walk) and would prove
/// nothing about this rule. What is under test is the walk that agrees with everything the story
/// declared and is random all the same.
#[test]
fn z6_a_new_direction_into_a_known_forest_is_marked_random_on_its_first_walk() {
    let Some(mut p) = ZPlay::advent() else { return };
    let hill = z_reach_hill(&mut p);
    let DeclaredExit::Room(forest1) = p.session.declared_exit(hill, Direction::S) else {
        panic!("expected a declared Room(_) south of the hill");
    };

    // The hill's south, marked random the way the suite above pins it.
    p.session.reseed_random(Z_DISAGREEING_SEED);
    p.turn("south");
    let forest2 = p.mapper.graph.current().expect("landed somewhere");
    assert_ne!(forest2, forest1, "the disagreeing seed must land in the OTHER forest");
    assert!(p.mapper.graph.is_random_exit(hill, Direction::S), "the hill's south is marked");
    let known: Vec<RoomId> = p.mapper.graph.random_destinations(hill, Direction::S).to_vec();
    assert_eq!(known.len(), 2, "non-vacuity guard: the pool this rule propagates names both forests");

    // Round to the valley by real navigation: back to the hill, east to At End Of Road, south.
    z_walk_back_to_hill(&mut p, hill, forest1);
    p.turn("east");
    p.turn("south");
    let valley = p.mapper.graph.current().expect("standing somewhere");
    assert_eq!(
        p.mapper.graph.room(valley).map(|r| r.label().to_string()),
        Some("In A Valley".to_string()),
        "the route reached In A Valley"
    );
    assert_eq!(p.session.declared_exit(valley, Direction::W), DeclaredExit::Room(forest1), "valley W");
    assert!(!p.mapper.graph.is_random_exit(valley, Direction::W), "west out of the valley is untouched so far");

    // ── The first walk of the valley's west, landing at the DECLARED room. ──
    p.session.reseed_random(Z_LUCKY_SEED);
    p.turn("west");
    let landed = p.mapper.graph.current().expect("landed somewhere");
    assert_eq!(landed, forest1, "the lucky seed lands at the declared forest — no mismatch to catch this");
    assert!(
        p.mapper.graph.is_random_exit(valley, Direction::W),
        "SQ-1370: a first walk into a room the map already knows a random exit reaches is marked at once"
    );
    assert_eq!(p.edge(valley, Direction::W), None, "and no confident arrow is drawn for it");
    let pool = p.mapper.graph.random_destinations(valley, Direction::W);
    assert!(
        pool.contains(&forest1) && pool.contains(&forest2),
        "the pool is copied from the one the hill earned: {pool:?}"
    );
}

/// SQ-1370, rule 2 on the Z-machine — the reported half: *"sometimes we remove the random
/// pointers if we happen to get lucky and have the same forest picked a few times in a row."*
///
/// # The streak, reproduced
///
/// Two things have to line up, and both are ordinary play. The mark has to hold a pool of ONE
/// room, which is what a session with no shadow armed produces ([`ZPlay::advent_without_a_shadow`]):
/// the declared mismatch has no edge to contradict, so `resolve_suspicion_as_random` pools only
/// the live landing. Then the same forest has to come up again — live AND on both reseeded shadow
/// attempts — because `deliver_upgrade`'s SQ-1269 pool guard is the only other thing standing in
/// the way, and a second distinct forest anywhere in the evidence closes the window for good.
///
/// [`Z_STREAK_SEED`] is a seed where exactly that happens on `advent.z6`. Before this quest the
/// mark was gone on the FIRST such walk: pool of one, one agreeing pair, upgrade, confident arrow
/// south out of the hill into a forest the story picks by coin flip. Now it takes
/// [`app::random_exit_probe::AGREEING_WALKS_TO_UPGRADE`] of them in a row — and walk 3 below
/// pins that the escape hatch still works, because a direction that has genuinely stopped
/// wandering (Lost Pig's gnome leading the player back out) must still be able to become an arrow.
#[test]
fn z6_a_lucky_streak_of_the_same_forest_does_not_clear_the_mark() {
    let Some(mut p) = ZPlay::advent_without_a_shadow() else { return };
    let hill = z_reach_hill(&mut p);
    let DeclaredExit::Room(forest1) = p.session.declared_exit(hill, Direction::S) else {
        panic!("expected a declared Room(_) south of the hill");
    };

    p.session.reseed_random(Z_STREAK_SEED);
    p.turn("south");
    let forest2 = p.mapper.graph.current().expect("landed somewhere");
    assert_ne!(forest2, forest1, "the streak seed lands in the OTHER forest, so this is a mismatch");
    assert!(p.mapper.graph.is_random_exit(hill, Direction::S), "marked, with no shadow involved");
    assert_eq!(
        p.mapper.graph.random_destinations(hill, Direction::S),
        &[forest2],
        "a pool of exactly ONE room: no probe ran, so only the live landing is in it"
    );

    // Now the shadow is armed and every walk of the marked direction is an Upgrade search.
    p.arm_shadow();
    for walk in 1..=2 {
        z_walk_back_to_hill(&mut p, hill, forest1);
        p.session.reseed_random(Z_STREAK_SEED);
        p.turn("south");
        assert_eq!(p.mapper.graph.current(), Some(forest2), "walk {walk}: the same forest again");
        assert!(
            p.mapper.graph.is_random_exit(hill, Direction::S),
            "walk {walk}: SQ-1370 — a lucky streak this short is not evidence the story stopped \
             randomising, and before this quest the mark was gone on walk 1"
        );
        assert_eq!(
            p.mapper.graph.random_destinations(hill, Direction::S),
            &[forest2],
            "walk {walk} non-vacuity guard: nothing disagreed, so the pool is still one room and \
             SQ-1269's guard is NOT what is keeping the mark"
        );
        assert_eq!(p.edge(hill, Direction::S), None, "walk {walk}: and no arrow is drawn");
    }

    // The third agreement in a row IS acted on — one chance in sixty-four for a two-destination
    // exit, and the only way a direction that has genuinely stopped wandering can become an arrow
    // again. Pinned so the constant above is not silently raised into "never".
    z_walk_back_to_hill(&mut p, hill, forest1);
    p.session.reseed_random(Z_STREAK_SEED);
    p.turn("south");
    assert_eq!(
        app::random_exit_probe::AGREEING_WALKS_TO_UPGRADE,
        3,
        "this case walks the streak out by hand; it has to know how long the streak is"
    );
    assert!(!p.mapper.graph.is_random_exit(hill, Direction::S), "the third agreement upgrades");
    assert_eq!(p.edge(hill, Direction::S), Some(forest2), "and mints the edge it agreed on");
}

/// Z-machine: `In Forest`'s own W/N/S all declare (and, walked under many reseeds, always
/// deliver) a self-loop — the randomization lives only in ARRIVING at the room, never in moving
/// within it once there (verified: Inform 6's move engine never re-invokes a room's `initial`
/// when `next_loc == location`, so a same-room "move" is a no-op as far as that hook is
/// concerned).
#[test]
fn z6_forest_self_loop_directions_are_always_deterministic() {
    let Some(mut p) = ZPlay::advent() else { return };
    let _hill = z_reach_hill(&mut p);
    p.session.reseed_random(Z_LUCKY_SEED); // lands in forest 1, per the lucky-seed table above
    p.turn("south");
    let forest1 = p.mapper.graph.current().expect("landed in a forest");

    for (word, _dir) in [("north", Direction::N), ("south", Direction::S), ("west", Direction::W)] {
        for seed in 0u32..15 {
            p.session.reseed_random(seed.wrapping_mul(0x2545_F491).max(1));
            p.turn(word);
            assert_eq!(
                p.mapper.graph.current(),
                Some(forest1),
                "{word} out of the forest must always loop back, seed {seed}"
            );
        }
    }
    assert_eq!(
        p.mapper.graph.self_loops(forest1),
        vec![Direction::N, Direction::S, Direction::W],
        "recorded as true self-loops"
    );
    assert!(!p.mapper.graph.is_random_exit(forest1, Direction::N));
    assert!(!p.mapper.graph.is_random_exit(forest1, Direction::S));
    assert!(!p.mapper.graph.is_random_exit(forest1, Direction::W));
}

/// Z-machine: an ORDINARY fixed passage near the forests (the hill's own EAST, back to `At End
/// Of Road`) keeps its edge across many reseeded attempts — this suite's negative control, so a
/// blanket "everything near the forest reads random" bug would be caught here too.
#[test]
fn z6_a_fixed_passage_near_the_forest_keeps_its_edge_under_every_seed() {
    let Some(mut p) = ZPlay::advent() else { return };
    let hill = z_reach_hill(&mut p);
    for seed in 0u32..10 {
        p.session.reseed_random(seed.wrapping_mul(0x9E37_79B9).max(1));
        p.turn("east");
        let end_of_road = p.mapper.graph.current().expect("landed somewhere");
        assert_eq!(p.mapper.graph.room(end_of_road).map(|r| r.label().to_string()), Some("At End Of Road".to_string()));
        assert!(!p.mapper.graph.is_random_exit(hill, Direction::E), "seed {seed}: never marked random");
        p.turn("west"); // real navigation back to the hill (End Of Road's w_to)
        assert_eq!(p.mapper.graph.current(), Some(hill), "seed {seed}: back at the hill");
    }
}

// ── Glulx driver ──────────────────────────────────────────────────────────

struct GPlay {
    state: AppState,
    mapper: Mapper,
    session: GlulxSession,
    death: DeathWatch,
    /// The blorb and the store the shadow must share, kept so a case that booted with NO shadow
    /// can arm one later — see [`ZPlay::advent_without_a_shadow`] for what that is for.
    bytes: Vec<u8>,
    store: PathBuf,
}

impl GPlay {
    /// A REAL (temp) store directory, shared between the live session and its shadow's
    /// [`ShadowRecipe::store`] — required for Phase 2 to work at all here. `GlulxSession`'s
    /// room-lock (SQ-0526) is host-side bookkeeping, never carried in the VM snapshot a shadow
    /// restores: a shadow that boots with NO store starts its OWN lock from scratch and reports
    /// its very first move's room under `heading_to_room`'s NAME hash — numerically nothing like
    /// the live session's ADDRESS hash — so every comparison `random_exit_probe::judge` makes
    /// would read as a disagreement regardless of what the game actually did. Pointing both
    /// sessions at the same `room-global` sidecar (`GlulxSession::remember_room_global`, written
    /// once the live lock resolves) lets the shadow read the SAME learned address at its own
    /// boot and report rooms in the same id space from its very first move.
    fn advent() -> Option<GPlay> {
        let mut p = GPlay::advent_without_a_shadow()?;
        p.arm_shadow();
        Some(p)
    }

    /// [`GPlay::advent`] with the shadow probe UNARMED — the Glulx half of
    /// [`ZPlay::advent_without_a_shadow`], for the same reason.
    fn advent_without_a_shadow() -> Option<GPlay> {
        let bytes = story("advent.blb")?;
        let blorb = blorb::Blorb::parse(bytes.clone()).ok()?;
        let (_kind, exec) = blorb.executable().ok()?;
        let store = app::scratch_dir("sq1264-glulx-play");
        let s = GlulxSession::new_in(
            store.clone(), exec.to_vec(), 80, 24, true, false, false, false, (1, 1), None, &[],
            [[(None, None); 11]; 2], false, None,
        )
        .expect("Adventure (Glulx) boots");
        Some(GPlay {
            state: AppState::default(),
            mapper: Mapper::default(),
            session: s,
            death: DeathWatch::default(),
            bytes,
            store,
        })
    }

    fn arm_shadow(&mut self) {
        let recipe = recipe_in(&self.bytes, self.store.clone());
        self.state.probe.arm(recipe);
    }

    /// A raw, UNTRACKED submit — no `apply_turn`, no mapper bookkeeping. Used only for the
    /// room-lock warmup below, so a mid-warmup id REMAP (SQ-0526) never has to be replayed
    /// through the mapper at all: tracking starts only once the id has stabilized.
    fn raw(&mut self, cmd: &str) {
        let _ = Engine::submit(&mut self.session, cmd);
    }

    fn turn(&mut self, cmd: &str) {
        let room_before = self.mapper.graph.current();
        let mut result = Engine::submit(&mut self.session, cmd);
        result.declared_exit = app::random_exit_probe::declared_exit_for_command(cmd, room_before, |o, d| {
            Engine::declared_exit(&self.session, o, d)
        });
        apply_turn(&mut self.mapper, cmd, &result, &mut self.death);
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
        app::random_exit_probe::settle_random_exit_search(&mut self.state, &mut self.mapper);

        self.state.random_exit_pre_move_save = Engine::rng_seed(&self.session)
            .map(|_| (self.mapper.graph.current().unwrap_or(0), Arc::new(Engine::save_state(&self.session))));
    }

    fn edge(&self, from: RoomId, dir: Direction) -> Option<RoomId> {
        self.mapper.graph.connections().iter().find(|c| c.origin == from && c.dir == dir).map(|c| c.dest)
    }
}

/// Reach `At Hill In Road` and wait for the room-lock to STOP changing its mind about the id
/// (see the module docs) before handing control to `apply_turn`/the mapper at all.
fn g_reach_hill(p: &mut GPlay) -> RoomId {
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
    // NOW start tracking: one `look` through `apply_turn` seeds the mapper with the STABLE id.
    p.turn("look");
    p.mapper.graph.current().expect("the hill is now the tracked current room")
}

/// Reach `In A Valley` via `At End Of Road` (`south` from there — a plain, non-forest passage),
/// stabilized the same way as [`g_reach_hill`]. Goes by way of the SAME `in`/`take lamp`/`down`/
/// `west` warmup that reliably locks the room-lock for the hill case, rather than trying to lock
/// straight off the boot room's single transition — one room change is thin evidence for the
/// learner (SQ-0526) to commit to a RAM word from, and this suite measured that path locking onto
/// a not-yet-stable id where the hill's four-room warmup does not.
fn g_reach_valley(p: &mut GPlay) -> RoomId {
    for cmd in ["in", "take lamp", "down", "west"] {
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
    assert_eq!(p.session.current_location().map(|l| l.name), Some("At End Of Road".to_string()));
    p.raw("south");
    let mut prev = p.session.current_location().map(|l| l.number);
    for _ in 0..8 {
        p.raw("north");
        p.raw("south");
        let now = p.session.current_location().map(|l| l.number);
        if now == prev {
            break;
        }
        prev = now;
    }
    let loc = p.session.current_location().expect("standing in the valley");
    assert_eq!(loc.name, "In A Valley");
    p.turn("look");
    p.mapper.graph.current().expect("the valley is now the tracked current room")
}

/// Glulx (`advent.blb`): the SAME plain-`Room(_)` shape as the Z-machine build — proves the
/// `gvm::world` seam (SQ-1264) recovers the identical fact from the Glulx object table.
#[test]
fn blb_declared_exits_toward_the_forest_are_plain_room_not_code() {
    let Some(mut p) = GPlay::advent() else { return };
    let hill = g_reach_hill(&mut p);
    let hill_s = p.session.declared_exit(hill, Direction::S);
    let DeclaredExit::Room(forest1) = hill_s else {
        panic!("hill S must be a plain declared Room(_), got {hill_s:?}");
    };

    let mut p2 = GPlay::advent().unwrap();
    let valley = g_reach_valley(&mut p2);
    assert_eq!(p2.session.declared_exit(valley, Direction::E), DeclaredExit::Room(forest1), "valley E");
    assert_eq!(p2.session.declared_exit(valley, Direction::W), DeclaredExit::Room(forest1), "valley W — the SAME declared room as E");
}

/// Glulx: the full random-walk cycle end to end — the Glulx mirror of
/// [`z6_forest_random_walk_stays_marked_once_the_pool_holds_both_forests`], at Glulx's own trial
/// seeds ([`G_LUCKY_SEED`]/[`G_DISAGREEING_SEED`] — `zvm` and `gvm` do not consume `random()`
/// draws identically for the same command sequence, so these are not the Z-machine's seeds).
///
/// SQ-1269 real-game specimen of the flicker fix — and, since SQ-1266 fixed the `zvm` defect
/// that kept every Version 6 shadow from restoring, no longer the only one: the Z-machine case
/// above now reaches the identical shape. Glulx's shadow completes real Phase-2/Suspicion
/// probes here, and on this fixture the VERY FIRST mismatch's own shadow attempts already see
/// BOTH forests — the pool is `[a, b]`, size 2, the moment it is first marked. That is exactly the
/// shape SQ-1269's flicker fix (`deliver_upgrade`'s pool≥2 check) exists for: a later re-walk that
/// happens to AGREE with the live landing on both reseeded attempts must not flip the mark back to
/// a confident edge, because the pool alone already proves the direction varies. Before SQ-1269
/// this test pinned the OPPOSITE — a `G_LUCKY_SEED` walk clearing the mark — which was the
/// statistical hole itself, caught here on a real story rather than only the synthetic
/// `random_exit_probe` unit tests.
#[test]
fn blb_forest_random_walk_stays_marked_once_the_pool_holds_both_forests() {
    let Some(mut p) = GPlay::advent() else { return };
    let hill = g_reach_hill(&mut p);
    let DeclaredExit::Room(forest1) = p.session.declared_exit(hill, Direction::S) else {
        panic!("expected a declared Room(_) south of the hill");
    };

    // ── 1: first walk, reseeded to land in the OTHER forest. `already_random` is read AFTER
    // `apply_turn` marks it (same ordering `turn::finish_command_turn` uses), so Phase 2 ALSO
    // arms on this very first mismatch — its own reseeded shadow attempts may land in either
    // forest too and add their own (real) evidence, so the destination set is checked with
    // `contains`, not exact equality. ──
    p.session.reseed_random(G_DISAGREEING_SEED);
    p.turn("south");
    let forest2 = p.mapper.graph.current().expect("landed somewhere");
    assert_ne!(forest2, forest1, "the disagreeing seed must land in the OTHER forest");
    assert!(p.mapper.graph.is_random_exit(hill, Direction::S), "marked random on the very first mismatch");
    assert_eq!(p.edge(hill, Direction::S), None, "no edge minted");
    assert!(
        p.mapper.graph.random_destinations(hill, Direction::S).contains(&forest2),
        "at least the live landing is recorded: {:?}",
        p.mapper.graph.random_destinations(hill, Direction::S)
    );

    // Walk back to the hill (forest 2 has a direct path north; forest 1's own "north" is a
    // self-loop, per the declared exits above, so it needs the east-then-north detour instead).
    p.turn("north");
    let at_end_of_road = p.mapper.graph.room(p.mapper.graph.current().unwrap_or(0))
        .map(|r| r.label().to_string())
        == Some("At End Of Road".to_string());
    if !at_end_of_road {
        p.turn("east"); // forest1 -> valley
        p.turn("north"); // valley -> at end of road
    }
    p.turn("west"); // -> hill
    assert_eq!(p.mapper.graph.current(), Some(hill), "back at the hill");

    // Measured on this exact fixture/seed pair (both fixed, so this is deterministic, not
    // flaky): the very first mismatch's own shadow attempts already see BOTH forests, so the
    // pool is size 2 before step 2 even runs.
    assert_eq!(
        p.mapper.graph.random_destinations(hill, Direction::S).len(),
        2,
        "non-vacuity guard: this test is specifically about the pool≥2 shape"
    );

    // ── 2: the LUCKY seed lands at the DECLARED room and, pre-SQ-1269, upgraded the mark to a
    // confident edge — the statistical hole this quest closes. SQ-1269's flicker fix keeps it
    // marked instead: the pool already proves the direction varies, so one agreeing pair must
    // not undo that. ──
    p.session.reseed_random(G_LUCKY_SEED);
    p.turn("south");
    assert_eq!(p.mapper.graph.current(), Some(forest1), "the lucky seed lands in forest 1");
    assert!(
        p.mapper.graph.is_random_exit(hill, Direction::S),
        "SQ-1269: the pool already held both forests, so a single agreeing pair must not upgrade it"
    );
    assert_eq!(p.edge(hill, Direction::S), None, "SQ-1269: still no edge — the mark stands");

    // Back to the hill (forest 1 this time).
    p.turn("east"); // -> valley
    p.turn("north"); // -> at end of road
    p.turn("west"); // -> hill
    assert_eq!(p.mapper.graph.current(), Some(hill));

    // ── 3: the DISAGREEING seed again — still no edge to contradict (the mark never lifted), so
    // this is simply another already-marked re-walk, landing in forest 2 again and adding nothing
    // new to a pool that already names both. ──
    p.session.reseed_random(G_DISAGREEING_SEED);
    p.turn("south");
    assert_eq!(p.mapper.graph.current(), Some(forest2), "the disagreeing seed lands in forest 2 again");
    assert!(p.mapper.graph.is_random_exit(hill, Direction::S), "still marked random");
    assert_eq!(p.edge(hill, Direction::S), None, "still no edge");
    let pool = p.mapper.graph.random_destinations(hill, Direction::S);
    assert!(pool.contains(&forest1) && pool.contains(&forest2), "both forests are in the pool: {pool:?}");
}

/// Walk back to the hill from whichever forest the player is standing in, by real navigation:
/// forest 2 goes north to `At End Of Road` directly, forest 1's own north is a self-loop and needs
/// the east-then-north detour through the valley. Then west. The Glulx mirror of
/// [`z_walk_back_to_hill`], which knows the two forests apart by id; this one asks the map what it
/// is standing in, since a Glulx id is only ever discovered at run time.
fn g_walk_back_to_hill(p: &mut GPlay, hill: RoomId) {
    p.turn("north");
    let at_end_of_road = p
        .mapper
        .graph
        .room(p.mapper.graph.current().unwrap_or(0))
        .map(|r| r.label().to_string())
        == Some("At End Of Road".to_string());
    if !at_end_of_road {
        p.turn("east"); // forest 1 -> valley
        p.turn("north"); // valley -> at end of road
    }
    p.turn("west");
    assert_eq!(p.mapper.graph.current(), Some(hill), "back at the hill");
}

/// SQ-1370, rule 1 on Glulx — the mirror of
/// [`z6_a_new_direction_into_a_known_forest_is_marked_random_on_its_first_walk`], on `advent.blb`.
#[test]
fn blb_a_new_direction_into_a_known_forest_is_marked_random_on_its_first_walk() {
    let Some(mut p) = GPlay::advent() else { return };
    let hill = g_reach_hill(&mut p);
    let DeclaredExit::Room(forest1) = p.session.declared_exit(hill, Direction::S) else {
        panic!("expected a declared Room(_) south of the hill");
    };

    p.session.reseed_random(G_DISAGREEING_SEED);
    p.turn("south");
    let forest2 = p.mapper.graph.current().expect("landed somewhere");
    assert_ne!(forest2, forest1, "the disagreeing seed must land in the OTHER forest");
    assert!(p.mapper.graph.is_random_exit(hill, Direction::S), "the hill's south is marked");
    assert_eq!(
        p.mapper.graph.random_destinations(hill, Direction::S).len(),
        2,
        "non-vacuity guard: the pool this rule propagates names both forests"
    );

    g_walk_back_to_hill(&mut p, hill);
    p.turn("east");
    p.turn("south");
    let valley = p.mapper.graph.current().expect("standing somewhere");
    assert_eq!(
        p.mapper.graph.room(valley).map(|r| r.label().to_string()),
        Some("In A Valley".to_string()),
        "the route reached In A Valley"
    );
    assert_eq!(p.session.declared_exit(valley, Direction::W), DeclaredExit::Room(forest1), "valley W");

    p.session.reseed_random(G_LUCKY_SEED);
    p.turn("west");
    assert_eq!(
        p.mapper.graph.current(),
        Some(forest1),
        "the lucky seed lands at the declared forest — no mismatch to catch this"
    );
    assert!(
        p.mapper.graph.is_random_exit(valley, Direction::W),
        "SQ-1370: marked on the FIRST walk, from the pool the hill already earned"
    );
    assert_eq!(p.edge(valley, Direction::W), None, "and no confident arrow is drawn for it");
    let pool = p.mapper.graph.random_destinations(valley, Direction::W);
    assert!(pool.contains(&forest1) && pool.contains(&forest2), "the pool is copied: {pool:?}");
}

/// SQ-1370, rule 2 on Glulx — the mirror of
/// [`z6_a_lucky_streak_of_the_same_forest_does_not_clear_the_mark`], at Glulx's own trial seed.
#[test]
fn blb_a_lucky_streak_of_the_same_forest_does_not_clear_the_mark() {
    let Some(mut p) = GPlay::advent_without_a_shadow() else { return };
    let hill = g_reach_hill(&mut p);
    let DeclaredExit::Room(forest1) = p.session.declared_exit(hill, Direction::S) else {
        panic!("expected a declared Room(_) south of the hill");
    };

    p.session.reseed_random(G_STREAK_SEED);
    p.turn("south");
    let forest2 = p.mapper.graph.current().expect("landed somewhere");
    assert_ne!(forest2, forest1, "the streak seed lands in the OTHER forest, so this is a mismatch");
    assert!(p.mapper.graph.is_random_exit(hill, Direction::S), "marked, with no shadow involved");
    assert_eq!(
        p.mapper.graph.random_destinations(hill, Direction::S),
        &[forest2],
        "a pool of exactly ONE room: no probe ran, so only the live landing is in it"
    );

    p.arm_shadow();
    for walk in 1..=2 {
        g_walk_back_to_hill(&mut p, hill);
        p.session.reseed_random(G_STREAK_SEED);
        p.turn("south");
        assert_eq!(p.mapper.graph.current(), Some(forest2), "walk {walk}: the same forest again");
        assert!(
            p.mapper.graph.is_random_exit(hill, Direction::S),
            "walk {walk}: SQ-1370 — before this quest the mark was gone on walk 1"
        );
        assert_eq!(
            p.mapper.graph.random_destinations(hill, Direction::S),
            &[forest2],
            "walk {walk} non-vacuity guard: nothing disagreed, so SQ-1269's pool guard is not \
             what is keeping the mark"
        );
        assert_eq!(p.edge(hill, Direction::S), None, "walk {walk}: and no arrow is drawn");
    }

    g_walk_back_to_hill(&mut p, hill);
    p.session.reseed_random(G_STREAK_SEED);
    p.turn("south");
    assert!(!p.mapper.graph.is_random_exit(hill, Direction::S), "the third agreement upgrades");
    assert_eq!(p.edge(hill, Direction::S), Some(forest2), "and mints the edge it agreed on");
}

/// Glulx: the seed derivation itself never repeats — same guard as `declared_exit.rs`'s, run
/// again here since this suite depends on it holding for the SAME trial seeds on both engines.
#[test]
fn derived_seeds_never_collide_for_the_trial_seeds_this_suite_uses() {
    for live in [G_LUCKY_SEED, G_DISAGREEING_SEED] {
        let [a, b] = derived_seeds(live);
        assert_ne!(a, live);
        assert_ne!(b, live);
        assert_ne!(a, b);
    }
}
