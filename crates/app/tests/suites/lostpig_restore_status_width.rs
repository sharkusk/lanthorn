//! SQ-1604: the SQ-0681 restore-width floor (`GameSession::boot_screen_cols`)
//! is a permanent pin — it holds a restored save's screen width for the rest
//! of the session even for a story that recomputes its status-line layout
//! from header byte `$21` (screen width, ZMSD §11.1) EVERY turn, for which
//! the pin protects nothing once the story has repainted once more.
//!
//! Zork 1 r52 (`zork1_restore_status_width.rs`) is the specimen that pin must
//! never let go of: it computes its field columns ONCE at boot and never
//! reads `$21` again, so nothing ever tells the host it is safe to release
//! the floor, and it must stay raised for the rest of the session.
//!
//! Lost Pig (`LostPig.z8`, Inform 7) is the other specimen: it lays its
//! status line out fresh from `$21` every turn, so the floor's protection is
//! needed only until the very next turn — and this suite is where that
//! release is pinned. `zvm::cpu::exec::Machine::take_header_width_read`
//! (SQ-1604) is the new instrumentation that tells the two apart: it fires
//! only from the game's own `loadb`/`loadw` of header byte `$21`, never from
//! the interpreter's internal reads.
//!
//! **Falsified**: this suite's `a_restore_floor_releases_once_the_story_reads_21_itself`
//! reproduces the permanently-clipped-width symptom against the pre-fix code
//! (reverting `note_restored_screen_cols`/`drain_turn`'s SQ-1604 addition makes
//! it fail with `boot_screen_cols` still pinned at the wide value after the
//! healing turn) and passes with the fix.
//!
//! Uses the same lightweight save/restore shape as `zork1_restore_status_width.rs`
//! (`Engine::save_state`/`restore_state` + `app::session::restore_screen`)
//! rather than the full `host::persist` archive path, since only the VM
//! snapshot and the screen it carries matter here.
//!
//! Gitignored fixture: skips vacuously when absent.

use app::engine::Engine;
use app::session::{GameSession, InputKind};

use crate::fixture_paths::fixture_path;

/// Boot `LostPig.z8` at `cols` columns and tap through to the first line
/// prompt, mirroring `lostpig_room_and_inventory.rs`'s `boot_lostpig`.
fn boot_lostpig(cols: u16) -> Option<GameSession> {
    let path = fixture_path("LostPig.z8");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut session =
        GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, Some((25, cols)))
            .expect("LostPig.z8 should load and boot without a ZError");
    let mut n = 0;
    while session.pending_input() == InputKind::Char && n < 10 {
        let _ = session.submit_char(13);
        n += 1;
    }
    assert_eq!(session.pending_input(), InputKind::Line, "boot should reach a line prompt");
    Some(session)
}

#[test]
fn a_restore_floor_releases_once_the_story_reads_21_itself() {
    const WIDE: u16 = 120;
    const NARROW: u16 = 60;

    // Capture a Save State from a session booted (and played one turn) wide.
    let Some(mut wide) = boot_lostpig(WIDE) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    assert_eq!(wide.boot_screen_cols, WIDE, "the capture session booted at its own pane");
    let r = wide.submit("look");
    assert!(r.fault.is_none() && !r.quit, "\"look\" faulted/quit: {:?}", r.fault);
    let save = Engine::save_state(&wide);
    let screen = wide.machine.screen.clone();

    // A fresh, narrower session — the SQ-0680 pre-boot pane seed, honestly
    // reporting $21 for a session that never restored anything.
    let Some(mut narrow) = boot_lostpig(NARROW) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    assert_eq!(narrow.boot_screen_cols, NARROW, "this session booted at the narrow pane");
    assert_eq!(
        narrow.machine.mem.read_byte(0x21),
        NARROW as u8,
        "before any restore, $21 answers this session's own narrow boot width"
    );

    // Restore the wide Save State into it.
    Engine::restore_state(&mut narrow, &save).expect("the Save State restores");
    app::session::restore_screen(&mut narrow, screen);

    // SQ-0681's own protection engages exactly as it always has: the floor is
    // raised to the restored (wide) game's own frame of reference. This is
    // NOT a regression — it is the correct starting state SQ-1604 builds on.
    assert_eq!(
        narrow.boot_screen_cols, WIDE,
        "the SQ-0681 floor raises to the restored game's own width immediately after restore"
    );

    // Perturb: play the turn(s) after the restore — CLAUDE.md's convention,
    // and also the mechanism itself: a restore never executes Z-code, so
    // `take_header_width_read` cannot have fired yet, and the release can
    // only happen once the story's own bytecode runs and re-reads $21.
    for cmd in ["", "look"] {
        let r = narrow.submit(cmd);
        assert!(r.fault.is_none() && !r.quit, "{cmd:?} faulted/quit: {:?}", r.fault);
        if narrow.boot_screen_cols == NARROW {
            break;
        }
    }

    // The floor has released: Lost Pig's own `loadb`/`loadw` of $21 while
    // laying its status line out this turn reported the read, and the
    // pre-restore floor came back.
    assert_eq!(
        narrow.boot_screen_cols, NARROW,
        "by the turn after its first redraw, the floor must release back to \
         this session's own width once the story reads $21 itself"
    );
}

/// Negative control, in the same spirit as `zork1_restore_status_width.rs`'s
/// permanently-pinned case: a restore into a WIDER session never raises
/// `pre_restore_screen_cols` at all (the `max` in `note_restored_screen_cols`
/// is a no-op), so there is nothing to release and the floor simply keeps
/// reporting this session's own (already wider) boot width throughout.
#[test]
fn restoring_a_narrower_save_into_a_wider_session_has_nothing_to_release() {
    const WIDE: u16 = 120;
    const NARROW: u16 = 60;

    let Some(mut narrow) = boot_lostpig(NARROW) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let r = narrow.submit("look");
    assert!(r.fault.is_none() && !r.quit);
    let save = Engine::save_state(&narrow);
    let screen = narrow.machine.screen.clone();

    let Some(mut wide) = boot_lostpig(WIDE) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    assert_eq!(wide.boot_screen_cols, WIDE);
    Engine::restore_state(&mut wide, &save).expect("the Save State restores");
    app::session::restore_screen(&mut wide, screen);
    assert_eq!(wide.boot_screen_cols, WIDE, "restoring a narrower save never lowers the floor");

    for cmd in ["", "look", "look"] {
        let r = wide.submit(cmd);
        assert!(r.fault.is_none() && !r.quit, "{cmd:?} faulted/quit: {:?}", r.fault);
    }
    assert_eq!(
        wide.boot_screen_cols, WIDE,
        "with nothing to release, the floor keeps reporting this session's own width"
    );
}
