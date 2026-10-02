//! Group binary: Arthur and graphical Adventure (v6) — status bars, intro plates, help bar,
//! Inform-authored v6 titles and the title smoke.
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 7. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

// Shared fixture-path resolution, declared ONCE per group binary: the suites
// below are modules of this one crate, so a `#[path]` module in each of them is
// the same file loaded several times over (clippy::duplicate_mod).
#[path = "suites/fixture_paths.rs"]
mod fixture_paths;

#[path = "suites/v6_advent_help_bar.rs"]
mod v6_advent_help_bar;
#[path = "suites/v6_advent_status.rs"]
mod v6_advent_status;
#[path = "suites/v6_arthur_amiga_proportional.rs"]
mod v6_arthur_amiga_proportional;
#[path = "suites/v6_arthur_hint_box.rs"]
mod v6_arthur_hint_box;
#[path = "suites/v6_arthur_hint_page.rs"]
mod v6_arthur_hint_page;
#[path = "suites/v6_arthur_intro_plates.rs"]
mod v6_arthur_intro_plates;
#[path = "suites/v6_arthur_status.rs"]
mod v6_arthur_status;
#[path = "suites/v6_arthur_room_description.rs"]
mod v6_arthur_room_description;
#[path = "suites/v6_inform_titles.rs"]
mod v6_inform_titles;
#[path = "suites/v6_location_mapper.rs"]
mod v6_location_mapper;
#[path = "suites/v6_titles_smoke.rs"]
mod v6_titles_smoke;
