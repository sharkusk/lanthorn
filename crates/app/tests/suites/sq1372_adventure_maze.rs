//! SQ-1372: Adventure's mazes, statically and live, on both engines.
//!
//! Graham Nelson's `Advent.inf` declares
//!
//! ```text
//! Class   MazeRoom
//!   with  short_name "Maze",
//!         description "You are in a maze of twisty little passages, all alike.";
//! MazeRoom Alike_Maze_1 with n_to Alike_Maze_1, e_to Alike_Maze_2, …;
//! ```
//!
//! — so the name the player reads is a PROPERTY, and the object header carries
//! only what Inform compiles for an object declared with no quoted name: the
//! identifier in parentheses, `(Alike_Maze_1)`. Reading the header alone named
//! every maze room after its source identifier, which cost two things at once:
//!
//!   * **Statically**, `lanthorn-mapgen` drew fifty-nine rooms on one layer
//!     called `(Alike_Maze_8)`, because `mapper::suggest::mentions_maze` reads
//!     `_` as a letter and so found no "maze" in `(Alike_Maze_8)` at all.
//!   * **Live**, the location ladder matches the status line's `Maze` against
//!     an object's name, and `(Alike_Maze_1)` matches nothing — so on
//!     `advent.z6` the map STOPPED at the room the maze was entered from and
//!     stayed there for every step inside it (measured: three moves into the
//!     alike maze all reported `At West End of Hall of Mists`, room 92).
//!
//! Both halves come from one fix, `zvm::objects::printed_name` /
//! `gvm::objects::ParseNames::printed_name`: the `short_name` property wins
//! when it holds a string, exactly as `parserm.h`'s `PrintShortName` does.
//!
//! Every case skips vacuously without `stories/` (gitignored, CI has none).

use std::path::{Path, PathBuf};

use app::engine::Engine;
use app::mapgen;
use app::session::{apply_turn, DeathWatch, GameSession};
use mapper::graph::RoomId;
use mapper::mapper::Mapper;

/// A story under the gitignored `stories/`, or `None` in a checkout without it
/// — the CI-safe vacuous-skip pattern (mirrors `sq1308_mapgen_layers::story`).
fn story(name: &str) -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    p.is_file().then_some(p)
}

/// The two Adventures on the shelf, one per engine: `advent.blb` is the Glulx
/// build (release 5 / serial 961209) and `advent.z6` the Z-machine one
/// (release 10 / serial 011123). Same source, same rooms, two readers.
const ADVENTURES: [&str; 2] = ["advent.blb", "advent.z6"];

/// What the story prints for a maze room is `Maze` — on both engines, and with
/// no compiled identifier anywhere on the map.
#[test]
fn every_adventure_maze_room_is_named_maze_and_no_room_keeps_an_identifier() {
    for name in ADVENTURES {
        let Some(path) = story(name) else { continue };
        let map = mapgen::generate(&path, true).expect("Adventure declares an I6-library map");

        let mazes = map.graph.rooms().filter(|r| r.label() == "Maze").count();
        assert_eq!(
            mazes, 25,
            "{name}: fourteen `Alike_Maze_*` rooms and eleven `Different_Maze_*`, all printed `Maze`"
        );
        for room in map.graph.rooms() {
            assert!(
                !room.label().contains('(') && !room.label().contains('_'),
                "{name}: {:?} is a compiled identifier, not a name the story prints",
                room.label()
            );
        }
        // The dead ends are named too — `DeadendRoom` is another `short_name`
        // class, and one of them overrides it with its own longer name.
        assert!(
            map.graph.rooms().filter(|r| r.label() == "Dead End").count() >= 10,
            "{name}: the `DeadendRoom` class prints `Dead End`"
        );
    }
}

/// The mazes peel onto maze-flagged layers of their own, each named after the
/// room it is entered from so that two mazes are two names.
///
/// **Three layers, not two.** The eleven-room "all different" maze is one
/// component; the fourteen-room "all alike" maze arrives in TWO, because
/// `At Brink of Pit` — a room with a name of its own, whose description says
/// "The maze continues at this level" — stands in the middle of it, and
/// `mapgen::maze_region` stops at a room name (the Cyclops Room rule its doc
/// comment sets out). Absorbed `Dead End` rooms (SQ-1311) make the layers
/// bigger than the maze rooms alone: 17 = 12 maze + 5 dead ends, 12 = 11 + 1,
/// 3 = 2 + 1.
#[test]
fn adventures_mazes_peel_onto_layers_named_after_their_entrances() {
    for name in ADVENTURES {
        let Some(path) = story(name) else { continue };
        let map = mapgen::generate(&path, true).expect("Adventure declares an I6-library map");

        let mut maze_layers: Vec<(String, usize)> = map
            .graph
            .layers()
            .iter()
            .filter(|(_, m)| m.maze)
            .map(|(&id, m)| (m.name.clone(), map.graph.rooms_in_layer(id).len()))
            .collect();
        maze_layers.sort();
        assert_eq!(
            maze_layers,
            vec![
                ("Maze (off At Brink of Pit)".to_string(), 3),
                ("Maze (off At West End of Hall of Mists)".to_string(), 17),
                ("Maze (off At West End of Long Hall)".to_string(), 12),
            ],
            "{name}: three maze layers, each named for the room it is entered from"
        );

        // Nothing named "Maze" is left on the main layer, and no maze room is
        // stranded on a layer that is not flagged as a maze.
        for room in map.graph.rooms().filter(|r| mapper::suggest::mentions_maze(r.label())) {
            assert!(
                map.graph.layer_is_maze(room.layer),
                "{name}: {:?} is on a layer that is not a maze",
                room.label()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Live: walking the alike maze on `advent.z6`
// ---------------------------------------------------------------------------

/// Boot `advent.z6` and drive real commands, the way `sq1264_forest_randomization`
/// does — `apply_turn` against a real `GameSession`, so the ladder in
/// `zvm::location` is the one under test.
struct Walk {
    mapper: Mapper,
    session: GameSession,
    death: DeathWatch,
}

impl Walk {
    fn advent() -> Option<Walk> {
        let bytes = std::fs::read(story("advent.z6")?).ok()?;
        let mut session = GameSession::new_with_trace(
            bytes,
            true,
            false,
            None,
            false,
            Vec::new(),
            None,
            None,
            Some((25, 80)),
        )
        .expect("advent.z6 boots without a ZError");
        session.set_strip_prompt(false);
        let _ = session.submit(""); // the v6 "[Press any key to start]" splash
        Some(Walk { mapper: Mapper::default(), session, death: DeathWatch::default() })
    }

    fn turn(&mut self, cmd: &str) -> Option<RoomId> {
        let result = self.session.submit(cmd);
        apply_turn(&mut self.mapper, cmd, &result, &mut self.death);
        self.mapper.graph.current()
    }

    fn label(&self, id: RoomId) -> String {
        self.mapper.graph.room(id).map(|r| r.label().to_string()).unwrap_or_default()
    }
}

/// The route from the start to the "all alike" maze, in the player's own
/// commands. The lamp and the keys are needed to get through the grate at all,
/// and the black rod (in the Debris Room) makes the crystal bridge that
/// reaches the west end of the Hall of Mists — the maze's own doorway.
const TO_THE_MAZE: [&str; 23] = [
    "look",
    "in",
    "take lamp",
    "take keys",
    "turn on lamp",
    "out",
    "south",
    "south",
    "south",
    "unlock grate with keys",
    "open grate",
    "down",
    "west",
    "west",
    "take rod",
    "west",
    "west",
    "west",
    "down",
    "west",
    "wave rod",
    "west",
    "west",
];

/// Three steps into the alike maze, the live map is in the maze — a different
/// room object each step, each one called `Maze`.
///
/// This is the case that fails with the fix reverted, and it fails in the
/// reported shape: the map never leaves `At West End of Hall of Mists` (room
/// 92), because the status line says `Maze` and the only name the ladder could
/// see was `(Alike_Maze_1)`.
#[test]
fn walking_advent_z6s_alike_maze_resolves_a_room_object_per_step() {
    let Some(mut w) = Walk::advent() else { return };
    for cmd in TO_THE_MAZE {
        w.turn(cmd);
    }
    let doorway = w.mapper.graph.current().expect("standing somewhere");
    assert_eq!(
        w.label(doorway),
        "At West End of Hall of Mists",
        "the walk must reach the maze's doorway — a non-vacuity guard on the route itself"
    );

    let mut seen: Vec<RoomId> = Vec::new();
    for (i, cmd) in ["south", "east", "south"].iter().enumerate() {
        let here = w
            .turn(cmd)
            .unwrap_or_else(|| panic!("step {i} ({cmd}) put the map nowhere"));
        assert_eq!(
            w.label(here),
            "Maze",
            "step {i} ({cmd}): the story prints `Maze`"
        );
        assert_ne!(
            here, doorway,
            "step {i} ({cmd}): the map is still outside the maze"
        );
        assert!(
            !seen.contains(&here),
            "step {i} ({cmd}): room {here} is a maze room seen already"
        );
        seen.push(here);
    }
    assert_eq!(seen.len(), 3, "three steps, three distinct maze rooms");
}
