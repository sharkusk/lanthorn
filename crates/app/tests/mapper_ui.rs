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
#[path = "suites/sq1287_advent_map_layout.rs"]
mod sq1287_advent_map_layout;
#[path = "suites/sq1289_random_room_placement.rs"]
mod sq1289_random_room_placement;
#[path = "suites/sq1291_zork_chasm_layout.rs"]
mod sq1291_zork_chasm_layout;
#[path = "suites/sq1292_probed_return_arrow.rs"]
mod sq1292_probed_return_arrow;
#[path = "suites/sq1266_v6_shadow_restore.rs"]
mod sq1266_v6_shadow_restore;
#[path = "suites/sq1267_shadow_room_identity.rs"]
mod sq1267_shadow_room_identity;
#[path = "suites/sq1260_zil_carousel_randomization.rs"]
mod sq1260_zil_carousel_randomization;
#[path = "suites/sq1268_zil_v4plus_exits.rs"]
mod sq1268_zil_v4plus_exits;
#[path = "suites/sq1284_glulx_restore_room_cache.rs"]
mod sq1284_glulx_restore_room_cache;
#[path = "suites/sq1286_glulx_room_lock.rs"]
mod sq1286_glulx_room_lock;
#[path = "suites/sq1283_shogun_room_identity.rs"]
mod sq1283_shogun_room_identity;
#[path = "suites/sq1283b_shogun_below_decks_fan.rs"]
mod sq1283b_shogun_below_decks_fan;
#[path = "suites/sq1285_bolded_object_name_room.rs"]
mod sq1285_bolded_object_name_room;
#[path = "suites/sq1293_glulx_opening_room.rs"]
mod sq1293_glulx_opening_room;
#[path = "suites/sq1294_glulx_silent_vehicle_move.rs"]
mod sq1294_glulx_silent_vehicle_move;
#[path = "suites/sq1295_glulx_bold_name_below_heading.rs"]
mod sq1295_glulx_bold_name_below_heading;
#[path = "suites/sq1294b_glulx_flashback_heading.rs"]
mod sq1294b_glulx_flashback_heading;
#[path = "suites/sq1301_spider_and_web_twin_rooms.rs"]
mod sq1301_spider_and_web_twin_rooms;
#[path = "suites/sq1302_wizard_sniffer_rooms.rs"]
mod sq1302_wizard_sniffer_rooms;
#[path = "suites/sq1304_anchorhead_twisting_lane.rs"]
mod sq1304_anchorhead_twisting_lane;
