//! SQ-0706 real-game smoke: scopa draws its cards with `erase_window`, and the
//! engine must publish every one of those filled rectangles.
//!
//! scopa.z6 (Mike Roberts' Scopa, 2011) uses no `draw_picture` for its deck at
//! all. From the game's own source, `drawpic` sends anything numbered 40 or below
//! to `HardPic`, which decodes run-length card data and emits each run through:
//!
//! ```text
//! [ fastsimplebox x y w h c;
//!     @window_size 3 h w;
//!     @move_window 3 y x;
//!     @set_colour 2 c 3;
//!     @erase_window 3;
//! ];
//! ```
//!
//! So one card is hundreds of moves-and-erases of a single window. A host that
//! reads only the canvas-clear sentinel keeps one empty canvas at the last
//! position and draws nothing — which is exactly what lanthorn did.
//!
//! A unit test cannot catch a regression here: it would assert the same shape the
//! implementation assumes. This drives the real story and counts real fills.
//!
//! The story is gitignored (see CLAUDE.md), so this skips cleanly when absent.

use std::path::PathBuf;

use zvm::cpu::exec::{Machine, PaintEvent, StepResult};
use zvm::memory::Memory;

fn story() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/scopa.z6");
    match std::fs::read(&p) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", p.display());
            None
        }
    }
}

#[test]
fn scopa_paints_its_cards_as_erase_window_fills() {
    let Some(bytes) = story() else { return };
    let mem = Memory::new(bytes).expect("scopa is a valid v6 story");
    let mut m = Machine::new(mem);

    // The opening MENU paints nothing — the fills start once a game begins, so the
    // test has to click into one. scopa's menu is mouse-driven: a click arrives at
    // `read_char` as ZSCII 254 (ZMSD §3.8) with the coordinates already set, and
    // the menu sits around y=230 in native pixels.
    let mut fills = 0usize;
    let mut clicks = 0u32;
    for _ in 0..50_000_000u64 {
        match m.step() {
            StepResult::NeedChar => {
                if clicks > 12 {
                    break; // the game is asking for something a click will not answer
                }
                m.set_mouse(230, 320, 0b1);
                m.supply_char(254);
                clicks += 1;
            }
            StepResult::NeedLine { .. } => m.supply_line("", 13),
            StepResult::Fault => panic!("scopa faulted while painting: {:?}", m.take_fault_trace()),
            _ => {}
        }
        fills = m.pending_erase_fills().len();
        if fills > 200 {
            break; // plenty: the mechanism is proven well before the whole deck is dealt
        }
    }
    let fills_at_boot = fills;
    assert!(
        fills_at_boot > 50,
        "scopa paints its table with erase_window fills — the engine published only {fills_at_boot}. \
         Before SQ-0706 this was 0 and no card could ever be drawn."
    );

    // Degenerate fills are expected and legitimate: erasing a window that was
    // never given a box (or a collapsed window 1) covers no pixels, and the host
    // simply skips it. What matters is that the PAINTING ones are real.
    let painted: Vec<_> = m.pending_erase_fills().iter().filter(|f| f.w > 0 && f.h > 0).collect();
    assert!(
        painted.len() > 50,
        "most fills must cover real pixels: only {} of {} did",
        painted.len(),
        m.pending_erase_fills().len()
    );

    // They land at MANY different positions — one window is moved between every
    // fill, which is the whole mechanism. A host keyed on the window's current
    // rect (as lanthorn was) collapses all of this to a single box.
    let distinct_positions =
        painted.iter().map(|f| (f.x, f.y)).collect::<std::collections::BTreeSet<_>>().len();
    assert!(
        distinct_positions > 10,
        "the fills must land at many different positions — one moved window paints a whole card; \
         got {distinct_positions} distinct origins across {} painted fills",
        painted.len()
    );

    // And they carry colour: a card is drawn as runs of DIFFERENT colours (the
    // face, the pips, the border), so a host that ignored `bg` would paint a
    // featureless block.
    let distinct_colours =
        painted.iter().map(|f| format!("{:?}", f.bg)).collect::<std::collections::BTreeSet<_>>().len();
    assert!(
        distinct_colours > 1,
        "fills carry the colour to paint with; got only {distinct_colours} distinct background(s)"
    );
}

/// SQ-1396: the same real turn, drained through `take_paint_events`, comes back
/// genuinely INTERLEAVED — not every picture and then every fill.
///
/// This is the half a unit test cannot settle. scopa's boot fills the green
/// table, draws its Neapolitan and Sicilian card pictures and then fills the menu
/// buttons over the top, so a host replaying all the fills last erases both cards
/// it had already painted. The interleave used to be the HOST's to reconstruct
/// from `EraseFill::pics_before`; now there is no other way to drain.
#[test]
fn scopa_drains_its_paints_interleaved() {
    let Some(bytes) = story() else { return };
    let mem = Memory::new(bytes).expect("scopa is a valid v6 story");
    let mut m = Machine::new(mem);

    let mut clicks = 0u32;
    for _ in 0..50_000_000u64 {
        match m.step() {
            StepResult::NeedChar => {
                if clicks > 12 {
                    break;
                }
                m.set_mouse(230, 320, 0b1);
                m.supply_char(254);
                clicks += 1;
            }
            StepResult::NeedLine { .. } => m.supply_line("", 13),
            StepResult::Fault => panic!("scopa faulted while painting: {:?}", m.take_fault_trace()),
            _ => {}
        }
        if m.pending_erase_fills().len() > 200 {
            break;
        }
    }
    let (fills, pictures) = (m.pending_erase_fills().len(), m.pending_pictures().len());
    assert!(fills > 50 && pictures > 0, "the turn must hold both kinds: {fills} fills, {pictures} pictures");

    let drained = m.take_paint_events();
    assert_eq!(drained.len(), fills + pictures, "the drain loses nothing");
    assert!(
        m.pending_erase_fills().is_empty() && m.pending_pictures().is_empty(),
        "and empties both queues",
    );

    // Both directions, because either alone is satisfied by a batch that is
    // simply all of one kind and then all of the other.
    let first_picture = drained.iter().position(|e| matches!(e, PaintEvent::Picture(_)));
    let last_picture = drained.iter().rposition(|e| matches!(e, PaintEvent::Picture(_)));
    let first_fill = drained.iter().position(|e| matches!(e, PaintEvent::Erase(_)));
    let last_fill = drained.iter().rposition(|e| matches!(e, PaintEvent::Erase(_)));
    assert!(
        matches!((first_picture, last_fill), (Some(p), Some(f)) if f > p),
        "a real scopa turn ends with fills after its pictures",
    );
    assert!(
        matches!((first_fill, last_picture), (Some(f), Some(p)) if p > f),
        "…and STARTS with fills before them — draining the two queues in series \
         would have put every fill after every picture, which is the bug",
    );
}

/// SQ-1403: `PaintLog::apply` is the host's to call, once per event drained
/// from `take_paint_events` (see `zvm::paint_log`'s "Feeding it" for why it is
/// not fed automatically at opcode-execution time — a cross-window erase can
/// only be computed from the SAME drain walk, and feeding early would put
/// every such entry after the whole turn's engine events regardless of how
/// they were truly interleaved). What the fold algorithm itself DOES
/// guarantee is that feeding it makes no difference: draining scopa's hundreds
/// of moved-and-erased-window card fills in small bursts (one drain per VM
/// step) and folding them as they arrive must land on the exact same log as
/// draining the WHOLE run in one call at the end and folding it all at once —
/// two hosts on different drain schedules must see identical results.
#[test]
fn scopa_paint_log_is_the_same_whatever_the_drain_schedule() {
    let Some(bytes) = story() else { return };

    // Host A: drains and folds on every VM step (fine-grained).
    let mem_a = Memory::new(bytes.clone()).expect("scopa is a valid v6 story");
    let mut m_a = Machine::new(mem_a);
    let mut log_a = zvm::paint_log::PaintLog::default();
    let mut all_events: Vec<PaintEvent> = Vec::new();
    let mut clicks = 0u32;
    for _ in 0..50_000_000u64 {
        match m_a.step() {
            StepResult::NeedChar => {
                if clicks > 12 {
                    break;
                }
                m_a.set_mouse(230, 320, 0b1);
                m_a.supply_char(254);
                clicks += 1;
            }
            StepResult::NeedLine { .. } => m_a.supply_line("", 13),
            StepResult::Fault => panic!("scopa faulted while painting: {:?}", m_a.take_fault_trace()),
            _ => {}
        }
        for ev in m_a.take_paint_events() {
            log_a.apply(&ev);
            all_events.push(ev);
        }
        if all_events.len() > 400 {
            break; // well past the fold rules exercised by the unit tests
        }
    }
    assert!(all_events.len() > 100, "the run must exercise real fold traffic; only {} events", all_events.len());

    // Host B: folds the SAME events, collected all at once, from a single pass.
    let mut log_b = zvm::paint_log::PaintLog::default();
    for ev in &all_events {
        log_b.apply(ev);
    }

    for win in 0u8..8 {
        assert_eq!(
            log_a.ops(win),
            log_b.ops(win),
            "window {win}: the fold must not depend on how often the host drained"
        );
    }
}
