//! Group binary: Journey (v6) — the Amiga floppy's frame, the menu strip, boot text and
//! prose containment. Release 30 (`.adf`) and release 83 (`journey.z6`) are
//! different builds; the suites say which they drive.
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 4. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

#[path = "suites/v6_art_resample.rs"]
mod v6_art_resample;
#[path = "suites/v6_journey_amiga_frame.rs"]
mod v6_journey_amiga_frame;
#[path = "suites/v6_journey_boot_text.rs"]
mod v6_journey_boot_text;
#[path = "suites/v6_journey_menu.rs"]
mod v6_journey_menu;
#[path = "suites/v6_journey_menu_band.rs"]
mod v6_journey_menu_band;
#[path = "suites/v6_journey_prose_containment.rs"]
mod v6_journey_prose_containment;
#[path = "suites/sq1579_journey_no_map.rs"]
mod sq1579_journey_no_map;
#[path = "suites/sq1589_journey_combat_party.rs"]
mod sq1589_journey_combat_party;
