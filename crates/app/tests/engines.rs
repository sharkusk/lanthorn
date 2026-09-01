//! Group binary: Engine and medium coverage — Glulx, Scott Adams, the headless driver,
//! interpreter profiles, disk images and the release pins each medium carries.
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 16. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

// Shared fixture-path resolution, declared ONCE per group binary: the suites
// below are modules of this one crate, so a `#[path]` module in each of them is
// the same file loaded several times over (clippy::duplicate_mod).
#[path = "suites/fixture_paths.rs"]
mod fixture_paths;

#[path = "suites/adf_disk_image.rs"]
mod adf_disk_image;
#[path = "suites/apple_disk_set_release.rs"]
mod apple_disk_set_release;
#[path = "suites/art_build_pairing.rs"]
mod art_build_pairing;
#[path = "suites/colour_regime_media.rs"]
mod colour_regime_media;
#[path = "suites/cover_frontispiece.rs"]
mod cover_frontispiece;
#[path = "suites/disk_set_rows.rs"]
mod disk_set_rows;
#[path = "suites/disk_story_rows.rs"]
mod disk_story_rows;
#[path = "suites/glulx_banner_rooms.rs"]
mod glulx_banner_rooms;
#[path = "suites/hfs_disk_image.rs"]
mod hfs_disk_image;
#[path = "suites/masterpieces_sides.rs"]
mod masterpieces_sides;
#[path = "suites/glulx_boot_room_id.rs"]
mod glulx_boot_room_id;
#[path = "suites/glulx_game_colours.rs"]
mod glulx_game_colours;
#[path = "suites/glulx_garglk_style_sentinel.rs"]
mod glulx_garglk_style_sentinel;
#[path = "suites/glulx_ingame_save_host_restore.rs"]
mod glulx_ingame_save_host_restore;
#[path = "suites/atari_st_profile.rs"]
mod atari_st_profile;
#[path = "suites/apple_iigs_profile.rs"]
mod apple_iigs_profile;
#[path = "suites/apple_release_artwork.rs"]
mod apple_release_artwork;
#[path = "suites/glulx_maze_identity.rs"]
mod glulx_maze_identity;
#[path = "suites/glulx_mouse_hyperlink_drawn_rect.rs"]
mod glulx_mouse_hyperlink_drawn_rect;
#[path = "suites/glk_grid_ground_reversed.rs"]
mod glk_grid_ground_reversed;
#[path = "suites/glulx_pending_io_host_restore.rs"]
mod glulx_pending_io_host_restore;
#[path = "suites/glulx_resume_location.rs"]
mod glulx_resume_location;
#[path = "suites/glulx_room_detection.rs"]
mod glulx_room_detection;
#[path = "suites/headless.rs"]
mod headless;
#[path = "suites/interpreter_profile.rs"]
mod interpreter_profile;
#[path = "suites/launch_options.rs"]
mod launch_options;
#[path = "suites/picture_override.rs"]
mod picture_override;
#[path = "suites/period_look_media.rs"]
mod period_look_media;
#[path = "suites/real_media_releases.rs"]
mod real_media_releases;
#[path = "suites/story_identity_sweep.rs"]
mod story_identity_sweep;
#[path = "suites/story_url_fetch.rs"]
mod story_url_fetch;
#[path = "suites/release_asset_span.rs"]
mod release_asset_span;
#[path = "suites/restart_reboots_in_place.rs"]
mod restart_reboots_in_place;
#[path = "suites/save_key_media.rs"]
mod save_key_media;
#[path = "suites/scott_mapper.rs"]
mod scott_mapper;
#[path = "suites/shogun_dict_words.rs"]
mod shogun_dict_words;
#[path = "suites/wizard_sniffer.rs"]
mod wizard_sniffer;

#[path = "suites/palette_lock_discipline.rs"]
mod palette_lock_discipline;
#[path = "suites/scratch_path_discipline.rs"]
mod scratch_path_discipline;
#[path = "suites/terminal_restore_order.rs"]
mod terminal_restore_order;
#[path = "suites/release_enumeration.rs"]
mod release_enumeration;
#[path = "suites/volume_chooser.rs"]
mod volume_chooser;
#[path = "suites/font3_shipped_font.rs"]
mod font3_shipped_font;
#[path = "suites/amiga_rom_face.rs"]
mod amiga_rom_face;
#[path = "suites/native_disk_font.rs"]
mod native_disk_font;
#[path = "suites/story_pick_media.rs"]
mod story_pick_media;
#[path = "suites/zip_story_entries.rs"]
mod zip_story_entries;
#[path = "suites/v6_dump_windows_face.rs"]
mod v6_dump_windows_face;
#[path = "suites/system_face_cascade.rs"]
mod system_face_cascade;
#[path = "suites/native_disk_sound.rs"]
mod native_disk_sound;
#[path = "suites/dialog_story_degradation.rs"]
mod dialog_story_degradation;

