// Test fixtures build structs by defaulting then setting a few fields, which is
// clearer than a full struct literal here. Silence the pedantic lint in tests only.
#![cfg_attr(test, allow(clippy::field_reassign_with_default))]

/// A scratch directory **this call alone owns**, created empty and named for `tag`.
///
/// The pid is not a discriminator between two tests. Under `cargo nextest run` it
/// looks like one, because nextest gives every test its own process; under
/// `cargo test`, which is what CI runs, one binary's tests share a process and run on
/// threads, so `format!("thing-{}", process::id())` inside a helper hands **every**
/// caller the same directory. `fs::write` truncates and `remove_dir_all` at the end of
/// a case deletes a neighbour's fixture mid-read, so what the race produces is not a
/// crash but a correct fixture failing its own assertion — and it is structurally
/// invisible to the local gate, which is how SQ-1131's exemplar
/// (`verb-synonyms-gen`'s `scratch()`, one directory shared by every caller of
/// `wordnet_fixture()`) turned CI red for eight consecutive commits while
/// `cargo nextest run --workspace` printed 0 every time.
///
/// A counter beside the pid is the whole fix, and having one place to get it is what
/// makes the wrong spelling unnecessary rather than merely discouraged. A `tag`
/// distinct per caller looks like the same thing and is not: it is a hand-maintained
/// invariant across call sites, which is exactly what the exemplar had.
///
/// ```ignore
/// let dir = app::scratch_dir("mac-profile");   // …-<pid>-<nth>, created, empty
/// ```
///
/// `scratch_path_discipline` fails a test file that builds one of these by hand from
/// a bare `process::id()`. Crates that cannot depend on `app` — `zvm`, `gvm`, `scott`
/// take zero external dependencies — spell the counter locally instead; the rule is
/// the counter, not this function.
///
/// # Panics
///
/// If the directory cannot be created, which for a fresh name under the system temp
/// directory means the harness has no scratch space and nothing after this would mean
/// anything.
pub fn scratch_dir(tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NTH: AtomicUsize = AtomicUsize::new(0);
    let nth = NTH.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("lanthorn-{tag}-{}-{nth}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("a scratch directory under the system temp directory");
    d
}

pub mod anim;
pub mod archive;
pub mod archive_worker;
pub mod assets;
pub mod assist;
pub mod cell_dump;
pub mod aux_store;
pub mod browser;
pub mod clipboard;
pub mod export;
pub mod history;
pub mod hints;
pub mod hint_download;
pub mod slash;
pub mod colors;
pub mod complete;
pub mod config;
pub mod config_template;
pub mod docs_reference;
pub mod corpus;
pub mod cover;
pub mod debug_panel;
/// Which files are volumes of one multi-disk release (SQ-0844).
///
/// **Re-exported, not declared** (SQ-0874): the rule moved to `cli-host` the day
/// `zvm-cli` needed it, because a CLI cannot depend on `app` and a second copy of
/// "which files form a release" is how two front-ends end up disagreeing about
/// what is on a shelf. One implementation, and every `app::disk_set::…` spelling
/// still reaches it.
pub use cli_host::disk_set;
pub mod engine;
pub mod native_font;
pub mod garglk_ini;
pub mod glk_backend;
pub mod glulx_debug;
pub mod glulx_roomlock;
pub mod glulx_session;
pub mod graphics;
pub mod inline_image;
pub mod inventory;
pub mod export_dot;
pub mod export_json;
pub mod export_svg;
pub mod fetch_worker;
pub mod metadata_import;
pub mod cover_gallery;
pub mod ifdb;
pub mod ifdb_search;
pub mod ifdb_search_modal;
pub mod ifiction;
pub mod ifid;
pub mod input;
pub mod interpreter;
pub mod keymap;
pub mod launch_options;
pub mod native_sound;
pub mod layout;
pub mod list_scroll;
pub mod map_dump;
pub mod mapgen;
pub mod menu;
pub mod notify;
pub mod pager;
pub mod pane_drag;
pub mod pcset_store;
pub mod period;
pub mod pixel_mouse;
pub mod persist_files;
pub mod picker;
pub mod probe;
pub mod query_sweep;
pub mod reload;
pub mod render;
pub mod random_exit_probe;
pub mod return_probe;
pub mod reveal;
pub mod roomid;
pub mod room_menu;
pub mod scott_debug;
pub mod scott_session;
pub mod session;
pub mod state;
pub mod stderr_redirect;
pub mod storage;
pub mod story_menu;
pub mod story_pick;
pub mod story_url;
pub mod style;
pub mod machine_boot;
pub mod story_info;
pub mod system_fonts;
pub mod styles;
pub mod symbols;
pub mod term_colors;
pub mod terminal_dump;
pub mod text_field;
pub mod textwidth;
pub mod theme;
pub mod tidy;
pub mod trace;
pub mod vfs_store;
pub mod vocab;
pub mod watch;
