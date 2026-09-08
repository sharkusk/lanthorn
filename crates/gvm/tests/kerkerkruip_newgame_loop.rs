//! SQ-0299 regression: a `glk_select` waiting only on a timer must yield
//! (`StepResult::NeedEvent`) so the host can deliver ticks, instead of spinning
//! on `evtype_None`. Kerkerkruip's "New game" intro arms a 130 ms timer and
//! selects on it; ~10 delivered ticks carry it through the intro cutscene into
//! the Entrance Hall. Before the fix this looped to the step cap. Skip-if-absent.

use std::path::PathBuf;

use gvm::{Machine, Memory, StepResult, TestBackend};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn extract_glulx(bytes: Vec<u8>) -> Vec<u8> {
    if !blorb::Blorb::is_blorb(&bytes) {
        return bytes;
    }
    let b = blorb::Blorb::parse(bytes).expect("valid Blorb");
    match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => data.to_vec(),
        _ => panic!("expected a Glulx Blorb"),
    }
}

fn text(m: &mut Machine) -> String {
    m.backend_mut()
        .as_any_mut()
        .downcast_mut::<TestBackend>()
        .unwrap()
        .all_text()
}

fn boot() -> Option<Machine> {
    let path = stories_dir().join("Kerkerkruip.gblorb");
    let raw = std::fs::read(&path).ok()?;
    let image = extract_glulx(raw);
    let mem = Memory::new(image).expect("valid Glulx image");
    Some(Machine::with_glk(mem, Box::new(TestBackend::new())))
}

/// Step until the next real input stop, delivering a timer tick on every
/// `NeedEvent { timer_ms: Some(_) }` (the host clock's job). Returns a tag.
fn run_delivering_timers(m: &mut Machine, budget: u64) -> String {
    let mut steps = 0u64;
    loop {
        match m.step() {
            StepResult::Continue => {
                steps += 1;
                if steps >= budget {
                    return "BUDGET".into();
                }
            }
            StepResult::NeedEvent { timer_ms: Some(_), .. } => m.deliver_timer(),
            StepResult::NeedEvent { .. } => return "NeedEvent(no-timer)".into(),
            StepResult::NeedLine { win } => return format!("NeedLine win={win}"),
            StepResult::NeedChar { win, unicode } => {
                return format!("NeedChar win={win} uni={unicode}")
            }
            StepResult::Quit => return "Quit".into(),
            StepResult::Fault => return "Fault".into(),
            StepResult::SaveRequest => return "SaveRequest".into(),
            StepResult::RestoreRequest => return "RestoreRequest".into(),
            StepResult::NeedFilename { .. } => return "NeedFilename".into(),
            _ => return "Unknown".into(),
        }
    }
}

#[test]
#[ignore = "slow (~13s): boots Kerkerkruip through many timer turns; run with --include-ignored"]
fn new_game_timer_wait_reaches_entrance_hall() {
    let Some(mut m) = boot() else {
        eprintln!("SKIP: stories/Kerkerkruip.gblorb missing");
        return;
    };

    // Boot to the screen-reader mode prompt (a char request on win 4).
    let tag = run_delivering_timers(&mut m, 100_000_000);
    assert!(tag.starts_with("NeedChar"), "boot reaches the mode prompt, got {tag}");

    // Answer "no" to the screen reader, reaching the ACTIONS menu (char request).
    m.supply_char('n' as u32);
    let tag = run_delivering_timers(&mut m, 100_000_000);
    assert!(tag.starts_with("NeedChar"), "reaches the menu, got {tag}");

    // Select "New game" (SPACE). Its intro arms a 130 ms timer and selects on it;
    // delivering ticks must carry the VM through the cutscene to a real stop.
    m.supply_char(0x20);
    let tag = run_delivering_timers(&mut m, 40_000_000);

    // A real input/save stop, NOT the step-cap spin the bug produced.
    assert!(
        tag == "SaveRequest" || tag.starts_with("NeedLine") || tag.starts_with("NeedChar"),
        "new game reaches a real input stop within budget, got {tag}"
    );
    let t = text(&mut m);
    assert!(
        t.contains("Entrance Hall"),
        "transcript reaches the Entrance Hall (cleared the intro); tail:\n{}",
        t.chars().rev().take(600).collect::<String>().chars().rev().collect::<String>()
    );
    assert_eq!(
        m.diagnostics.iter().filter(|d| d.contains("no pending input request")).count(),
        0,
        "no evtype_None spin diagnostics: {:?}",
        m.diagnostics
    );
}
