//! SQ-1283, round two: a restored v6 shadow must not read the previous
//! question's status band.
//!
//! # The report
//!
//! With the room-identity half fixed (`sq1283_shogun_room_identity.rs`), a
//! re-walk below decks on the Erasmus produced a map whose `Below Decks` (#13)
//! had a connection to `Bridge` (#57) in **every one** of S, E, W, SE, SW, N, NE
//! and NW — eight directions the game refuses outright — and whose `Deck` (#10)
//! carried a random-exit pool of `[13, 57, 10]` for N. The player saw an SE arrow
//! out of a room that has no SE exit.
//!
//! # The mechanism
//!
//! The SQ-1284 shape, on the Z-machine side. `probe::serve` restores the shadow
//! to the player's moment before **every** question, and Quetzal archives no
//! screen — so `GameSession::restore_state` blanks the upper window, "memory
//! restored without a screen must not be read against another moment's screen"
//! (SQ-0785). A Version 6 story has nothing in that grid: it paints its status
//! text into the v6 window model, which nothing was clearing. And Shogun's own
//! status routine repaints line 2 only when `HERE` changes.
//!
//! So a shadow whose previous question walked `up` into `Deck` still had `Deck`
//! painted after the next restore; the refused `se` printed "You can't go that
//! way" and repainted nothing; `detect_location` read the band and answered
//! `StatusName(#10)` — a room the map holds, so `return_probe::deliver` minted
//! `Below Decks -SE-> Deck`. One phantom per direction the search tried, all
//! landing on whatever room the shadow had last walked into. Traced live:
//!
//! ```text
//!   up         band BEFORE=[]            -> loc=Some((10, "Deck"))  "You open the focsle door…"
//!   south      band BEFORE=["Deck","SH"] -> loc=Some((10, "Deck"))  "You can't go that way."
//!   southeast  band BEFORE=["Deck","SH"] -> loc=Some((10, "Deck"))  "You can't go that way."
//! ```
//!
//! `record_probed_passage` refuses `from == to`, but a stale room is not `from`.
//!
//! The fix is `zvm::location::clear_v6_status_band`, called from
//! `restore_state` beside the existing `upper.blank()` — the band only, never
//! the prose window, which is v6's lower window and has never been blanked.
//!
//! Skips vacuously without the gitignored `stories/` fixture.

use std::sync::Arc;

use crate::fixture_paths::fixture_path;

use app::engine::Engine;
use app::graphics::PictSource;
use app::probe::ShadowRecipe;
use app::session::{apply_turn, DeathWatch, GameSession, InputKind};
use app::state::AppState;
use mapper::direction::{parse_direction, Direction};
use mapper::mapper::Mapper;

const SHOGUN: &str = "shogun-r322-s890706.z6";

/// `BRIDGE-OF-ERASMUS`, `erasmus.zil` — the room every phantom edge landed on.
const BRIDGE_OF_ERASMUS: u16 = 57;
/// `ON-DECK`, the Erasmus's main deck.
const ON_DECK: u16 = 10;
/// `BELOW-DECKS`, forward through the focsle door.
const BELOW_DECKS: u16 = 13;
/// The room `aft` reaches from `Deck` — the second SQ-1290 specimen.
const PASSAGEWAY: u16 = 56;

/// The nine commands the reported session was made of, from its
/// `command_history.json` — including the two the parser refused, which is part
/// of the shape: a refused turn leaves a search running.
const REPORTED_WALK: [&str; 9] =
    ["straighten wheel", "d", "forward", "u", "d", "forward", "aft", "for", "fore"];

/// Every compass direction, which is what the fan was made of. `Up` is excluded
/// deliberately: `Below Decks` really does climb to the deck, and that edge must
/// survive the fix.
const COMPASS: [Direction; 8] = [
    Direction::N,
    Direction::S,
    Direction::E,
    Direction::W,
    Direction::NE,
    Direction::NW,
    Direction::SE,
    Direction::SW,
];

fn story_bytes() -> Option<Vec<u8>> {
    let path = fixture_path(SHOGUN);
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            None
        }
    }
}

/// Boot Shogun the way `sq1283_shogun_room_identity::boot` does — the picture
/// source and the archive's own standard window, so the game lays its windows
/// out the way the player sees them and the status band read here is the real
/// one.
fn boot_live() -> Option<GameSession> {
    let path = fixture_path(SHOGUN);
    let bytes = std::fs::read(&path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session = GameSession::new_with_trace(
        bytes, true, false, None, false, picture_dims, picts.std_window(), None, None,
    )
    .expect("Shogun (v6) boots without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

/// A twin booted exactly the way `probe::boot_shadow` builds a Z-machine shadow:
/// no sound, no graphics, no standard window.
fn boot_twin(bytes: &[u8]) -> GameSession {
    GameSession::new_with_trace(bytes.to_vec(), true, false, None, false, Vec::new(), None, None, None)
        .expect("the twin boots")
}

fn advance_to_line(session: &mut GameSession, budget: usize) -> bool {
    for _ in 0..budget {
        match session.pending_input() {
            InputKind::Line => return true,
            InputKind::Char => {
                let _ = session.submit_char(13);
            }
            InputKind::Event => {
                let _ = session.submit("");
            }
        }
    }
    matches!(session.pending_input(), InputKind::Line)
}

fn recipe(bytes: &[u8]) -> ShadowRecipe {
    ShadowRecipe {
        story_bytes: Arc::new(bytes.to_vec()),
        store: std::path::PathBuf::new(),
        vfs_bytes: Arc::new(Vec::new()),
        honor_game_colours: true,
        interpreter_number: None,
        random_seed: None,
        acceleration: true,
        screen: (80, 24),
    }
}

/// The engine-level defect, with no probe seam in sight: a session restored to a
/// moment must not answer a heading-less turn with the room it walked into
/// before the restore.
#[test]
fn a_refused_move_after_a_v6_restore_reports_no_room_not_the_previous_one() {
    let Some(bytes) = story_bytes() else { return };
    let Some(mut live) = boot_live() else { return };
    assert!(advance_to_line(&mut live, 16), "Shogun reaches an in-game prompt after its menu");
    for cmd in ["d", "forward"] {
        assert!(advance_to_line(&mut live, 8), "a line prompt before {cmd:?}");
        let _ = live.submit(cmd);
    }
    let here = live.current_location().expect("the walk ends somewhere");
    assert_eq!(
        (here.number, here.name.as_str()),
        (BELOW_DECKS, "Below Decks"),
        "non-vacuity: the live session is standing below decks"
    );
    let save = Engine::save_state(&live);

    // The twin, restored to that same moment and asked the two questions in the
    // order the return search asks them: one that moves and one that is refused.
    let mut twin = boot_twin(&bytes);
    // A freshly booted Shogun is parked on its title `read_char`, and the first
    // line handed to a shadow finishes THAT rather than the restored prompt —
    // the twin prints its banner and lands on the game's own read. `serve` pays
    // the same toll on a shadow's first question and the search simply asks the
    // next candidate, so spend it here too rather than measuring it.
    Engine::restore_state(&mut twin, &save).expect("the twin takes the live snapshot");
    let _ = Engine::submit(&mut twin, "look");
    let _ = twin.take_transcript(); // …and its banner, as `probe::serve` drops it

    Engine::restore_state(&mut twin, &save).expect("the twin takes the live snapshot");
    let _ = twin.take_transcript();
    let r = Engine::submit(&mut twin, "up");
    assert!(
        r.transcript.contains("focsle door"),
        "the twin really climbed back to the deck: {:?}",
        r.transcript
    );
    let deck = twin.current_location().expect("…and Shogun repainted its band saying so");
    assert_eq!(
        deck.number, ON_DECK,
        "non-vacuity: the previous question's landing is a room the map holds, which is what \
         made the stale answer minting-worthy"
    );

    // Question two, from the same moment: a direction the game refuses.
    Engine::restore_state(&mut twin, &save).expect("the twin takes the snapshot back");
    let _ = twin.take_transcript();
    let r = Engine::submit(&mut twin, "southeast");
    assert!(
        r.transcript.contains("can't go that way"),
        "\"southeast\" is refused below decks: {:?}",
        r.transcript
    );
    let got = twin.current_location();
    assert_ne!(
        got.as_ref().map(|l| l.number),
        Some(ON_DECK),
        "SQ-1283: a restore must clear the v6 status band, or a refused move is answered with \
         the PREVIOUS question's landing and `return_probe::deliver` mints an edge to it — the \
         reported S/E/W/SE/SW/N/NE/NW fan out of Below Decks; got {got:?}"
    );
    assert!(
        got.is_none(),
        "…and with nothing repainted the honest answer is no room at all, not another one: {got:?}"
    );
}

/// The whole reported session, driven the way `turn::finish_command_turn` drives
/// one — both probes armed and settled every turn — and then read the way the
/// player read it: as a map.
#[test]
fn replaying_the_reported_walk_leaves_no_fan_out_of_below_decks() {
    let Some(bytes) = story_bytes() else { return };
    let Some(mut live) = boot_live() else { return };
    assert!(advance_to_line(&mut live, 16), "Shogun reaches an in-game prompt after its menu");

    let mut state = AppState::default();
    state.config.return_probe = true;
    state.probe.arm(recipe(&bytes));
    assert!(state.probe.is_armed(), "the recipe carries real story bytes");

    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();

    for cmd in REPORTED_WALK {
        assert!(advance_to_line(&mut live, 8), "a line prompt before {cmd:?}");
        let room_before = mapper.graph.current();
        let mut result = live.submit(cmd);
        if let (Some(origin), Some(dir)) = (room_before, parse_direction(cmd)) {
            result.declared_exit = Some(live.declared_exit(origin, dir));
        }
        apply_turn(&mut mapper, cmd, &result, &mut death);

        let mut turn_save = app::engine::TurnSave::default();
        app::return_probe::arm_return_search(
            &mut state, &mapper, &live, cmd, room_before, &mut turn_save,
        );
        // The Phase-2 shapes `finish_command_turn` arms, in its own order.
        if let (Some(origin), Some(dir), Some(live_dest)) =
            (room_before, parse_direction(cmd), mapper.graph.current())
        {
            let already_random = mapper.graph.is_random_exit(origin, dir);
            let worth_probing = live_dest != origin
                && (already_random
                    || matches!(
                        result.declared_exit,
                        Some(app::engine::DeclaredExit::Absent)
                            | Some(app::engine::DeclaredExit::Code)
                    ));
            if worth_probing {
                if let Some((saved_room, save)) = &state.random_exit_pre_move_save {
                    if *saved_room == origin {
                        let save = Arc::clone(save);
                        let kind = if already_random {
                            app::random_exit_probe::SearchKind::Upgrade
                        } else {
                            app::random_exit_probe::SearchKind::FirstWalk
                        };
                        app::random_exit_probe::arm_random_exit_search(
                            &mut state, &live, origin, dir, live_dest, kind, save,
                        );
                    }
                }
            }
        }
        if let Some(susp) = mapper.take_random_exit_suspicion() {
            let mut armed = false;
            if let Some((saved_room, save)) = &state.random_exit_pre_move_save {
                if *saved_room == susp.origin {
                    let save = Arc::clone(save);
                    app::random_exit_probe::arm_random_exit_search(
                        &mut state, &live, susp.origin, susp.dir, susp.live_dest,
                        app::random_exit_probe::SearchKind::Suspicion { old_dest: susp.old_dest },
                        save,
                    );
                    armed = state.random_exit_search.is_some();
                }
            }
            if !armed {
                mapper.resolve_suspicion_as_random(susp);
            }
        }
        // Both searches share one shadow, so drain them rather than racing the
        // event loop's per-pass dispatch.
        let _ = app::random_exit_probe::settle_random_exit_search(&mut state, &mut mapper);
        let _ = app::return_probe::settle_return_search(&mut state, &mut mapper);
        state.random_exit_pre_move_save =
            live.rng_seed().map(|_| (mapper.graph.current().unwrap_or(0), turn_save.get(&live)));
    }

    // ── Non-vacuity: this really is the reported session ────────────────────
    assert_eq!(
        mapper.graph.current(),
        Some(BELOW_DECKS),
        "the reported walk ends below decks (`meta.json`: location \"Below Decks\")"
    );
    for id in [BRIDGE_OF_ERASMUS, ON_DECK, BELOW_DECKS] {
        assert!(mapper.graph.room(id).is_some(), "the map holds room #{id}, as the report's does");
    }
    let below = mapper.graph.room(BELOW_DECKS).expect("Below Decks");
    assert!(
        !below.probed.is_empty(),
        "non-vacuity: the return search must actually have walked out of Below Decks — a run \
         that probed nothing could not fan either; probed {:?}",
        below.probed
    );
    // SQ-1290 shrank the count this can pin: before it, the search burned through eight
    // refused compass words before `up` was the first thing that worked, so this room's
    // probed list ran to several entries. Now the player's own word ("aft") is tried FIRST
    // out of Below Decks and succeeds immediately, so a single probed entry is not a weaker
    // signal of a real run — it is what a search that got the answer right on the first try
    // looks like. Non-vacuity only needs "not empty"; a specific count would be pinning the
    // OLD bug's shape.

    // ── The fan ────────────────────────────────────────────────────────────
    let edges: Vec<_> =
        mapper.graph.connections().iter().map(|c| (c.origin, c.dir, c.dest)).collect();
    let fan: Vec<_> = COMPASS
        .iter()
        .filter(|d| edges.contains(&(BELOW_DECKS, **d, BRIDGE_OF_ERASMUS)))
        .collect();
    assert!(
        fan.is_empty(),
        "SQ-1283: every one of these is a direction Shogun refuses below decks, minted from a \
         stale v6 status band; got {fan:?} in {edges:?}"
    );
    // …and not merely re-aimed at some other room the shadow happened to hold.
    // The one compass edge Below Decks legitimately has is `S`: Shogun's own
    // ship words are directions (`defs.zil`), and `parse_direction` aliases
    // AFT onto S — the focsle door back to the deck, which the walk really
    // types. Everything else out of this room is the fan.
    let strays: Vec<_> = edges
        .iter()
        .filter(|(o, _, d)| *o == BELOW_DECKS && *d != ON_DECK)
        .collect();
    assert!(
        strays.is_empty(),
        "the only room Below Decks joins is the deck above it, by the focsle door and the \
         companionway; got {strays:?}"
    );

    // ── And the pool the same staleness fed ────────────────────────────────
    let deck = mapper.graph.room(ON_DECK).expect("Deck");
    assert!(
        deck.random_exits.is_empty(),
        "the Erasmus is a deterministic ship: `Deck` must carry no random exit (the report's had \
         N with a pool of [13, 57, 10] — Below Decks, the Bridge, and Deck itself); got {:?}",
        deck.random_exits
    );

    // The real passages the walk earned must survive: this is a fix that removes
    // edges, and removing the right ones is only half of it.
    assert!(
        edges.contains(&(BELOW_DECKS, Direction::Up, ON_DECK)),
        "climbing up from below decks is a real passage and stays on the map; got {edges:?}"
    );
    assert!(
        edges.contains(&(ON_DECK, Direction::Up, BRIDGE_OF_ERASMUS)),
        "…as does the deck's own climb to the bridge; got {edges:?}"
    );
}

/// Drive an armed `return_search` to completion (or exhaustion) the way
/// `settle_return_search` does, but keep the FIRST attempt's command word —
/// which `settle_return_search` throws away and SQ-1290's pin needs to assert
/// on directly.
fn drain_search(state: &mut AppState, mapper: &mut Mapper) -> Option<String> {
    let mut first = None;
    while state.return_search.is_some() {
        if !app::return_probe::pump_return_search(state) && state.return_search.is_none() {
            break;
        }
        let Some(answer) = state.probe.settle() else { break };
        if !app::return_probe::owns(state, answer.token) {
            continue;
        }
        if first.is_none() {
            first = answer.run.as_ref().and_then(|run| run.steps.first()).map(|s| s.command.clone());
        }
        if app::return_probe::deliver(state, mapper, &answer).is_some() {
            break;
        }
    }
    first
}

/// SQ-1290: the return probe must ask the way back in the player's OWN
/// vocabulary before falling through to a portal. Reported: "if we go down to
/// the deck, then fore to below decks, the return path is immediately shown
/// as up, vs the reciprocal aft."
///
/// Traced live (`d`, `forward`): compass `south`…`northwest` are all refused
/// out of `Below Decks` — Shogun models FORE/AFT as exits distinct from the
/// compass — and before this fix `up` was the first thing that worked, so the
/// map recorded `Below Decks -Up-> Deck` and never asked `aft` at all. `aft`
/// is a real, deterministic passage back (confirmed by walking it live), so
/// once it is tried FIRST the search never reaches `up`.
///
/// Falsify: revert the `reciprocal_word` prepend in `arm_return_search` and
/// this fails with `first_word == Some("south")` and the recorded edge
/// `(BELOW_DECKS, Up, ON_DECK)` from the probe rather than `(.., S, ..)`.
#[test]
fn sq1290_reciprocal_word_wins_the_below_decks_return() {
    let Some(bytes) = story_bytes() else { return };
    let Some(mut live) = boot_live() else { return };
    assert!(advance_to_line(&mut live, 16), "Shogun reaches an in-game prompt after its menu");

    let mut state = AppState::default();
    state.config.return_probe = true;
    state.probe.arm(recipe(&bytes));

    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();

    let mut first_word = None;
    for cmd in ["straighten wheel", "d", "forward"] {
        assert!(advance_to_line(&mut live, 8), "a line prompt before {cmd:?}");
        let room_before = mapper.graph.current();
        let mut result = live.submit(cmd);
        if let (Some(origin), Some(dir)) = (room_before, parse_direction(cmd)) {
            result.declared_exit = Some(live.declared_exit(origin, dir));
        }
        apply_turn(&mut mapper, cmd, &result, &mut death);

        let mut turn_save = app::engine::TurnSave::default();
        app::return_probe::arm_return_search(&mut state, &mapper, &live, cmd, room_before, &mut turn_save);
        first_word = drain_search(&mut state, &mut mapper);
    }

    assert_eq!(
        mapper.graph.current(),
        Some(BELOW_DECKS),
        "non-vacuity: the walk really reaches Below Decks"
    );
    assert_eq!(
        first_word.as_deref(),
        Some("aft"),
        "the probe's first question out of Below Decks must be the player's own word"
    );
    let edges: Vec<_> = mapper.graph.connections().iter().map(|c| (c.origin, c.dir, c.dest)).collect();
    assert!(
        edges.contains(&(BELOW_DECKS, Direction::S, ON_DECK)),
        "the return is recorded in the aft/S slot, from the word \"aft\"; got {edges:?}"
    );
    assert!(
        !edges.contains(&(BELOW_DECKS, Direction::Up, ON_DECK)),
        "SQ-1290: `up` must not be what this probe records, even though climbing up is itself a \
         real passage the player later walks — this search's job is the aft return, and it must \
         not settle for a portal it found only because aft was never asked; got {edges:?}"
    );
}

/// SQ-1290, second specimen: from `Deck`, `aft` leads to `Passageway` (#56),
/// whose own reciprocal is `fore` (confirmed live: `fore` really does climb
/// back to `Deck`). Before this fix the probe walked north/west/east/…/up/
/// down/in and settled for `out` — the first word Shogun happened to accept —
/// without ever asking `fore`; after the fix `fore`, the player's own word
/// for the N slot, is tried FIRST and succeeds immediately.
///
/// Falsify: revert the `reciprocal_word` prepend and this fails with
/// `first_word == Some("north")` and the recorded edge
/// `(PASSAGEWAY, Out, ON_DECK)` rather than `(.., N, ..)`.
#[test]
fn sq1290_reciprocal_word_wins_the_deck_return() {
    let Some(bytes) = story_bytes() else { return };
    let Some(mut live) = boot_live() else { return };
    assert!(advance_to_line(&mut live, 16), "Shogun reaches an in-game prompt after its menu");

    let mut state = AppState::default();
    state.config.return_probe = true;
    state.probe.arm(recipe(&bytes));

    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();

    let mut first_word = None;
    for cmd in ["straighten wheel", "d", "aft"] {
        assert!(advance_to_line(&mut live, 8), "a line prompt before {cmd:?}");
        let room_before = mapper.graph.current();
        let mut result = live.submit(cmd);
        if let (Some(origin), Some(dir)) = (room_before, parse_direction(cmd)) {
            result.declared_exit = Some(live.declared_exit(origin, dir));
        }
        apply_turn(&mut mapper, cmd, &result, &mut death);

        let mut turn_save = app::engine::TurnSave::default();
        app::return_probe::arm_return_search(&mut state, &mapper, &live, cmd, room_before, &mut turn_save);
        first_word = drain_search(&mut state, &mut mapper);
    }

    assert_eq!(
        mapper.graph.current(),
        Some(PASSAGEWAY),
        "non-vacuity: the walk really reaches the room aft of Deck"
    );
    assert_eq!(
        first_word.as_deref(),
        Some("fore"),
        "the probe's first question out of Passageway must be the player's own word"
    );
    let edges: Vec<_> = mapper.graph.connections().iter().map(|c| (c.origin, c.dir, c.dest)).collect();
    assert!(
        edges.contains(&(PASSAGEWAY, Direction::N, ON_DECK)),
        "the return is recorded in the fore/N slot, from the word \"fore\"; got {edges:?}"
    );
    assert!(
        !edges.contains(&(PASSAGEWAY, Direction::Out, ON_DECK)),
        "SQ-1290: `out` must not be what this probe records — it is merely the first word \
         Shogun happens to accept once the compass runs out, tried only because the nautical \
         word was never asked first; got {edges:?}"
    );
}
