//! Group binary: Shogun (v6) — gameplay walkthrough, the declared status columns, the title
//! header and the native archive round-trip.
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 5. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

// Shared fixture-path resolution, declared ONCE per group binary: the suites
// below are modules of this one crate, so a `#[path]` module in each of them is
// the same file loaded several times over (clippy::duplicate_mod).
#[path = "suites/fixture_paths.rs"]
mod fixture_paths;

#[path = "suites/v6_shogun_credit_replay.rs"]
mod v6_shogun_credit_replay;
#[path = "suites/v6_shogun_declared_columns.rs"]
mod v6_shogun_declared_columns;
#[path = "suites/v6_shogun_emphasis_rule.rs"]
mod v6_shogun_emphasis_rule;
#[path = "suites/v6_shogun_gameplay.rs"]
mod v6_shogun_gameplay;
#[path = "suites/v6_shogun_room_description.rs"]
mod v6_shogun_room_description;
#[path = "suites/v6_hint_clue_wrap.rs"]
mod v6_hint_clue_wrap;
#[path = "suites/v6_shogun_menu_ground.rs"]
mod v6_shogun_menu_ground;
#[path = "suites/v6_shogun_native_archive.rs"]
mod v6_shogun_native_archive;
#[path = "suites/v6_shogun_prompt_style.rs"]
mod v6_shogun_prompt_style;
#[path = "suites/v6_shogun_room_art.rs"]
mod v6_shogun_room_art;
#[path = "suites/v6_shogun_status_alignment.rs"]
mod v6_shogun_status_alignment;
#[path = "suites/v6_shogun_title_header.rs"]
mod v6_shogun_title_header;

#[path = "suites/v6_screen_palette.rs"]
mod v6_screen_palette;
