//! SQ-1088 real-game smoke: Anchorhead's HELP menu must keep its bottom rows
//! while the player arrows down it.
//!
//! `anchor.z8` (Inform 6) opens HELP with `@erase_window(-1)`,
//! `@split_window(7)`, `@set_window(upper)` and then prints **thirteen** rows
//! into window 1 — six entries on rows 5–10 and
//! `[press BACKSPACE to return to game]` on row 13. Six of those rows sit below
//! the game's own 7-row split, which is legal and is what real interpreters show:
//! ZMSD §8.6.1.1.1, "Printing onto the upper window overlays whatever text is
//! already there", and nothing scrolls over it while the menu is up.
//!
//! Each arrow key is a `read_char`. SQ-0696's quote-box retirement fired on every
//! one of them, truncating the grid back to the split — so the last three entries
//! and the BACKSPACE line vanished on the first press, in lanthorn and `zvm-cli`
//! alike, while `set_cursor` went on selecting rows that were no longer painted.
//!
//! A unit test pins the mechanism (`rows_painted_below_the_split_survive_a_keypress`
//! in `cpu::exec`); this drives the real menu, because the mechanism is only worth
//! anything if it is the shape a shipped game actually uses.
//!
//! The story is gitignored (see CLAUDE.md), so this skips cleanly when absent.

use std::path::PathBuf;

use zvm::cpu::exec::{Machine, StepResult};
use zvm::memory::Memory;
use zvm::text::input::ZsciiInput;

/// ZSCII cursor down (ZMSD §3.8) — the key that drives the menu.
const CURSOR_DOWN: ZsciiInput = ZsciiInput::DOWN;

fn story() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/anchor.z8");
    match std::fs::read(&p) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", p.display());
            None
        }
    }
}

/// Run until the machine wants input (or gives up), returning what it wants.
fn run_to_input(m: &mut Machine) -> StepResult {
    for _ in 0..200_000_000u64 {
        match m.step() {
            r @ (StepResult::NeedChar | StepResult::NeedLine { .. } | StepResult::Quit) => return r,
            StepResult::Fault => panic!("anchor.z8 faulted: {:?}", m.take_fault_trace()),
            _ => {}
        }
    }
    panic!("anchor.z8 never asked for input");
}

fn row_text(m: &Machine, row: u16) -> String {
    (1..=m.screen.upper.cols).map(|c| m.screen.upper.cell(row, c).ch).collect::<String>().trim_end().to_string()
}

#[test]
fn anchor_help_menu_keeps_its_bottom_rows_while_arrowing_down() {
    let Some(bytes) = story() else { return };
    let mem = Memory::new(bytes).expect("anchor.z8 is a valid v8 story");
    let mut m = Machine::new(mem);
    m.set_screen_dims(25, 80);

    // Three `read_char` intro cards (title plate and two Lovecraft box quotes)
    // stand between boot and the first prompt.
    for _ in 0..8 {
        match run_to_input(&mut m) {
            StepResult::NeedChar => m.supply_char(ZsciiInput::NEWLINE),
            StepResult::NeedLine { .. } => break,
            other => panic!("anchor.z8 did not reach its prompt: {other:?}"),
        }
    }
    m.supply_line("help", 13);
    assert!(matches!(run_to_input(&mut m), StepResult::NeedChar), "the menu waits on a key");

    // The frame this case is pinned to: the FIRST paint of the menu, one `help`
    // command in. Non-vacuity guard — if the menu ever stops being thirteen rows
    // behind a seven-row split, every assertion below is measuring another screen.
    assert_eq!(m.screen.upper_window_rows, 7, "the menu's own split");
    assert_eq!(m.screen.upper.rows, 13, "…with thirteen rows painted behind it");
    let backspace_line = row_text(&m, 13);
    assert!(
        backspace_line.contains("BACKSPACE"),
        "row 13 carries the return-to-game line: {backspace_line:?}"
    );
    let entries: Vec<String> = (5..=10).map(|r| row_text(&m, r)).collect();
    assert!(
        entries.iter().all(|e| e.len() > 4),
        "six menu entries on rows 5–10: {entries:?}"
    );

    // Now arrow down through the whole menu. Every row must still be there.
    for press in 1..=6u32 {
        m.supply_char(CURSOR_DOWN);
        assert!(matches!(run_to_input(&mut m), StepResult::NeedChar), "still a menu after press {press}");
        assert_eq!(
            m.screen.upper.rows, 13,
            "press {press}: the menu's painted height must survive the keypress"
        );
        let now: Vec<String> = (5..=10).map(|r| row_text(&m, r)).collect();
        for (before, after) in entries.iter().zip(now.iter()) {
            // Only the leading selection marker moves; the entry text stays.
            let strip = |s: &str| s.trim_start_matches([' ', '>']).to_string();
            assert_eq!(
                strip(before), strip(after),
                "press {press}: entry text must not be erased ({before:?} -> {after:?})"
            );
        }
        assert_eq!(row_text(&m, 13), backspace_line, "press {press}: the BACKSPACE line stays");
    }
}
