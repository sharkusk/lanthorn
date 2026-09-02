// Test fixtures build structs by defaulting then setting a few fields, which is
// clearer than a full struct literal here. Silence the pedantic lint in tests only.
#![cfg_attr(test, allow(clippy::field_reassign_with_default))]

/// The ONE lock every integration suite holds while the process-global palette is set.
///
/// The palette is process-global. Every suite that boots a story on a machine profile
/// sets it, and every suite that asserts a resolved colour depends on it, so they must
/// exclude one another — and for a long time each suite declared its **own**
/// `static PALETTE`, twenty-three of them. Within a suite that reads as the rule its
/// doc claims ("no two cases here may boot at once"); across suites it excludes
/// nothing, because `crates/app/tests/suites/` are MODULES pulled into ~14 group
/// binaries and every suite in a group shares one process.
///
/// Invisible to the local gate and fatal on CI, which is the whole point of putting it
/// here: `cargo nextest run` gives every test its own PROCESS, so no suite can observe
/// another's palette and twenty-three locks are indistinguishable from one. `cargo
/// test`, which CI runs, gives a binary's tests one process and many threads. MEASURED
/// on main: `arthurs_notices_are_the_machines_white_on_the_machines_dark_grey` read a
/// page whose r channel was 90 — `#5A5A5A`, §8.3.1's standard grey — where the Amiga's
/// is 68, because another suite in `v6_render` held the standard palette at the moment
/// it looked (SQ-0904).
///
/// It lives in the library rather than in a test module because one static per
/// PROCESS is what correctness needs, and every group binary links this crate.
///
/// **Private since SQ-0987.** It was `pub`, and thirty suites took it directly and
/// then called `zvm::screen::set_palette` themselves — which meant the lock was held
/// but nothing was ever put back, so the first machine press a group binary booted
/// stayed installed for the rest of the process. Taking the raw lock is now
/// impossible from outside this crate: the only ways in are [`v6_palette`] and
/// [`v6_palette_at_boot`], both of which hand back a [`V6PaletteGuard`] that restores
/// `Palette::Standard` on drop. That is a compile error rather than a convention, and
/// it is half of what makes "a suite cannot set the palette without something that
/// restores it" true; [`v6_set_palette`] is the other half.
static V6_PALETTE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

thread_local! {
    /// Whether THIS thread currently holds a [`V6PaletteGuard`].
    ///
    /// The witness that lets [`v6_set_palette`] be safe without threading a guard
    /// through every harness's `boot()`. Set when a guard is handed out, cleared when
    /// it drops; a `std::sync::Mutex` is not reentrant, so at most one guard exists at
    /// a time and a plain flag is enough. Cleared on an unwinding panic too, because
    /// `Drop` runs.
    static PALETTE_GUARD_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Take [`V6_PALETTE_LOCK`] **and** state the palette, in one call that cannot do
/// one without the other.
///
/// The lock alone only ever protected the WRITERS, and a write is a call and a call is
/// visible in source — `palette_lock_discipline` fails a suite that reaches
/// `zvm::screen::set_palette` outside a guard. The other half is a suite that
/// merely READS — one that asserts a resolved colour without installing a palette of
/// its own, and so believes whatever the last suite in its group binary happened to
/// leave behind. That is an ABSENCE, invisible both to the source check and to the
/// gate: `cargo nextest run` gives every test its own process, where the inherited
/// palette is always the default and the suite is always right. MEASURED on main
/// (SQ-0958): `v6_shogun_gameplay` asserted §8.3.1 white while its sibling
/// `v6_shogun_title_header` booted THE SAME STORY under `InterpreterProfile::IbmPc`
/// and installed the IBM YZIP table, so two of its cases read `Rgb(173, 173, 173)`
/// under `cargo test` — which is what CI runs, so main was red and the local gate
/// green.
///
/// Hence the pairing here rather than two habits kept in step by hand: a suite that
/// wants the lock must name a palette to get it, and a suite that names a palette
/// gets the lock whether it thought about the race or not. The user's rule is
/// **"no test should be written that makes an assumption about a palette it did not
/// write"**, and `Palette::Standard` is as much an assumption as any other — a suite
/// that means the default still has to say so.
///
/// ```ignore
/// let _g = app::v6_palette(zvm::screen::Palette::Standard); // held for the case
/// ```
///
/// The guard must outlive every boot, render and colour assertion in the case: the
/// palette is process-global, so dropping it early lets a sibling install a machine's
/// table between the render and the assertion about it — and, since SQ-0959, dropping
/// it also puts `Palette::Standard` back, so an early drop moves the table under the
/// case that asked for it.
pub fn v6_palette(p: zvm::screen::Palette) -> V6PaletteGuard {
    let guard = take_lock();
    zvm::screen::set_palette(p);
    guard
}

/// A guard for a case whose palette is named by its own `boot()`, not here.
///
/// Identical to `v6_palette(Palette::Standard)` — it installs the default and holds
/// the lock — and it exists because thirty harnesses cannot use the other spelling.
/// Each of them resolves an `InterpreterProfile` from a medium DEEP INSIDE its `boot()`
/// and sets `profile.palette()` there, several rows below the point at which the case
/// took the lock. At the lock site nobody knows which palette is wanted yet, so the
/// only honest thing a case can state there is where it starts; it names the table it
/// actually measures through [`v6_set_palette`] a moment later.
///
/// It still installs `Standard` rather than leaving the table alone, and that is the
/// point rather than a detail. "Leave whatever was there" is how SQ-0958 happened: a
/// case that never writes a palette is asserting against whatever its group binary's
/// last writer left, which is `Standard` under `cargo nextest run` and a machine's
/// table under `cargo test`. A guard that starts from a known table has no such
/// question in it, whether or not its `boot()` goes on to name another.
///
/// ```ignore
/// let _g = app::v6_palette_at_boot();     // held for the whole case
/// let session = boot(story, profile)?;    // calls app::v6_set_palette inside
/// ```
pub fn v6_palette_at_boot() -> V6PaletteGuard {
    let guard = take_lock();
    zvm::screen::set_palette(zvm::screen::Palette::Standard);
    guard
}

/// Take [`V6_PALETTE_LOCK`] and post the thread's witness that it is held.
fn take_lock() -> V6PaletteGuard {
    let lock = V6_PALETTE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    PALETTE_GUARD_HELD.with(|held| held.set(true));
    V6PaletteGuard { _lock: lock }
}

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

/// Install a palette from inside a harness, having already taken a guard.
///
/// **The only route to `zvm::screen::set_palette` open to a test**, and the reason
/// SQ-0987 could migrate thirty suites without touching one line of any harness's boot
/// ORDER. Those harnesses resolve their profile below the lock site and set the table
/// from it; the two alternatives were to hoist the profile above the lock — restructuring
/// thirty `boot()`s, which is exactly how CLAUDE.md says one starts measuring a screen
/// the app never draws — or to thread a `&V6PaletteGuard` down through every boot
/// signature and its hundred-odd call sites. This does neither: the guard posts a
/// thread-local witness when it is handed out, and this asserts on it.
///
/// # Panics
///
/// If this thread holds no [`V6PaletteGuard`] — which is a caller that would have set
/// the process-global palette with nothing undertaking to put it back, and, worse,
/// without excluding the case running beside it in the same group binary. The panic is
/// the point: it names the defect in the writer rather than producing an inexplicable
/// colour in some other suite an hour later (SQ-0904).
pub fn v6_set_palette(p: zvm::screen::Palette) {
    assert!(
        PALETTE_GUARD_HELD.with(std::cell::Cell::get),
        "app::v6_set_palette was called on a thread holding no V6PaletteGuard.\n\
         The palette is process-global and every file under crates/app/tests/suites/ shares \
         a group binary — and therefore a process and a palette — with a dozen others, so a \
         write that takes no guard both races its neighbours and leaves the table dirty for \
         the rest of the run (SQ-0904/SQ-0959). Open the case with\n\
         \x20   let _g = app::v6_palette_at_boot();\n\
         held for as long as the boot and every assertion about its colours."
    );
    zvm::screen::set_palette(p);
}

/// What [`v6_palette`] hands back: the shared lock, plus the undertaking to put the
/// palette back when the case ends.
///
/// # Why the guard restores at all (SQ-0959)
///
/// Taking the lock says "no one else may write the palette while I hold this". It
/// never said anything about afterwards, so the first case in a group binary to boot
/// a machine press left that machine's table installed for the WHOLE PROCESS. Every
/// later case then ran on the last writer's palette rather than on the default —
/// under `cargo test`, which is what CI runs, and only there: `cargo nextest run`
/// gives every test its own process, so the table it inherits is always `Standard`
/// and the dirt is structurally invisible. That is the SQ-0958 shape exactly, one
/// level down: the reader rule tells a suite to state its palette, and this makes the
/// state a suite inherits when it does the same one nextest would have given it.
///
/// # Why `Standard` rather than whatever was there before
///
/// Restoring the PREVIOUS value is the tempting "leave no trace" reading, and it is
/// the right one for `zvm-cli`'s `swatch`, which borrows the palette per table row
/// inside a run whose machine is a real fact. Here there is no such fact: a test
/// process has no machine, and the value a guard would find on entry is only
/// meaningful if every writer restores. Restoring `Standard` needs no saved state, is
/// idempotent, and leaves each case starting from the same table nextest's fresh
/// process gives it, which is the property the reader rule assumes.
///
/// That reasoning was written (SQ-0959) while thirty suites still took the raw lock and
/// restored nothing, so restore-previous would have copied whichever machine's table
/// they left through every later guard for ever. SQ-0987 closed that: the lock is
/// private, [`v6_set_palette`] is the only route in, and there is no writer left that
/// does not restore. Restore-previous would now be merely pointless rather than
/// actively wrong — every value a guard could find on entry is `Standard` — which is
/// the invariant `the_only_route_to_the_palette_is_through_a_guard` asserts.
///
/// The palette goes back while the lock is still held: `Drop::drop` runs before the
/// struct's fields, so no other guard can be admitted into the window between.
#[must_use = "the palette is only guaranteed for as long as the guard is held, and is \
              put back the moment it is dropped"]
pub struct V6PaletteGuard {
    /// Released when this guard drops — after [`Drop::drop`] has put the palette back.
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Drop for V6PaletteGuard {
    fn drop(&mut self) {
        zvm::screen::set_palette(zvm::screen::Palette::Standard);
        PALETTE_GUARD_HELD.with(|held| held.set(false));
    }
}

/// The guard's own behaviour, asserted HERE rather than in an integration suite.
///
/// `tests/suites/` is the wrong place for it: every one of those files is compiled
/// into a group binary beside a dozen others, so reading the table back after a drop
/// is a race against whichever sibling happens to hold the lock next. This crate's own
/// test binary has no palette writer at all — `interpreter.rs`'s cases only ask a
/// *profile* which palette it names, which touches nothing global.
///
/// Both cases below could otherwise race EACH OTHER, and only one of them writes: the
/// `#[should_panic]` case asserts before it sets anything, so it leaves the table
/// exactly as it found it. Any third case that installs a palette has to be folded
/// into the first rather than added beside it.
#[cfg(test)]
mod v6_palette_guard {
    use zvm::screen::{palette, set_palette, Palette};

    /// The guard installs the palette it names, puts `Standard` back when it drops,
    /// puts back `Standard` rather than the value it displaced, and is the only thing
    /// that opens [`super::v6_set_palette`].
    ///
    /// Falsified by deleting the `Drop` impl above: the second assertion then reads
    /// `Amiga`, which is exactly the dirt SQ-0959 is about.
    #[test]
    fn the_guard_installs_a_palette_and_puts_the_default_back() {
        assert_eq!(palette(), Palette::Standard, "the process starts on the default");
        {
            let _g = super::v6_palette(Palette::Amiga);
            assert_eq!(palette(), Palette::Amiga, "the guard installs what it was named");
        }
        assert_eq!(palette(), Palette::Standard, "and hands the default back on drop");

        // Now the other half of the choice: leave the table dirty the way a raw-lock
        // suite left it before SQ-0987, and guard over the top. Restoring the DISPLACED
        // value would carry the IBM table forward for ever; the default ends it here.
        set_palette(Palette::IbmCga);
        {
            let _g = super::v6_palette(Palette::IbmYzip);
        }
        assert_eq!(palette(), Palette::Standard, "the guard cleans up after the writer before it");

        // The deferred spelling the thirty migrated harnesses use: the guard states
        // where the case starts, `boot()` names the table it measures further down,
        // and the drop still ends on the default.
        {
            let _g = super::v6_palette_at_boot();
            assert_eq!(palette(), Palette::Standard, "a deferred guard starts from the default");
            super::v6_set_palette(Palette::IbmXzip);
            assert_eq!(palette(), Palette::IbmXzip, "and the harness names its own table");
        }
        assert_eq!(palette(), Palette::Standard, "which the drop still puts back");
    }

    /// Setting the palette with no guard held panics rather than dirtying the process.
    ///
    /// The runtime half of SQ-0987's rule; `palette_lock_discipline`'s source scan is
    /// the other half, and catches the same defect one step earlier. Both are needed:
    /// the scan cannot see a call made through a re-export or an alias, and this cannot
    /// see a suite that calls `zvm::screen::set_palette` directly.
    #[test]
    #[should_panic(expected = "holding no V6PaletteGuard")]
    fn setting_the_palette_outside_a_guard_panics() {
        super::v6_set_palette(Palette::Amiga);
    }
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
pub mod return_probe;
pub mod reveal;
pub mod roomid;
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
