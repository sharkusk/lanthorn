//! Group binary: v6 palette and persistence — adaptive palettes, the display list, the
//! native archive, paced pictures, and everything that must survive a restore.
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 11. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

// Shared fixture-path resolution, declared ONCE per group binary: the suites
// below are modules of this one crate, so a `#[path]` module in each of them is
// the same file loaded several times over (clippy::duplicate_mod).
#[path = "suites/fixture_paths.rs"]
mod fixture_paths;

#[path = "suites/v6_adaptive_palette_arthur.rs"]
mod v6_adaptive_palette_arthur;
#[path = "suites/v6_adaptive_palette_zork0.rs"]
mod v6_adaptive_palette_zork0;
#[path = "suites/v6_bpal_oracle.rs"]
mod v6_bpal_oracle;
#[path = "suites/v6_display_list_capacity.rs"]
mod v6_display_list_capacity;
#[path = "suites/v6_display_list_restore.rs"]
mod v6_display_list_restore;
#[path = "suites/v6_hardware_palette.rs"]
mod v6_hardware_palette;
#[path = "suites/v6_native_adaptive_palette.rs"]
mod v6_native_adaptive_palette;
#[path = "suites/v6_native_archive_inline.rs"]
mod v6_native_archive_inline;
#[path = "suites/v6_paced_pictures.rs"]
mod v6_paced_pictures;
#[path = "suites/v6_restore_input_window_echo.rs"]
mod v6_restore_input_window_echo;
#[path = "suites/v6_restore_paint_ground.rs"]
mod v6_restore_paint_ground;
#[path = "suites/v6_restore_palette_replay.rs"]
mod v6_restore_palette_replay;
#[path = "suites/v6_restore_screen_layers.rs"]
mod v6_restore_screen_layers;
#[path = "suites/v6_restore_screen_pixels.rs"]
mod v6_restore_screen_pixels;
#[path = "suites/v6_restore_streamed_prose.rs"]
mod v6_restore_streamed_prose;
#[path = "suites/v6_resume_colours.rs"]
mod v6_resume_colours;
