//! SQ-1506: a shadow the story KILLED was relocated, and a resurrection room is not a destination.
//!
//! Reported from play on Zork I (`stories/zork1-r88-s840726.z3`): *"our return probe is sometimes
//! dying when exploring. This is causing destination rooms to mis-detect random destinations and
//! label the map with just a direction arrow and the superscript (e.g. the new room + forest 1 in
//! zork-1) … in the cellar area and in the maze."*
//!
//! # The mechanism, read off the shadow's own step
//!
//! `cargo run -p lanthorn --example return_probe_trace -- --story zork1-r88-s840726.z3 --walk
//! 'w;e;s;e;open window;w;w;take lamp;move rug;open trap door;turn on lamp;d'` printed it whole.
//! The player walks down the trap door into the Cellar (#72), the door bars itself behind them,
//! and the return search opens looking for the way back to the Living Room (#193). Its second
//! candidate is `north`, and the shadow's reply to it was:
//!
//! ```text
//! The Troll Room
//! …
//! Conquering his fears, the troll puts you to death.
//! It appears that that last blow was too much for you. I'm afraid you are dead.
//!
//!    ****  You have died  ****
//!
//! Now, let's take a look here... Well, you probably deserve another chance. …
//!
//! Forest
//! ```
//!
//! `quit=false`, `escaped=false`, `location=Some(78)` — the Forest west of West of House, a room
//! the player HAD walked and the map therefore held. Every guard `return_probe::deliver` had was
//! about rooms the map could NOT name, so this one sailed through and
//! `Mapper::record_probed_passage` minted `Cellar —north→ Forest`: the drawn arrow labelled by
//! its direction alone, pointing at the `Forest ¹` the report names (Zork I ships four rooms
//! called `Forest`, so the map disambiguates them with superscripts).
//!
//! And it cost the map twice over, which is why the Cellar looked so bare. With a `north` edge
//! already leaving the Cellar, the player's own later walk into the Troll Room minted nothing —
//! `Mapper::observe` will not overwrite a passage that already exists — so no crossing was
//! recorded and no return search was armed from the Troll Room either. One unlucky combat round
//! in a shadow erased a real passage and its reciprocal.
//!
//! # The fix
//!
//! `app::session::turn_reports_death` is the detector the LIVE turn path has read since SQ-0259
//! to tell a resurrection from a walked passage (`Mapper::observe_relocation`, never a minted
//! edge). The shadow now reads the identical fact off its own step as `ProbeStep::died`, and
//! `ProbeStep::landing` — the one reading both probe consumers go through — answers `None` for
//! it, exactly as it does for a step that quit or escaped.
//!
//! Falsification: drop `!s.died` from `ProbeStep::landing` and
//! `the_cellars_north_probe_does_not_mint_a_passage_to_the_resurrection_forest` fails with the
//! reported edge, `Cellar —N→ Forest`, in its message.

use std::path::PathBuf;
use std::sync::Arc;

use app::engine::Engine;
use app::probe::ShadowRecipe;
use app::state::AppState;

use mapper::direction::Direction;
use mapper::graph::RoomId;
use mapper::mapper::Mapper;

// ── Zork I r88 room numbers, as `--example return_probe_trace` reports them ──
const LIVING_ROOM: RoomId = 193;
const CELLAR: RoomId = 72;
const TROLL_ROOM: RoomId = 102;
/// The room `JIGS-UP` resurrects into: the Forest west of West of House, mapped by this walk's
/// first two moves so it is a room the map already holds when the death happens.
const FOREST: RoomId = 78;

/// The exact walk the report reproduces on, in order. The first two moves are what put `FOREST`
/// on the map; without them the death would land somewhere the map could not name and the OLD
/// guard would already have caught it.
const WALK: &[&str] = &[
    "w",
    "e",
    "s",
    "e",
    "open window",
    "w",
    "w",
    "take lamp",
    "move rug",
    "open trap door",
    "turn on lamp",
    "d",
];

fn story() -> Option<Vec<u8>> {
    let path: PathBuf =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/zork1-r88-s840726.z3");
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            None
        }
    }
}

/// Drive the real story the way the turn path does — `apply_turn`, then the return search armed
/// against the crossing it just made, settled to its end. The app's own calls throughout.
struct Play {
    state: AppState,
    mapper: Mapper,
    session: Box<dyn Engine>,
    death: app::session::DeathWatch,
}

impl Play {
    fn zork1() -> Option<Play> {
        let bytes = story()?;
        let inner = match app::hints::extract_story(bytes).ok()? {
            app::hints::LoadedStory::ZCode(b) => b,
            _ => return None,
        };
        let mut s = app::session::GameSession::new_with_trace(
            inner.clone(),
            true,
            false,
            None,
            false,
            Vec::new(),
            None,
            None,
            Some((25, 80)),
        )
        .expect("zork1-r88-s840726.z3 boots without a ZError");
        s.set_strip_prompt(false);
        let mut state = AppState::default();
        state.config.return_probe = true;
        state.probe.arm(ShadowRecipe {
            story_bytes: Arc::new(inner),
            store: PathBuf::new(),
            vfs_bytes: Arc::new(Vec::new()),
            honor_game_colours: true,
            interpreter_number: None,
            random_seed: None,
            acceleration: true,
            screen: (80, 24),
        });
        let mut p = Play {
            state,
            mapper: Mapper::default(),
            session: Box::new(s),
            death: app::session::DeathWatch::default(),
        };
        let r = p.session.submit("look");
        app::session::apply_turn(&mut p.mapper, "look", &r, &mut p.death);
        Some(p)
    }

    fn turn(&mut self, cmd: &str) {
        let room_before = self.mapper.graph.current();
        let r = self.session.submit(cmd);
        app::session::apply_turn(&mut self.mapper, cmd, &r, &mut self.death);
        app::return_probe::arm_return_search(
            &mut self.state,
            &self.mapper,
            &*self.session,
            cmd,
            room_before,
            &mut app::engine::TurnSave::default(),
        );
        let _ = app::return_probe::settle_return_search(&mut self.state, &mut self.mapper);
    }

    fn edge(&self, from: RoomId, dir: Direction) -> Option<RoomId> {
        self.mapper
            .graph
            .connections()
            .iter()
            .find(|c| c.origin == from && c.dir == dir)
            .map(|c| c.dest)
    }
}

/// The drive is a fixture: name what it reached before asserting on it, so a story that stopped
/// answering (a fixture swapped for another release, a parser change) fails as a wrong SHAPE
/// rather than as a vacuous pass.
fn walked_to_the_cellar() -> Option<Play> {
    let mut p = Play::zork1()?;
    for cmd in WALK {
        p.turn(cmd);
    }
    assert_eq!(
        p.mapper.graph.current(),
        Some(CELLAR),
        "the walk must end in the Cellar; it ended in {:?}",
        p.mapper.graph.current().and_then(|r| p.mapper.graph.room(r)).map(|r| r.label().to_string())
    );
    assert!(
        p.mapper.graph.room(FOREST).is_some(),
        "the resurrection Forest (#{FOREST}) must be ON the map, or this case is testing the \
         unnameable-landing guard instead of the death guard"
    );
    assert_eq!(
        p.edge(LIVING_ROOM, Direction::Down),
        Some(CELLAR),
        "the trap door crossing is what arms the search"
    );
    Some(p)
}

/// The reported defect, on the story it was reported on.
#[test]
fn the_cellars_north_probe_does_not_mint_a_passage_to_the_resurrection_forest() {
    let Some(p) = walked_to_the_cellar() else { return };
    assert_ne!(
        p.edge(CELLAR, Direction::N),
        Some(FOREST),
        "SQ-1506: the shadow was killed by the troll and Zork I resurrected it in the Forest \
         (#{FOREST}) — a room the map holds, so the false passage `Cellar —N→ Forest` minted \
         cleanly and drew as an arrow labelled by its direction alone"
    );
    // Nothing at all is the right answer here: the only rooms `north` can reach from the Cellar
    // are the Troll Room (which kills the shadow) and, on a luckier round, nowhere the map knows.
    assert!(
        p.edge(CELLAR, Direction::N).is_none_or(|d| d == TROLL_ROOM),
        "north out of the Cellar may only ever be the Troll Room; it was {:?}",
        p.edge(CELLAR, Direction::N)
    );
}

/// The second half of the same cost, and the one the player actually sees: the false edge sat in
/// the `north` slot, so the player's own walk into the Troll Room could not mint the real one and
/// no return search was armed from there either. With the death guard in place both happen.
#[test]
fn the_real_north_passage_and_its_return_survive_the_walk_that_follows() {
    let Some(mut p) = walked_to_the_cellar() else { return };
    p.turn("n");
    assert_eq!(
        p.mapper.graph.current(),
        Some(TROLL_ROOM),
        "the player walks north into the Troll Room"
    );
    assert_eq!(
        p.edge(CELLAR, Direction::N),
        Some(TROLL_ROOM),
        "the walked crossing mints `Cellar —N→ Troll Room`; before SQ-1506 the probe's false \
         edge already occupied that slot and `observe` left it standing"
    );
    assert_eq!(
        p.edge(TROLL_ROOM, Direction::S),
        Some(CELLAR),
        "and the return search from the Troll Room finds `south` — it was never armed at all \
         while the outbound crossing was being swallowed by the false edge"
    );
}
