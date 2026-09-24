//! Group binary: The v6 render path itself, game-agnostic — frame borders at every medium,
//! the game-colour regression matrix, prose freezing and raster text loss — plus the
//! app-wide art-scaling policy every one of those paths now shares (SQ-0829).
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 4. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

// Shared fixture-path resolution, declared ONCE per group binary: the suites
// below are modules of this one crate, so a `#[path]` module in each of them is
// the same file loaded several times over (clippy::duplicate_mod).
#[path = "suites/fixture_paths.rs"]
mod fixture_paths;

#[path = "suites/art_resample_policy.rs"]
mod art_resample_policy;
#[path = "suites/v6_archive_border_sweep.rs"]
mod v6_archive_border_sweep;
#[path = "suites/v6_amiga_global_colour_pair.rs"]
mod v6_amiga_global_colour_pair;
#[path = "suites/v6_amiga_shipped_interpreter.rs"]
mod v6_amiga_shipped_interpreter;
#[path = "suites/v6_frame_border_medium.rs"]
mod v6_frame_border_medium;
#[path = "suites/v6_game_colour_regression.rs"]
mod v6_game_colour_regression;
#[path = "suites/v6_headless_compose.rs"]
mod v6_headless_compose;
#[path = "suites/v6_extended_frame.rs"]
mod v6_extended_frame;
#[path = "suites/v6_modal_dialog_bounds.rs"]
mod v6_modal_dialog_bounds;
#[path = "suites/v6_menu_plan_zero_slack.rs"]
mod v6_menu_plan_zero_slack;
#[path = "suites/v6_pixel_lock_backend.rs"]
mod v6_pixel_lock_backend;
#[path = "suites/v6_pixel_lock_centring.rs"]
mod v6_pixel_lock_centring;
#[path = "suites/v6_prose_freeze.rs"]
mod v6_prose_freeze;
#[path = "suites/v6_raster_reveal.rs"]
mod v6_raster_reveal;
#[path = "suites/v6_raster_text_loss.rs"]
mod v6_raster_text_loss;
#[path = "suites/v6_side_border_tiling.rs"]
mod v6_side_border_tiling;
#[path = "suites/v6_status_window_edges.rs"]
mod v6_status_window_edges;
