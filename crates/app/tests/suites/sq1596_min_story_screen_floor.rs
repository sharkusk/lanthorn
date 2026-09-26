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

use std::path::{Path, PathBuf};

use app::config::Config;
use app::engine_helpers::zvm_session_opt;
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;

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
        data_base: home.join("saves"),
        flags: LaunchFlags::default(),
        terminal,
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
