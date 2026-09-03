// MAP#3 verification — v4+ location detection (commit 4adf7f9).
//
// Drives real story files (gitignored, kept locally under `stories/` at the
// repo root — see TODO.md) through boot and a few turns, then checks what
// `zvm::location::detect_location` reports. All tests skip gracefully when
// the `stories/` directory or an individual story file is absent (e.g. CI),
// so this is best-effort local verification, not a required CI gate.
//
// Findings (2026-07-02, see docs/TODO.md MAP#3):
//   - v3 games are unaffected: they still resolve via GlobalVar0 (unchanged
//     path). Regression check below.
//   - Commit 4adf7f9 fixed detection for v4+ games whose status line is
//     LEFT-JUSTIFIED (the common Infocom form: "Room Name    Score: n  Moves: n")
//     or uses a "Location:" label. Several such games now resolve.
//   - BeyondZork's status line is NOT left-justified: it CENTERS the room
//     name in row 1 and puts stats in row 2 ("EN:16  ST:08 ..."). A
//     centered-title fallback (accepted only when it validates against the
//     object tree) now resolves BeyondZork to "Hilltop" — closing the second
//     half of TODO MAP#3. See the dedicated test below.

use std::path::PathBuf;
use zvm::cpu::exec::{Machine, StepResult};
use zvm::location::{detect_location, find_player_object, LocationMethod};
use zvm::memory::Memory;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn load_story(name: &str) -> Option<Vec<u8>> {
    let path = stories_dir().join(name);
    if !path.exists() {
        return None;
    }
    std::fs::read(&path).ok()
}

/// Boot a story and run until the first line-read prompt (or a step cap),
/// answering any char-reads with '\n' and refusing save/restore along the
/// way. Returns the machine positioned at that first prompt.
fn boot_to_first_read(data: Vec<u8>) -> Option<Machine> {
    let mem = Memory::new(data).ok()?;
    let mut machine = Machine::new(mem);
    machine.init_caps();
    for _ in 0..2_000_000u64 {
        match machine.step() {
            StepResult::NeedLine { .. } => return Some(machine),
            StepResult::Quit | StepResult::Restart | StepResult::Fault => return Some(machine),
            StepResult::Continue => {}
            StepResult::NeedChar => machine.supply_char(b'\n'),
            StepResult::SaveRequest => machine.complete_save(false),
            StepResult::RestoreRequest => machine.complete_restore_failure(),
        }
    }
    None
}

/// Run `machine` for one more turn by supplying `input` as a line, stepping
/// until the next prompt (or a step cap). Char-reads get '\n'.
fn run_one_turn(machine: &mut Machine, input: &str) {
    machine.supply_line(input, 13);
    for _ in 0..2_000_000u64 {
        match machine.step() {
            StepResult::NeedLine { .. } | StepResult::Quit | StepResult::Restart | StepResult::Fault => return,
            StepResult::Continue => {}
            StepResult::NeedChar => machine.supply_char(b'\n'),
            StepResult::SaveRequest => machine.complete_save(false),
            StepResult::RestoreRequest => machine.complete_restore_failure(),
        }
    }
}

// ── v3 regression: unaffected by the v4+ fix ────────────────────────────────

#[test]
fn v3_games_still_resolve_via_global_var0() {
    if !stories_dir().exists() {
        return; // stories/ absent (fresh checkout / CI) — skip.
    }
    // A representative spread of v3 titles; each entry skips individually if
    // its file is absent so this test degrades gracefully on any machine.
    let v3_games = [
        ("zork1-r88-s840726.z3", "West of House"),
        ("deadline-r27-s831005.z3", "South Lawn"),
        ("enchanter-r29-s860820.z3", "Fork"),
        ("planetfall-r37-s851003.z3", "Deck Nine"),
        ("wishbringer-r69-s850920.z3", "Hilltop"),
    ];
    let mut checked = 0;
    for (file, expected_prefix) in v3_games {
        let Some(story) = load_story(file) else { continue };
        let Some(machine) = boot_to_first_read(story) else {
            panic!("{file}: never reached a line-read prompt");
        };
        let loc = detect_location(&machine)
            .unwrap_or_else(|| panic!("{file}: expected a v3 GlobalVar0 location, got None"));
        assert_eq!(loc.method(), LocationMethod::GlobalVar0, "{file}: v3 must use GlobalVar0, not the v4+ path");
        let name = loc.object().expect("GlobalVar0 always carries an object").name.clone();
        assert!(
            name.starts_with(expected_prefix),
            "{file}: expected room starting with {expected_prefix:?}, got {name:?}"
        );
        checked += 1;
    }
    if checked == 0 {
        eprintln!("SKIP: no v3 fixture files present under {:?}", stories_dir());
    }
}

// ── v4+ games the 4adf7f9 fix newly resolves (left-justified status lines) ──

#[test]
fn v4plus_left_justified_status_lines_now_resolve() {
    if !stories_dir().exists() {
        return;
    }
    let v4plus_games = [
        "zork1-invclues-r52-s871125.z5",
        "wishbringer-invclues-r23-s880706.z5",
        "sherlock-r26-s880127.z5",
        "LostPig.z8",
        "hitchhiker-invclues-r31-s871119.z5",
        "planetfall-invclues-r10-s880531.z5",
        "leathergoddesses-invclues-r4-s880405.z5",
        "amfv-r77-s850814.z4",
    ];
    let mut checked = 0;
    for file in v4plus_games {
        let Some(story) = load_story(file) else { continue };
        let Some(machine) = boot_to_first_read(story) else {
            panic!("{file}: never reached a line-read prompt");
        };
        let loc = detect_location(&machine);
        assert!(loc.is_some(), "{file}: expected a v4+ location (PlayerParent/StatusName), got None — regression?");
        checked += 1;
    }
    if checked == 0 {
        eprintln!("SKIP: no v4+ fixture files present under {:?}", stories_dir());
    }
}

// ── BeyondZork: TODO MAP#3 residual gap — now closed ────────────────────────

#[test]
fn beyondzork_centered_status_line_now_resolves() {
    // Closes the second half of TODO MAP#3. Drives past the VT220 prompt /
    // title crawl / character-creation menus (accepting the default character
    // via '\n' at each char-read) into the first real room, then asserts
    // `detect_location` now resolves the room. BeyondZork centers the room
    // name in row 1 (padded with leading spaces, stats on row 2); the
    // centered-title fallback in `status_line_room_name`/`detect_location`
    // now parses "Hilltop" and validates it against the object tree.
    let Some(story) = load_story("beyondzork-r57-s871221.z5") else {
        return; // fixture absent — skip.
    };
    let Some(mut machine) = boot_to_first_read(story) else {
        panic!("beyondzork: never reached a line-read prompt");
    };
    // "Is this a VT220?" -> no; "begin a new game..." -> begin; character
    // creation is an arrow-key menu, accept defaults via NeedChar '\n'.
    for input in ["no", "begin"] {
        run_one_turn(&mut machine, input);
    }
    // A few more '\n'-only turns to walk through the character-creation
    // menu screens (race/class/sex/name confirmation) and the "press any
    // key to begin the story" gate.
    for _ in 0..4 {
        run_one_turn(&mut machine, "");
    }

    let upper = &machine.screen.upper;
    let row1: String = (1..=upper.cols).map(|c| upper.cell(1, c).ch).collect();
    assert!(
        row1.contains("Hilltop"),
        "expected to have reached the Hilltop starting room by now, upper row1={row1:?}"
    );

    let loc = detect_location(&machine).unwrap_or_else(|| {
        panic!("BeyondZork centered status line should now resolve a location, got None")
    });
    let name = loc
        .object()
        .map(|o| o.name.clone())
        .unwrap_or_else(|| panic!("expected a validated room object, got {loc:?}"));
    assert!(
        name.starts_with("Hilltop"),
        "expected the resolved room to be Hilltop, got {name:?} (method {:?})",
        loc.method()
    );
}

#[test]
fn beyondzork_vt220_mode_bordered_title_resolves() {
    // VT220 mode (answer "yes" to "Is this a VT220?") frames the centered room
    // title with half-block bars: `▐  Hilltop  ▌`. The leading bar defeated
    // detection until `deframe` mapped box/block glyphs to spaces for parsing.
    // This locks in that fix at the story level.
    let Some(story) = load_story("beyondzork-r57-s871221.z5") else {
        return; // fixture absent — skip.
    };
    let Some(mut machine) = boot_to_first_read(story) else {
        panic!("beyondzork: never reached a line-read prompt");
    };
    for input in ["yes", "begin"] {
        run_one_turn(&mut machine, input);
    }
    for _ in 0..4 {
        run_one_turn(&mut machine, "");
    }

    let upper = &machine.screen.upper;
    let row1: String = (1..=upper.cols).map(|c| upper.cell(1, c).ch).collect();
    assert!(
        row1.contains('▐') && row1.contains("Hilltop"),
        "expected the bordered VT220 Hilltop title, upper row1={row1:?}"
    );

    let loc = detect_location(&machine)
        .unwrap_or_else(|| panic!("VT220 bordered status line should resolve a location, got None"));
    assert_eq!(loc.method(), LocationMethod::PlayerParent, "must validate via the avatar's parent chain");
    let name = loc.object().map(|o| o.name.clone()).unwrap_or_default();
    assert!(name.starts_with("Hilltop"), "expected Hilltop, got {name:?}");
}

// ── SQ-1259: room/player detection unchanged on titles the fix must not
// disturb — widening `player_candidates` to parse words, and tie-breaking
// `resolve_room_object` by the game's own `location` global / top-level
// parent, both apply only when they actually resolve an ambiguity. Falsified
// by reverting `crates/zvm/src/location.rs` alone (keeping this file) and
// confirming these still pass unchanged — they do, byte for byte, both
// before and after the fix. ─────────────────────────────────────────────────

/// Photopia opens on a title/credits screen with no room yet: `(self object)`
/// is genuinely this game's only avatar (it never renames or replaces it),
/// so widening candidate matching to parse words adds no new contender here.
#[test]
fn photopia_room_and_player_detection_unchanged() {
    let Some(story) = load_story("photopia.z5") else {
        return; // fixture absent — skip.
    };
    let Some(machine) = boot_to_first_read(story) else {
        panic!("photopia: never reached a line-read prompt");
    };
    assert_eq!(
        detect_location(&machine),
        None,
        "photopia's opening prompt is still a title/credits screen, not a room"
    );
    assert_eq!(
        find_player_object(&machine).map(|p| zvm::objects::short_name(&machine.mem, p)),
        Some("(self object)".to_string()),
        "photopia's avatar is genuinely the unrenamed Inform selfobj"
    );
}

/// Curses opens in the Attic with an avatar literally named "yourself" — the
/// Inform 6 idiom SQ-0701/SQ-1259's PLAYER_NAMES/PLAYER_WORDS both already
/// cover — so nothing about the SQ-1259 widening changes its resolution.
#[test]
fn curses_room_and_player_detection_unchanged() {
    let Some(story) = load_story("curses.z5") else {
        return; // fixture absent — skip.
    };
    let Some(machine) = boot_to_first_read(story) else {
        panic!("curses: never reached a line-read prompt");
    };
    let loc = detect_location(&machine).expect("curses opens in a detectable room");
    assert_eq!(loc.method(), LocationMethod::PlayerParent);
    let room = loc.object().expect("object-backed");
    assert_eq!(room.number, 35);
    assert_eq!(room.name, "Attic");
    let player = find_player_object(&machine).expect("curses has an identifiable player object");
    assert_eq!(player, 15);
    assert_eq!(zvm::objects::short_name(&machine.mem, player), "yourself");
}

// ── SQ-0358: a stale status line must not outrank the object tree ───────────

/// Restore `save_name` through the game's OWN restore path, so it redraws its status line exactly
/// as in a real session. Returns the machine at the next prompt, or None if the fixture is absent.
fn restore_fixture(story: &str, save_name: &str) -> Option<Machine> {
    let save = std::fs::read(stories_dir().join(save_name)).ok()?;
    let mut m = boot_to_first_read(load_story(story)?)?;
    let mut restored = false;
    m.supply_line("restore", 13);
    for _ in 0..2_000_000u64 {
        match m.step() {
            StepResult::RestoreRequest => {
                m.complete_restore_success(&save).ok()?;
                restored = true;
            }
            StepResult::NeedLine { .. } if restored => return Some(m),
            StepResult::NeedLine { .. } => m.supply_line("x", 13), // "Restore from file:" prompt
            StepResult::NeedChar => m.supply_char(b'\n'),
            StepResult::SaveRequest => m.complete_save(false),
            StepResult::Quit | StepResult::Restart | StepResult::Fault => return None,
            StepResult::Continue => {}
        }
    }
    None
}

/// Zork's Loud Room: the room that proves the status line is a rendering, not a fact.
///
/// Its echo routine intercepts input, so Zork never refreshes the status line while you stand
/// there — it keeps naming the room you came FROM. Detection used to take that text as ground
/// truth (via `StatusName`) and reported the previous room, so the mapper saw Round Room -E->
/// Damp Cave and the Loud Room never existed on the map at all. `cretin`'s parent pointed at the
/// Loud Room the whole time (SQ-0358).
///
/// Needs `stories/zork1-loud-room.qzl` (a save standing in the Round Room); skips without it.
#[test]
fn a_stale_status_line_does_not_hide_the_room_the_player_is_in() {
    let Some(mut m) = restore_fixture("zork1-invclues-r52-s871125.z5", "zork1-loud-room.qzl") else {
        return; // fixture absent — best-effort local verification, as with every test here.
    };
    let room = |m: &Machine| {
        detect_location(m).and_then(|l| l.object().map(|o| o.name.clone())).unwrap_or_default()
    };
    let shown = |m: &Machine| {
        zvm::location::status_line_room_name(&m.screen.upper, m.screen.upper_window_rows)
            .unwrap_or_default()
    };

    assert_eq!(room(&m), "Round Room", "the save stands in the Round Room");

    run_one_turn(&mut m, "east");
    assert_eq!(shown(&m), "Round Room", "Zork does not refresh its status line in the Loud Room");
    assert_eq!(room(&m), "Loud Room", "but the player IS there, and the object tree says so");

    run_one_turn(&mut m, "east");
    assert_eq!(room(&m), "Damp Cave", "a refreshed status line still resolves normally");

    // Westward too: the Loud Room is not skipped in either direction.
    run_one_turn(&mut m, "west");
    assert_eq!(shown(&m), "Damp Cave", "stale again, now naming the room we just left");
    assert_eq!(room(&m), "Loud Room");

    run_one_turn(&mut m, "west");
    assert_eq!(room(&m), "Round Room");
}
