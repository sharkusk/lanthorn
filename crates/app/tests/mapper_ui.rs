//! Group binary: The automap and its UI — matrix view and path highlighting, maze layers,
//! the room dock, the command band, and room detection/corroboration.
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 12. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

// Shared fixture-path resolution, declared ONCE per group binary: the suites
// below are modules of this one crate, so a `#[path]` module in each of them is
// the same file loaded several times over (clippy::duplicate_mod).
#[path = "suites/fixture_paths.rs"]
mod fixture_paths;

#[path = "suites/adult_words.rs"]
mod adult_words;
#[path = "suites/advent_toolbar.rs"]
mod advent_toolbar;
#[path = "suites/anchor_box_quote.rs"]
mod anchor_box_quote;
#[path = "suites/anchor_room_detection.rs"]
mod anchor_room_detection;
#[path = "suites/command_band.rs"]
mod command_band;
#[path = "suites/border_controls.rs"]
mod border_controls;
#[path = "suites/return_probe.rs"]
mod return_probe;
#[path = "suites/declared_exit.rs"]
mod declared_exit;
#[path = "suites/matrix_path_highlight.rs"]
mod matrix_path_highlight;
#[path = "suites/matrix_view.rs"]
mod matrix_view;
#[path = "suites/maze_layer_commands.rs"]
mod maze_layer_commands;
#[path = "suites/maze_layer_frozen.rs"]
mod maze_layer_frozen;
#[path = "suites/lostpig_room_and_inventory.rs"]
mod lostpig_room_and_inventory;
#[path = "suites/mysterious_room_detection.rs"]
mod mysterious_room_detection;
#[path = "suites/nameonly_room_corroboration.rs"]
mod nameonly_room_corroboration;
#[path = "suites/retired_exit_surfaces.rs"]
mod retired_exit_surfaces;
#[path = "suites/room_dock_render.rs"]
mod room_dock_render;
#[path = "suites/sq1264_forest_randomization.rs"]
mod sq1264_forest_randomization;
