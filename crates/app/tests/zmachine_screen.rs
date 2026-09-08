//! Group binary: The non-v6 Z-machine screen model — ZMSD compliance, the upper window,
//! pane titles, the MORE pager and the input band's chrome.
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 10. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

// Shared fixture-path resolution, declared ONCE per group binary: the suites
// below are modules of this one crate, so a `#[path]` module in each of them is
// the same file loaded several times over (clippy::duplicate_mod).
#[path = "suites/fixture_paths.rs"]
mod fixture_paths;

#[path = "suites/beyondzork_title_repaint.rs"]
mod beyondzork_title_repaint;
#[path = "suites/boot_screen_clear.rs"]
mod boot_screen_clear;
#[path = "suites/drawn_edge_honesty.rs"]
mod drawn_edge_honesty;
#[path = "suites/input_suggestion_border_style.rs"]
mod input_suggestion_border_style;
#[path = "suites/more_pager_arming.rs"]
mod more_pager_arming;
#[path = "suites/more_pager_first_new_row.rs"]
mod more_pager_first_new_row;
#[path = "suites/pane_title_sources.rs"]
mod pane_title_sources;
#[path = "suites/print_then_erase_boundary.rs"]
mod print_then_erase_boundary;
#[path = "suites/sq1354b_bureaucracy_bp_line.rs"]
mod sq1354b_bureaucracy_bp_line;
#[path = "suites/sq1355_bureaucracy_form_exit.rs"]
mod sq1355_bureaucracy_form_exit;
#[path = "suites/sq1411_splash_resume_pager.rs"]
mod sq1411_splash_resume_pager;
#[path = "suites/transparent_backdrop_audit.rs"]
mod transparent_backdrop_audit;
#[path = "suites/upper_grid_resize.rs"]
mod upper_grid_resize;
#[path = "suites/upper_window_border_style.rs"]
mod upper_window_border_style;
#[path = "suites/upper_window_terminal_bg.rs"]
mod upper_window_terminal_bg;
#[path = "suites/zmsd_screen_compliance.rs"]
mod zmsd_screen_compliance;
