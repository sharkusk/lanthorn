//! Group binary: The diagnostic dump commands — `cell dump`, `window dump` on every engine,
//! and the death/turn bookkeeping they read.
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 6. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

// Shared fixture-path resolution, declared ONCE per group binary: the suites
// below are modules of this one crate, so a `#[path]` module in each of them is
// the same file loaded several times over (clippy::duplicate_mod).
#[path = "suites/fixture_paths.rs"]
mod fixture_paths;

#[path = "suites/assist_voice.rs"]
mod assist_voice;

#[path = "suites/vocabulary_offer.rs"]
mod vocabulary_offer;
#[path = "suites/story_word_scrape.rs"]
mod story_word_scrape;
#[path = "suites/scope_completion.rs"]
mod scope_completion;
#[path = "suites/story_spellings.rs"]
mod story_spellings;

#[path = "suites/vocabulary_vetting.rs"]
mod vocabulary_vetting;
#[path = "suites/word_reveal.rs"]
mod word_reveal;
#[path = "suites/cell_dump_command.rs"]
mod cell_dump_command;
#[path = "suites/font_check.rs"]
mod font_check;
#[path = "suites/death_turn_tried.rs"]
mod death_turn_tried;
#[path = "suites/window_dump_bound_key.rs"]
mod window_dump_bound_key;
#[path = "suites/window_dump_engines.rs"]
mod window_dump_engines;
#[path = "suites/window_dump_last_game_frame.rs"]
mod window_dump_last_game_frame;
#[path = "suites/window_dump_non_v6.rs"]
mod window_dump_non_v6;
