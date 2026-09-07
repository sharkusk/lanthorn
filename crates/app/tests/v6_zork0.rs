//! Group binary: Zork Zero (v6) — the hybrid frame, the icon backdrop, the splash, mouse
//! input and the hint menus.
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 5. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

#[path = "suites/honor_colours_artwork_pin.rs"]
mod honor_colours_artwork_pin;
#[path = "suites/v6_band_tiling.rs"]
mod v6_band_tiling;
#[path = "suites/v6_cga_stencil_page.rs"]
mod v6_cga_stencil_page;
#[path = "suites/v6_ega_dither_blend.rs"]
mod v6_ega_dither_blend;
#[path = "suites/v6_float_machine_page.rs"]
mod v6_float_machine_page;

#[path = "suites/v6_meta_line_ground.rs"]
mod v6_meta_line_ground;
#[path = "suites/v6_float_margin_ground.rs"]
mod v6_float_margin_ground;
#[path = "suites/v6_glyphs_over_art.rs"]
mod v6_glyphs_over_art;
#[path = "suites/v6_hybrid_frame_gate.rs"]
mod v6_hybrid_frame_gate;
#[path = "suites/v6_hybrid_zork0.rs"]
mod v6_hybrid_zork0;
#[path = "suites/v6_mac_input_echo.rs"]
mod v6_mac_input_echo;
#[path = "suites/v6_mac_pillar_feet.rs"]
mod v6_mac_pillar_feet;
#[path = "suites/v6_macintosh_profile.rs"]
mod v6_macintosh_profile;
#[path = "suites/v6_hint_menu_mouse.rs"]
mod v6_hint_menu_mouse;
#[path = "suites/v6_mouse_zork0.rs"]
mod v6_mouse_zork0;
#[path = "suites/v6_click_vs_selection.rs"]
mod v6_click_vs_selection;
#[path = "suites/v6_zork0_color_command.rs"]
mod v6_zork0_color_command;
#[path = "suites/v6_zork0_hints.rs"]
mod v6_zork0_hints;
#[path = "suites/v6_zork0_icon_backdrop.rs"]
mod v6_zork0_icon_backdrop;
#[path = "suites/v6_zork0_splash.rs"]
mod v6_zork0_splash;
