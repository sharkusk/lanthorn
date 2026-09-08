//! SQ-0778: in Version 6 the text style belongs to a WINDOW, so selecting a
//! window makes that window's style the live one.
//!
//! ZMSD §8.8.3.2 lists the style as window property **10**, and §8.8.3.2.3 says
//! it "is set just as in Version 4, using `set_text_style` (which sets that for
//! the current window)." The style is therefore per-window in v6 exactly as the
//! colour pair is (§8.3) — `set_window` has always mirrored the colours, and had
//! to mirror the style with them.
//!
//! It did not, and the leak had a victim: Amiga Shogun (`James Clavell's
//! Shogun.adf`, release 295 / serial 890321) reverse-videos its status line in
//! window 1 every turn and returns to window 0 without a `set_text_style 0`,
//! because on a conforming interpreter it does not need one. Everything window 0
//! printed from the second turn onwards therefore came out inverted — the `>`
//! prompt, the room headings, the death notice.
//!
//! v1–v5 have one global style and are untouched; the last case pins that.

use zvm::cpu::exec::{Machine, StepResult};
use zvm::memory::Memory;

/// A structurally valid story blob for `version`, with the program area at
/// [`PROG`] inside dynamic memory. Deliberately hand-rolled rather than shared
/// with zvm's in-crate `sample_story`, which is `#[cfg(test)] pub(crate)` and so
/// invisible from an integration test.
fn story(version: u8) -> Vec<u8> {
    let mut buf = vec![0u8; 0x400];
    buf[0x00] = version;
    buf[0x04] = 0x04; // high memory base 0x0400
    buf[0x06] = 0x00; // initial PC / v6 packed `main`
    buf[0x07] = 0x40;
    buf[0x08] = 0x02; // dictionary
    buf[0x0A] = 0x01; // object table
    buf[0x0C] = 0x03; // globals
    buf[0x0E] = 0x04; // static memory base 0x0400
    buf[0x18] = 0x00; // abbreviations
    buf[0x19] = 0x40;
    buf
}

/// Where every program below is assembled and executed from. Inside dynamic
/// memory, clear of the header and of the table addresses named in it.
const PROG: usize = 0x0140;

/// Append one VAR-form instruction taking a single small-constant operand:
/// opcode byte, then a type byte of `small, omitted, omitted, omitted`.
fn var1(buf: &mut Vec<u8>, op: u8, operand: u8) {
    buf.push(0b1110_0000 | op);
    buf.push(0b01_11_11_11);
    buf.push(operand);
}

const SET_WINDOW: u8 = 0x0B;
const SET_TEXT_STYLE: u8 = 0x11;
const QUIT: u8 = 0xBA; // 0OP:0x0A

/// Assemble `prog` into a `version` story at [`PROG`] and run it to `quit`.
fn run(version: u8, prog: &[u8]) -> Machine {
    let mut buf = story(version);
    buf[PROG..PROG + prog.len()].copy_from_slice(prog);
    let mem = Memory::new(buf).expect("the hand-rolled header is structurally valid");
    let mut m = Machine::new(mem);
    m.state.set_pc(PROG as u32);
    for _ in 0..64 {
        if matches!(m.step(), StepResult::Quit) {
            return m;
        }
    }
    panic!("program never reached quit");
}

/// The style window 1 was given must not follow the game back to window 0.
///
/// Falsified by dropping the `screen.text_style` restore from `set_window`
/// (`crates/zvm/src/cpu/exec.rs`, VAR:0x0B):
///
/// ```text
/// selecting window 0 must make WINDOW 0's style live — the reverse video
/// window 1 was given is not window 0's, and Amiga Shogun's `>` prompt inherited
/// it: left as 1
/// ```
#[test]
fn selecting_a_window_makes_that_windows_style_live() {
    let mut prog = Vec::new();
    var1(&mut prog, SET_WINDOW, 1);
    var1(&mut prog, SET_TEXT_STYLE, 1); // reverse video, in window 1
    var1(&mut prog, SET_WINDOW, 0);
    prog.push(QUIT);
    let m = run(6, &prog);

    assert_eq!(
        m.screen.text_style, 0,
        "selecting window 0 must make WINDOW 0's style live — the reverse video window 1 \
         was given is not window 0's, and Amiga Shogun's `>` prompt inherited it: left as {}",
        m.screen.text_style
    );
    let v6 = m.screen.v6.as_ref().expect("a v6 story has the eight-window model");
    assert_eq!(
        v6.windows[1].text_style, 1,
        "…and window 1 KEEPS it: ZMSD §8.8.3.2 makes the style window property 10, which a \
         `get_wind_prop(1, 10)` must still read back as reverse"
    );
    assert_eq!(v6.windows[0].text_style, 0, "window 0 was never given a style");
}

/// The other direction, which is the half a naive "reset the style on every
/// `set_window`" would get wrong: a style set in window 0 survives an excursion
/// into window 1 and back.
#[test]
fn a_windows_own_style_is_restored_when_it_is_reselected() {
    let mut prog = Vec::new();
    var1(&mut prog, SET_TEXT_STYLE, 2); // bold, in window 0 (initially selected)
    var1(&mut prog, SET_WINDOW, 1);
    var1(&mut prog, SET_TEXT_STYLE, 1); // reverse, in window 1
    var1(&mut prog, SET_WINDOW, 0);
    prog.push(QUIT);
    let m = run(6, &prog);

    assert_eq!(
        m.screen.text_style, 2,
        "window 0 was bold before the excursion and is bold after it — the style is REMEMBERED \
         per window, not cleared on every select"
    );
    let v6 = m.screen.v6.as_ref().expect("a v6 story has the eight-window model");
    assert_eq!(v6.windows[1].text_style, 1, "window 1 still holds its own reverse");
}

/// v1–v5 have one global style and no window property table to hold another —
/// `set_window` there must leave the style exactly where the game put it.
#[test]
fn below_v6_the_style_is_global_and_set_window_does_not_touch_it() {
    let mut prog = Vec::new();
    var1(&mut prog, SET_TEXT_STYLE, 1); // reverse
    var1(&mut prog, SET_WINDOW, 1);
    var1(&mut prog, SET_WINDOW, 0);
    prog.push(QUIT);
    let m = run(5, &prog);

    assert!(m.screen.v6.is_none(), "a v5 story has no eight-window model");
    assert_eq!(
        m.screen.text_style, 1,
        "v5 keeps one style for the whole screen (ZMSD §8.7.2); switching windows must not \
         disturb it"
    );
}
