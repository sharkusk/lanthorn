//! SQ-1596: a host willing to draw a story's grid windows at a SMALLER cell
//! size than its own terminal's — more, smaller cells in the same physical
//! screen space — can ask [`TerminalFacts::min_story_screen`] for an
//! effective pre-boot pane no smaller than a story's own minimum, even when
//! the host's REAL measured pane falls short of it, and have lanthorn seed
//! that larger cell count for the host to then render at whatever per-cell
//! pixel size actually fits.
//!
//! # The mechanism
//!
//! Some v4+ Z-machine stories read header bytes $20/$21 (screen rows/cols,
//! ZMSD §8.4) ONCE, at boot, and refuse to run below their own floor —
//! *Bureaucracy* wants 40 cols x 19 rows as the game itself sees them, and
//! prints its own ZIL string `[Screen too small.]` on the first turn
//! otherwise. A LATER resize to something bigger does not help, because the
//! story never looks at $20/$21 again (the same mechanism
//! `declared_story_screen_dims`'s post-boot floor already protects, from the
//! other side — SQ-0679/SQ-0680). This is pure game bytecode; no such check
//! exists anywhere in `crates/zvm` itself.
//!
//! `min_terminal_size_for_story_floor` (`crates/app/src/host/boot.rs`)
//! inverts the pre-boot pane arithmetic by SEARCH (the same shape
//! `layout::split_pct_for_story_width` already inverts the drag-to-resize
//! split by), widening the terminal size fed to `pre_boot_host_screen` in
//! whichever dimension(s) actually fall short, independently — a dimension
//! already at or above its floor is left exactly alone, and a pinned
//! `virtual_screen_cols`/`virtual_screen_rows` always wins over the floor,
//! exactly as it already wins over `story_screen_dims`'s own measurement.
//!
//! # Specimen
//!
//! | fixture | release / serial | floor |
//! |---|---|---|
//! | `stories/bureaucracy-r116-s870602.z4` | 116 / 870602 | 40 cols x 19 rows |
//!
//! `stories/` is gitignored (CLAUDE.md), so every case here skips vacuously
//! without it.
//!
//! # SQ-1602: the floor must survive `@restart` too
//!
//! `TerminalFacts::min_story_screen` is read once, at boot, into
//! [`app::state::AppState::min_story_screen`] — mirroring how SQ-1598 carries
//! `glk_cell_px` forward the same way — so `host::reset::reset_game` (the
//! engine behind `@restart`) can re-apply the SAME floor the launch used
//! rather than silently reverting to the real (narrow) terminal size, which
//! for a story booted under a floor would otherwise hit its own
//! `[Screen too small.]` refusal again on restart. The tests below drive
//! `reset_game` directly, the same call shape
//! `sq1598_glk_cell_px.rs`'s own restart test uses.

use std::path::{Path, PathBuf};

use app::config::Config;
use app::engine::Engine;
use app::engine_helpers::zvm_session_opt;
use app::host::{
    boot_story, story_screen_in, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts,
};
use app::launch_options::LaunchOverrides;

use crate::fixture_paths::fixture_path;

const STORY: &str = "bureaucracy-r116-s870602.z4";

/// Bureaucracy's own minimum — the exact pair its boot code compares $20/$21
/// against, per SQ-1596's own quest text.
const FLOOR: (u16, u16) = (40, 19);

/// A terminal narrow enough that, at the shipped default 50/50 map split
/// (`Config::split_ratio` = 50, map visible by default), the pre-boot story
/// pane misses `FLOOR` in BOTH dimensions — confirmed empirically against
/// `story_screen_in` while building this suite (a 30x10 terminal seeds a
/// ~14x9 story pane here).
const NARROW: (u16, u16) = (30, 10);

fn story_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(STORY)
}

/// A config rooted in a scratch home, so nothing here reads or writes the
/// real `~/.lanthorn` — same shape as `host_boot.rs`'s own `headless_config`.
fn headless_config(home: &Path) -> Config {
    Config {
        user_dir: home.to_path_buf(),
        config_file: home.join("config.toml"),
        random_seed: Some(1),
        ..Config::default()
    }
}

/// Boot Bureaucracy under `cfg`/`terminal` the way a headless host would, then
/// submit one empty command — the turn that answers `[Screen too small.]`
/// when the boot pane misses the floor, or proceeds into the licence form
/// when it doesn't.
fn boot_and_first_turn(cfg: Config, terminal: TerminalFacts, home: &Path) -> (BootedStory, String) {
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: story_path(),
        disk_entry: None,
        overrides: &overrides,
        cfg,
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal,
        fresh_start: false,
    };
    let mut b = boot_story(req, &mut QuietBoot).expect("Bureaucracy boots headlessly");
    let r = b.session.submit("");
    (b, r.transcript)
}

/// The story's own boot-time header bytes: $20 (rows) / $21 (cols), ZMSD
/// §8.4 — what `set_screen_dims` wrote before the game's first instruction
/// ran, read straight off the running Z-machine rather than inferred from the
/// render layer.
fn declared_screen(b: &BootedStory) -> (u16, u16) {
    let s = zvm_session_opt(&*b.session).expect("Bureaucracy is a Z-machine story");
    (s.machine.mem.read_byte(0x20) as u16, s.machine.mem.read_byte(0x21) as u16)
}

/// The real repro, both halves in one test: the SAME narrow pane refuses with
/// no floor set and boots normally with one.
///
/// Falsified by turning `min_terminal_size_for_story_floor`'s call at
/// `host/boot.rs`'s `host_screen` site into a no-op (`None => size` for both
/// arms) — the "with a floor" half then fails, still showing the refusal.
#[test]
fn a_narrow_pane_refuses_without_a_floor_and_boots_with_one() {
    if !story_path().is_file() {
        eprintln!("SKIP: {} absent", story_path().display());
        return;
    }

    let home = app::scratch_dir("sq1596-no-floor");
    let (_, turn1) = boot_and_first_turn(
        headless_config(&home),
        TerminalFacts { size: Some(NARROW), ..TerminalFacts::default() },
        &home,
    );
    assert!(
        turn1.contains("[Screen too small.]"),
        "premise: a narrow pane with no floor set must hit Bureaucracy's own refusal; turn was {turn1:?}"
    );
    let _ = std::fs::remove_dir_all(&home);

    let home = app::scratch_dir("sq1596-with-floor");
    let (_, turn1) = boot_and_first_turn(
        headless_config(&home),
        TerminalFacts { size: Some(NARROW), min_story_screen: Some(FLOOR), ..TerminalFacts::default() },
        &home,
    );
    assert!(
        !turn1.contains("[Screen too small.]"),
        "the SAME narrow pane, with a floor set, must clear Bureaucracy's refusal; turn was {turn1:?}"
    );
    assert!(
        turn1.contains("licence") || turn1.contains("Important"),
        "past the refusal, the same turn reaches Bureaucracy's licence-form intro; turn was {turn1:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// A user-pinned `virtual_screen_cols`/`virtual_screen_rows` — explicit
/// intent — outranks the floor exactly as it already outranks
/// `story_screen_dims`'s own measurement.
#[test]
fn a_pin_wins_over_the_floor() {
    if !story_path().is_file() {
        eprintln!("SKIP: {} absent", story_path().display());
        return;
    }
    let home = app::scratch_dir("sq1596-pin-wins");
    let mut cfg = headless_config(&home);
    // Pinned NARROWER than the floor in both dimensions.
    cfg.virtual_screen_cols = Some(12);
    cfg.virtual_screen_rows = Some(8);
    let terminal =
        TerminalFacts { size: Some(NARROW), min_story_screen: Some(FLOOR), ..TerminalFacts::default() };
    let (b, _) = boot_and_first_turn(cfg, terminal, &home);
    assert_eq!(
        declared_screen(&b),
        (8, 12),
        "the pin is what the story is actually told, not the floor's value"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// A real terminal pane already big enough in one dimension is left exactly
/// alone in that dimension — only the dimension actually falling short gets
/// bumped, so a host's already-adequate pane is never distorted.
#[test]
fn only_the_dimension_that_falls_short_gets_bumped() {
    if !story_path().is_file() {
        eprintln!("SKIP: {} absent", story_path().display());
        return;
    }
    const WIDE_ENOUGH_ROWS: (u16, u16) = (80, 24);

    let home = app::scratch_dir("sq1596-baseline");
    let (baseline, _) =
        boot_and_first_turn(headless_config(&home), TerminalFacts { size: Some(WIDE_ENOUGH_ROWS), ..TerminalFacts::default() }, &home);
    let (base_rows, base_cols) = declared_screen(&baseline);
    assert!(
        base_cols < FLOOR.0,
        "premise: 80 cols split 50/50 for the map already falls short of the {}-col floor; got {base_cols}",
        FLOOR.0
    );
    assert!(
        base_rows >= FLOOR.1,
        "premise: 24 rows already clears the {}-row floor with room to spare; got {base_rows}",
        FLOOR.1
    );
    let _ = std::fs::remove_dir_all(&home);

    let home = app::scratch_dir("sq1596-mixed-floor");
    let (floored, turn1) = boot_and_first_turn(
        headless_config(&home),
        TerminalFacts { size: Some(WIDE_ENOUGH_ROWS), min_story_screen: Some(FLOOR), ..TerminalFacts::default() },
        &home,
    );
    let (floored_rows, floored_cols) = declared_screen(&floored);
    assert_eq!(
        floored_rows, base_rows,
        "rows already cleared the floor, so the real terminal size is left exactly alone in that dimension"
    );
    assert!(
        floored_cols >= FLOOR.0,
        "cols were short, so the floor bumped the seeded terminal until the pane cleared {}; got {floored_cols}",
        FLOOR.0
    );
    assert!(!turn1.contains("[Screen too small.]"), "the bumped cols clear the refusal too; turn was {turn1:?}");
    let _ = std::fs::remove_dir_all(&home);
}

// ── SQ-1602: @restart parity — the floor must survive reset_game too ───────

/// The real repro: boot Bureaucracy narrow WITH a floor (as the first test
/// above already proves clears the boot-time refusal), then `@restart`
/// (`reset_game`) at the SAME narrow terminal size, and confirm the
/// restarted session also does not hit `[Screen too small.]` — i.e. the
/// floor carried from `TerminalFacts::min_story_screen` onto
/// `AppState::min_story_screen` (SQ-1602) survives the restart.
///
/// Falsified by dropping `reset.rs`'s new floor-check back to the old bare
/// `terminal_size.and_then(|size| super::story_screen_in(state, size))` —
/// this test then fails, again showing the refusal on restart.
#[test]
fn restart_keeps_the_hosts_floor_not_the_narrow_real_pane() {
    if !story_path().is_file() {
        eprintln!("SKIP: {} absent", story_path().display());
        return;
    }
    let home = app::scratch_dir("sq1602-restart-with-floor");
    let (mut b, turn1) = boot_and_first_turn(
        headless_config(&home),
        TerminalFacts { size: Some(NARROW), min_story_screen: Some(FLOOR), ..TerminalFacts::default() },
        &home,
    );
    assert!(
        !turn1.contains("[Screen too small.]"),
        "premise: the same narrow pane with a floor set must boot fine; turn was {turn1:?}"
    );
    assert_eq!(
        b.state.min_story_screen,
        Some(FLOOR),
        "premise: the boot's floor is carried onto AppState for reset.rs to re-read"
    );

    app::host::reset::reset_game(
        &mut *b.session,
        &mut b.mapper,
        &mut b.state,
        &b.story_bytes,
        &b.story_path,
        &b.game_dir,
        Some(NARROW),
        app::host::reset::ResetOptions::default(),
    );

    let restart_turn1 = b.session.submit("").transcript;
    assert!(
        !restart_turn1.contains("[Screen too small.]"),
        "an @restart at the SAME narrow terminal size must still clear Bureaucracy's refusal; turn was {restart_turn1:?}"
    );
    assert!(
        restart_turn1.contains("licence") || restart_turn1.contains("Important"),
        "past the refusal, the restarted turn reaches Bureaucracy's licence-form intro again; turn was {restart_turn1:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// Negative control: the SAME restart scenario, but with
/// `AppState::min_story_screen` cleared before the restart — proving the
/// floor is what does the work above, not some other boot-vs-restart
/// difference (e.g. the restarted session simply inheriting a wider screen
/// some other way).
#[test]
fn restart_without_the_floor_hits_the_refusal_again() {
    if !story_path().is_file() {
        eprintln!("SKIP: {} absent", story_path().display());
        return;
    }
    let home = app::scratch_dir("sq1602-restart-floor-cleared");
    let (mut b, turn1) = boot_and_first_turn(
        headless_config(&home),
        TerminalFacts { size: Some(NARROW), min_story_screen: Some(FLOOR), ..TerminalFacts::default() },
        &home,
    );
    assert!(!turn1.contains("[Screen too small.]"), "premise: boots fine with the floor; turn was {turn1:?}");

    // Clear the carried floor before the restart — simulating a host state
    // that never had one, without changing anything else about the scenario.
    b.state.min_story_screen = None;

    app::host::reset::reset_game(
        &mut *b.session,
        &mut b.mapper,
        &mut b.state,
        &b.story_bytes,
        &b.story_path,
        &b.game_dir,
        Some(NARROW),
        app::host::reset::ResetOptions::default(),
    );

    let restart_turn1 = b.session.submit("").transcript;
    assert!(
        restart_turn1.contains("[Screen too small.]"),
        "with no floor carried, an @restart at the same narrow terminal size must hit the refusal again; turn was {restart_turn1:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// Pin precedence on restart, mirroring `a_pin_wins_over_the_floor` above: a
/// user-pinned `virtual_screen_cols`/`virtual_screen_rows` must still outrank
/// the carried floor after `@restart`, not just at the original boot.
#[test]
fn a_pin_wins_over_the_floor_after_restart_too() {
    if !story_path().is_file() {
        eprintln!("SKIP: {} absent", story_path().display());
        return;
    }
    let home = app::scratch_dir("sq1602-restart-pin-wins");
    let mut cfg = headless_config(&home);
    // Pinned NARROWER than the floor in both dimensions.
    cfg.virtual_screen_cols = Some(12);
    cfg.virtual_screen_rows = Some(8);
    let terminal =
        TerminalFacts { size: Some(NARROW), min_story_screen: Some(FLOOR), ..TerminalFacts::default() };
    let (mut b, _) = boot_and_first_turn(cfg, terminal, &home);
    assert_eq!(
        declared_screen(&b),
        (8, 12),
        "premise: the pin wins over the floor at the original boot too"
    );

    app::host::reset::reset_game(
        &mut *b.session,
        &mut b.mapper,
        &mut b.state,
        &b.story_bytes,
        &b.story_path,
        &b.game_dir,
        Some(NARROW),
        app::host::reset::ResetOptions::default(),
    );

    assert_eq!(
        declared_screen(&b),
        (8, 12),
        "the pin is what the story is told after @restart too, not the carried floor's value"
    );
    let _ = std::fs::remove_dir_all(&home);
}

// ── SQ-1606: the search itself must be reachable by a host directly ────────

/// A host at a live resize wants the same search boot/`@restart` already use,
/// not only through `TerminalFacts::min_story_screen`. This calls
/// `app::host::boot::min_terminal_size_for_story_floor` directly, exactly as
/// an embedding host would: pulling `cfg`/`cs`/`garglk_overlay`/`layout` off
/// a booted session's own live `AppState`, at a real terminal size (`NARROW`)
/// below `FLOOR`, then feeds the returned terminal size back through the
/// same pane-derivation the boot path uses (`story_screen_in`) and confirms
/// it actually clears the floor.
///
/// Before SQ-1606 widened `min_terminal_size_for_story_floor` from
/// `pub(crate)` to `pub`, this integration test — compiled as a separate
/// crate that sees only `app`'s public API — could not even COMPILE. That
/// compile failure, confirmed before this fix and gone after it, is this
/// test's own falsification.
#[test]
fn a_host_can_call_the_search_directly_at_a_live_resize() {
    if !story_path().is_file() {
        eprintln!("SKIP: {} absent", story_path().display());
        return;
    }
    let home = app::scratch_dir("sq1606-host-direct-call");
    // Boot with no floor set: this test drives the search FUNCTION directly,
    // not the TerminalFacts::min_story_screen boot mechanism exercised above.
    let (b, _) = boot_and_first_turn(
        headless_config(&home),
        TerminalFacts { size: Some(NARROW), ..TerminalFacts::default() },
        &home,
    );

    let (cols, rows) = app::host::boot::min_terminal_size_for_story_floor(
        &b.state.config,
        &b.state.colors,
        &b.state.garglk_overlay,
        b.state.layout,
        NARROW,
        FLOOR,
    );
    assert!(
        cols > NARROW.0 || rows > NARROW.1,
        "NARROW misses FLOOR in at least one dimension (per this suite's own premise), \
         so the search called directly must bump something: got {:?}",
        (cols, rows)
    );

    let seeded = story_screen_in(&b.state, (cols, rows)).expect("a non-zero pane");
    assert!(
        seeded.0 >= FLOOR.1 && seeded.1 >= FLOOR.0,
        "the terminal size returned by a direct host call must clear the floor once fed back \
         through the same pane-derivation the boot path uses: seeded={seeded:?} floor={FLOOR:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

// ── SQ-1606 (reopened): the search reachable in PANE space, at a live resize ─
//
// A host at a live resize holds a PANE (the rect its own layout already carved
// past the help bar and any other chrome), not a terminal size — exactly what
// `host::screen::set_story_pane` takes. The case above still needs a second
// call (`story_screen_in`) to learn what pane its terminal-space answer
// yields; the cases below drive `set_story_pane` directly, in ONE call, the
// way a host's resize poll actually would.

/// The acceptance case: boot Bureaucracy with a floor in force, then simulate
/// a LIVE RESIZE — a `set_story_pane` call at a too-small PANE, with no
/// terminal size in sight — and confirm that ONE call lands the story's own
/// declared header ($20/$21) at or above the floor.
///
/// `LIVE_RESIZE_PANE` is `NARROW`'s own ~14x9 derived pane (this suite's own
/// module doc, confirmed empirically against `story_screen_in`), fed here
/// directly as a pane rather than rederived from a terminal size — the whole
/// point being that this test never calls `story_screen_in` or any
/// terminal-space function at all.
///
/// Falsified by short-circuiting `set_story_pane`'s new
/// `state.min_story_screen` floor-application step back to the pane
/// unmodified: the resize below then leaves the header's row count under
/// `FLOOR.1`, which this test's assertion catches.
#[test]
fn set_story_pane_at_a_live_resize_clears_the_floor_in_one_call() {
    if !story_path().is_file() {
        eprintln!("SKIP: {} absent", story_path().display());
        return;
    }
    let home = app::scratch_dir("sq1606-zmachine-live-resize");
    let (mut b, turn1) = boot_and_first_turn(
        headless_config(&home),
        TerminalFacts { size: Some((80, 24)), min_story_screen: Some(FLOOR), ..TerminalFacts::default() },
        &home,
    );
    assert!(
        !turn1.contains("[Screen too small.]"),
        "premise: boots fine at a comfortable size; turn was {turn1:?}"
    );

    const LIVE_RESIZE_PANE: (u16, u16) = (14, 9);
    let changed = app::host::screen::set_story_pane(&mut *b.session, &b.state, LIVE_RESIZE_PANE);
    assert!(changed, "the resize down to a too-small pane must change the Z-machine header");

    let (rows, cols) = declared_screen(&b);
    assert!(
        cols >= FLOOR.0 && rows >= FLOOR.1,
        "one call to set_story_pane at a too-small LIVE pane must still clear the floor: \
         got (rows={rows}, cols={cols}), floor={FLOOR:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// Negative control for the case above: with `state.min_story_screen` cleared,
/// the SAME too-small live pane is applied verbatim (today's unchanged
/// behaviour) — proving the floor above is what does the work, not some
/// unrelated floor already baked into the boot.
#[test]
fn set_story_pane_with_no_floor_applies_the_live_pane_verbatim() {
    if !story_path().is_file() {
        eprintln!("SKIP: {} absent", story_path().display());
        return;
    }
    let home = app::scratch_dir("sq1606-zmachine-live-resize-no-floor");
    let (mut b, _) = boot_and_first_turn(
        headless_config(&home),
        TerminalFacts { size: Some((80, 24)), ..TerminalFacts::default() },
        &home,
    );
    assert_eq!(b.state.min_story_screen, None, "premise: no floor carried");

    const LIVE_RESIZE_PANE: (u16, u16) = (14, 9);
    app::host::screen::set_story_pane(&mut *b.session, &b.state, LIVE_RESIZE_PANE);

    let (rows, _cols) = declared_screen(&b);
    assert!(
        rows < FLOOR.1,
        "with no floor set, the too-small live pane's row count must NOT be bumped: got rows={rows}, floor={FLOOR:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// v1-3/v6 exemption: `min_story_pane_for_floor` mirrors
/// `declared_story_screen_dims`'s own exemption — a v1-3 Z-machine story has
/// no $20/$21 fields at all, so no floor logic can apply to one. This drives
/// a minimal in-memory v3 story (the same shape `sq1598_glk_cell_px.rs`'s own
/// `set_glk_cell_px_is_a_no_op_for_a_non_glulx_engine` builds) directly through
/// `GameSession::new`, sets a floor on a throwaway `AppState`, and confirms
/// `set_story_pane` applies the live pane completely unchanged.
#[test]
fn v1_to_3_stories_are_exempt_from_the_pane_space_floor_too() {
    let mut buf = vec![0u8; 0x0100];
    buf[0x00] = 3;
    buf[0x04] = 0x00; // high memory
    buf[0x06] = 0x40; // initial PC (v3: PC itself, not PC-1)
    buf[0x08] = 0x40; // dictionary (empty-ish, unused)
    buf[0x0A] = 0x10; // objects
    buf[0x0C] = 0x20; // globals
    buf[0x0E] = 0x00; // static memory
    buf[0x40] = 0xBA; // QUIT
    let mut sess = app::session::GameSession::new(buf, true, false, None).expect("a minimal v3 story boots");
    let mut state = app::state::AppState::default();
    state.min_story_screen = Some((80, 40)); // an outlandish floor no real pane clears
    let tiny_pane = (10, 5);
    app::host::screen::set_story_pane(&mut sess, &state, tiny_pane);
    // v1-3 never writes $20/$21 in the first place — `sync_zvm_screen_dims`
    // itself is a no-op for this version — so the only observable proof here
    // is that the call does not panic and the header stays at its construction
    // default (zvm never wrote it).
    assert_eq!(sess.machine.mem.read_byte(0x20), 0, "v3: $20 is never written, floor or no floor");
    assert_eq!(sess.machine.mem.read_byte(0x21), 0, "v3: $21 is never written, floor or no floor");
}

// ── SQ-1606 (third follow-up): the search must probe the RAW pane ──────────
//
// `min_story_pane_for_floor`'s Z-machine probe used to call
// `declared_story_screen_dims`, which floors its own result at
// `boot_screen_cols` (SQ-0679/SQ-0680 — a running story's header column count
// must never shrink below what it booted with). That floor is correct for
// WRITING the header (`sync_zvm_screen_dims`), but wrong as this search's own
// yardstick: a host that also seeds `TerminalFacts::min_story_screen` at boot
// has already widened the pre-boot terminal until the RAW pre-boot pane
// clears the floor (`min_terminal_size_for_story_floor`), so
// `boot_screen_cols` itself ends up >= the floor from the moment the session
// boots. Probing with the header-floored function then makes EVERY
// candidate's returned cols >= boot_cols >= floor_cols trivially, so the very
// first probe "passes" regardless of the candidate's actual raw width, and
// the search returns `real` completely unchanged even though the pane's real
// rendered grid is still narrower than `floor`.
//
// Falsified by reverting `min_story_pane_for_floor`'s probe back to
// `declared_story_screen_dims(..., version, gs.boot_screen_cols)` — this test
// then fails, `got` reading back as the unchanged `REAL_PANE` (or its raw
// `story_screen_dims` reading short of `FLOOR.0`), exactly the originally
// reported symptom.
#[test]
fn min_story_pane_for_floor_bumps_past_a_trivial_boot_cols_floor() {
    if !story_path().is_file() {
        eprintln!("SKIP: {} absent", story_path().display());
        return;
    }
    let home = app::scratch_dir("sq1606-followup-raw-probe");
    // NARROW is narrow enough that, with FLOOR set, boot's own pre-boot search
    // (min_terminal_size_for_story_floor) widens the terminal until the RAW
    // pre-boot pane clears FLOOR — which is exactly what makes
    // `gs.boot_screen_cols` end up >= FLOOR.0 from the moment the session
    // boots, the precondition this bug needs.
    let (b, turn1) = boot_and_first_turn(
        headless_config(&home),
        TerminalFacts { size: Some(NARROW), min_story_screen: Some(FLOOR), ..TerminalFacts::default() },
        &home,
    );
    assert!(
        !turn1.contains("[Screen too small.]"),
        "premise: boots fine under the pre-boot floor search; turn was {turn1:?}"
    );

    // The exact repro from the quest note: a real pane narrower than FLOOR,
    // fed directly to the search — not through set_story_pane/story_screen_in,
    // and unrelated to the terminal size the session actually booted at.
    const REAL_PANE: (u16, u16) = (30, 19);
    let got = app::host::screen::min_story_pane_for_floor(&*b.session, &b.state, REAL_PANE, FLOOR);
    assert_ne!(
        got, REAL_PANE,
        "the search must bump the pane, not return the too-narrow real pane unchanged — \
         the bug this test catches is a trivially-satisfied boot_cols floor making the \
         very first probe pass regardless of the candidate's actual raw width"
    );

    let raw = app::render::screen::story_screen_dims(
        ratatui::layout::Rect::new(0, 0, got.0, got.1),
        &b.state,
    )
    .expect("a non-zero returned pane");
    assert!(
        raw.1 >= FLOOR.0,
        "the RETURNED pane's RAW story_screen_dims must itself clear the floor's column \
         count — not merely read as clearing it through the header-floored \
         declared_story_screen_dims, which is exactly what let the original bug through: \
         got pane={got:?} raw={raw:?} floor={FLOOR:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

// ── Glulx: the identity mapping (no chrome subtraction at all) ─────────────

/// Bureaucracy's own floor from above is meaningless to Glulx (a different
/// story, a different screen model); this is its own specimen and its own
/// floor, chosen only to be comfortably inside `chlorophyll.gblorb`'s usable
/// range at a plain headless boot.
const GLULX_FLOOR: (u16, u16) = (60, 30);

fn glulx_story_path() -> PathBuf {
    fixture_path("chlorophyll.gblorb")
}

fn boot_glulx(home: &Path, size: (u16, u16), min_story_screen: Option<(u16, u16)>) -> Option<BootedStory> {
    let story = glulx_story_path();
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return None;
    }
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: story,
        disk_entry: None,
        overrides: &overrides,
        cfg: headless_config(home),
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts { size: Some(size), min_story_screen, ..TerminalFacts::default() },
        fresh_start: false,
    };
    Some(boot_story(req, &mut QuietBoot).expect("chlorophyll boots headlessly"))
}

/// The actual laid-out screen size gvm's own window tree covers — since
/// SQ-1220 this is the whole pane for a plain single-window layout, so it is
/// exactly what a resize's `(cols, rows)` argument becomes once
/// `GlulxSession::resize` (no chrome subtraction, unlike the Z-machine side)
/// applies it.
fn glulx_content_size(session: &dyn Engine) -> (u16, u16) {
    session.screen().content_size
}

/// The Glulx pane-space case: a too-small live pane must be bumped up to the
/// floor with NO chrome subtraction at all — `GlulxSession::resize` applies
/// whatever `set_story_pane` hands it verbatim, so this is the "identity"
/// half of `min_story_pane_for_floor`'s contract.
///
/// Falsified the same way as the Z-machine case: short-circuit
/// `set_story_pane`'s floor step and this test's assertion catches the
/// pane landing at the too-small size instead of the floor.
#[test]
fn glulx_set_story_pane_bumps_a_too_small_pane_to_the_floor() {
    let home = app::scratch_dir("sq1606-glulx-too-small");
    let Some(mut b) = boot_glulx(&home, (80, 24), Some(GLULX_FLOOR)) else { return };

    let too_small = (20, 10);
    let changed = app::host::screen::set_story_pane(&mut *b.session, &b.state, too_small);
    assert!(changed, "the resize to a too-small pane must change the Glulx screen");

    let got = glulx_content_size(&*b.session);
    assert!(
        got.0 >= GLULX_FLOOR.0 && got.1 >= GLULX_FLOOR.1,
        "a too-small live pane must be bumped to at least the floor with no chrome subtraction: \
         got={got:?} floor={GLULX_FLOOR:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// A pane already at/above the floor is left completely untouched — the
/// identity mapping does not distort an already-adequate pane the way a
/// naive "always clamp to floor" might.
#[test]
fn glulx_set_story_pane_leaves_an_already_adequate_pane_untouched() {
    let home = app::scratch_dir("sq1606-glulx-already-adequate");
    let Some(mut b) = boot_glulx(&home, (80, 24), Some(GLULX_FLOOR)) else { return };

    let already_big = (90, 40);
    assert!(already_big.0 >= GLULX_FLOOR.0 && already_big.1 >= GLULX_FLOOR.1, "premise");
    app::host::screen::set_story_pane(&mut *b.session, &b.state, already_big);

    let got = glulx_content_size(&*b.session);
    assert_eq!(
        got, already_big,
        "a pane already clearing the floor must be applied EXACTLY as given, not distorted"
    );
    let _ = std::fs::remove_dir_all(&home);
}

// ── Default-unchanged: no floor set behaves byte-identical to before ───────

/// `set_story_pane` with `state.min_story_screen == None` must behave exactly
/// as it did before this floor mechanism existed: the pane it is handed is
/// exactly the pane both engine calls receive, nothing bumped.
#[test]
fn set_story_pane_with_no_floor_is_unchanged_for_glulx_too() {
    let home = app::scratch_dir("sq1606-glulx-no-floor");
    let Some(mut b) = boot_glulx(&home, (80, 24), None) else { return };
    assert_eq!(b.state.min_story_screen, None, "premise: no floor carried");

    let pane = (20, 10); // deliberately below GLULX_FLOOR — must NOT be bumped
    app::host::screen::set_story_pane(&mut *b.session, &b.state, pane);

    let got = glulx_content_size(&*b.session);
    assert_eq!(got, pane, "with no floor set, the pane is applied completely verbatim");
    let _ = std::fs::remove_dir_all(&home);
}
