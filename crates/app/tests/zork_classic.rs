//! Group binary: Zork I (v3) and Zork Zero driven as a plain Z-machine — status lines,
//! inventory, the here-column, window behaviour and persistence.
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 8. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

// Shared fixture-path resolution, declared ONCE per group binary: the suites
// below are modules of this one crate, so a `#[path]` module in each of them is
// the same file loaded several times over (clippy::duplicate_mod).
#[path = "suites/fixture_paths.rs"]
mod fixture_paths;

#[path = "suites/transcript_stream_files.rs"]
mod transcript_stream_files;
#[path = "suites/zork0_v6_gameplay.rs"]
mod zork0_v6_gameplay;
#[path = "suites/zork0_v6_persistence.rs"]
mod zork0_v6_persistence;
#[path = "suites/zork0_v6_windows.rs"]
mod zork0_v6_windows;
#[path = "suites/zork1_cellar_suggestion.rs"]
mod zork1_cellar_suggestion;
#[path = "suites/zork1_here_column.rs"]
mod zork1_here_column;
#[path = "suites/zork1_inventory.rs"]
mod zork1_inventory;
#[path = "suites/zork1_restore_status_width.rs"]
mod zork1_restore_status_width;
#[path = "suites/zork1_status_line.rs"]
mod zork1_status_line;
#[path = "suites/zork1_underground_render.rs"]
mod zork1_underground_render;
