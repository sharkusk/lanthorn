// SQ-0158 — READ-direction save-format interop.
//
// Proves lanthorn's zvm can restore a bare Quetzal `.qzl` save produced by a
// *different* interpreter (`dfrotz`) and land in the same state a native
// play-through would reach. The golden fixture's PC points at the `save`
// instruction's result descriptor (Quetzal §5.8), so this exercises the
// descriptor-COMPLETING restore path (`complete_restore_success`), not a
// bare resume.
//
// See `crates/zvm/tests/fixtures/interop/PROVENANCE.md` for how the golden
// was produced.

use zvm::cpu::exec::{Machine, StepResult};
use zvm::memory::Memory;

/// Verbatim commands that reach interop point P: room "North of House",
/// leaflet carried.
const PREFIX: [&str; 3] = ["open mailbox", "take leaflet", "north"];

/// Verbatim commands that reveal the room and the carried leaflet.
const PROBE: [&str; 2] = ["look", "inventory"];

/// Boot a story and run until the first line-read prompt (or a step cap),
/// answering any char-reads with '\n' and refusing save/restore along the
/// way. Mirrors `story_location_verify.rs`'s `boot_to_first_read`.
fn boot_to_first_read(data: Vec<u8>) -> Machine {
    let mem = Memory::new(data).expect("valid story file");
    let mut machine = Machine::new(mem);
    machine.init_caps();
    for _ in 0..2_000_000u64 {
        match machine.step() {
            StepResult::NeedLine { .. } => return machine,
            StepResult::Quit | StepResult::Restart | StepResult::Fault => return machine,
            StepResult::Continue => {}
            StepResult::NeedChar => machine.supply_char(b'\n'),
            StepResult::SaveRequest => machine.complete_save(false),
            StepResult::RestoreRequest => machine.complete_restore_failure(),
            _ => return machine,
        }
    }
    panic!("boot_to_first_read: never reached a line-read prompt within step cap");
}

/// Drive `machine` for one more turn by supplying `input` as a line, stepping
/// until the next prompt (or a step cap). Mirrors `run_one_turn`.
fn run_one_turn(machine: &mut Machine, input: &str) {
    machine.supply_line(input, 13);
    for _ in 0..2_000_000u64 {
        match machine.step() {
            StepResult::NeedLine { .. } | StepResult::Quit | StepResult::Restart | StepResult::Fault => return,
            StepResult::Continue => {}
            StepResult::NeedChar => machine.supply_char(b'\n'),
            StepResult::SaveRequest => machine.complete_save(false),
            StepResult::RestoreRequest => machine.complete_restore_failure(),
            _ => return,
        }
    }
    panic!("run_one_turn({input:?}): never reached the next prompt within step cap");
}

/// Drive `machine` until the next line-read prompt (or a step cap) WITHOUT
/// supplying any input line first. Needed right after
/// `complete_restore_success`: the restored PC resumes mid-turn (completing
/// whichever command the foreign interpreter was mid-executing when it saved,
/// e.g. printing "Ok." for the `save` verb itself) before it reaches the next
/// actual prompt.
fn drain_to_next_read(machine: &mut Machine) {
    for _ in 0..2_000_000u64 {
        match machine.step() {
            StepResult::NeedLine { .. } | StepResult::Quit | StepResult::Restart | StepResult::Fault => return,
            StepResult::Continue => {}
            StepResult::NeedChar => machine.supply_char(b'\n'),
            StepResult::SaveRequest => machine.complete_save(false),
            StepResult::RestoreRequest => machine.complete_restore_failure(),
            _ => return,
        }
    }
    panic!("drain_to_next_read: never reached the next prompt within step cap");
}

fn run_turns(machine: &mut Machine, cmds: &[&str]) {
    for cmd in cmds {
        run_one_turn(machine, cmd);
    }
}

/// Transcript text accumulated in `machine`'s buffer output since `mark`.
fn transcript_since(machine: &Machine, mark: usize) -> String {
    let buf = &machine.buffer_output().expect("buffer output sink").buf;
    buf[mark..].to_string()
}

#[test]
fn zmachine_reads_reference_save() {
    let story = zvm::fixtures::load("minizork.z3").expect("required CI fixture minizork.z3 missing");
    let golden = zvm::fixtures::load("interop/minizork-at-P.qzl")
        .expect("required CI fixture interop/minizork-at-P.qzl missing");

    // Baseline: boot minizork, play PREFIX then PROBE, capture the PROBE-phase transcript.
    let played = {
        let mut machine = boot_to_first_read(story.clone());
        run_turns(&mut machine, &PREFIX);
        let mark = machine.buffer_output().expect("buffer output sink").buf.len();
        run_turns(&mut machine, &PROBE);
        transcript_since(&machine, mark)
    };

    // Cross-load: boot a FRESH minizork, descriptor-complete the foreign
    // dfrotz save, then run the SAME PROBE, capture only the PROBE-phase
    // transcript. Two things must be excluded: the boot banner/initial room,
    // and the tail of dfrotz's own `save` turn that the restored PC resumes
    // mid-execution (it prints "Ok." before reaching the next real prompt).
    let restored = {
        let mut machine = boot_to_first_read(story);
        machine
            .complete_restore_success(&golden)
            .expect("restoring the dfrotz golden save must succeed");
        drain_to_next_read(&mut machine);
        let mark = machine.buffer_output().expect("buffer output sink").buf.len();
        run_turns(&mut machine, &PROBE);
        transcript_since(&machine, mark)
    };

    assert_eq!(
        restored.trim(),
        played.trim(),
        "restoring dfrotz's save must reproduce the state reached by playing the prefix"
    );
    assert!(
        restored.contains("leaflet"),
        "probe output must reveal the mutated state (guards against a vacuous match)"
    );
}

// SQ-0158 — WRITE-direction save-format interop.
//
// Proves a save that *lanthorn writes* (via the game's `@save`) is read
// correctly by the reference interpreter `dfrotz`. Compares two dfrotz runs
// through the identical dfrotz code path: A loads lanthorn's save, B loads
// dfrotz's own committed golden save. Both encode point P; if lanthorn wrote
// a correct, dfrotz-readable save, A and B produce byte-identical output.

/// Drive minizork through PREFIX and the game's own `save` verb, capturing
/// the descriptor-PC Quetzal bytes `save_quetzal` emits when `pending_save`
/// is set (the same convention an in-game `@save` produces). Writes the
/// bytes to a unique temp file and returns its path.
fn lanthorn_save_at_p() -> std::path::PathBuf {
    let story = zvm::fixtures::load("minizork.z3").expect("required CI fixture minizork.z3 missing");
    let mut machine = boot_to_first_read(story);
    run_turns(&mut machine, &PREFIX);

    machine.supply_line("save", 13);
    let bytes = 'save: {
        for _ in 0..2_000_000u64 {
            match machine.step() {
                StepResult::SaveRequest => break 'save machine.save_quetzal(),
                StepResult::NeedChar => machine.supply_char(b'\n'),
                StepResult::RestoreRequest => machine.complete_restore_failure(),
                StepResult::Continue => {}
                StepResult::NeedLine { .. } | StepResult::Quit | StepResult::Restart | StepResult::Fault => {
                    panic!("lanthorn_save_at_p: expected a SaveRequest from the `save` verb but the machine reached a different terminal state first");
                }
                _ => {}
            }
        }
        panic!("lanthorn_save_at_p: never reached SaveRequest within step cap");
    };
    machine.complete_save(true);

    // A counter beside the pid: under `cargo test` one binary's tests share a
    // process, so the pid alone would hand every caller the same file (SQ-1131).
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NTH: AtomicUsize = AtomicUsize::new(0);
    let nth = NTH.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("lanthorn-158b-{}-{nth}.qzl", std::process::id()));
    std::fs::write(&path, &bytes).expect("write lanthorn's save to a temp file");
    path
}

/// Run dfrotz against `save_path`, piping `look`/`inventory`/`quit`/`y` and
/// returning stdout. Uses an absolute story path (built from
/// `CARGO_MANIFEST_DIR`) since integration tests run with CWD = the crate
/// directory, not the repo root.
fn dfrotz_probe(save_path: &std::path::Path) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let story = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minizork.z3");
    let mut child = Command::new("dfrotz")
        .args(["-w", "80", "-L"])
        .arg(save_path)
        .arg(&story)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("dfrotz failed to spawn (this --ignored test requires dfrotz on PATH): {e}"));
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"look\ninventory\nquit\ny\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
#[ignore = "needs dfrotz on PATH; run with: cargo test -p zvm --test save_interop -- --ignored"]
fn zmachine_save_read_by_dfrotz() {
    // A: dfrotz loads lanthorn's save.
    let bab = lanthorn_save_at_p();
    let a = dfrotz_probe(&bab);
    // B: dfrotz loads its own golden save.
    let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/interop/minizork-at-P.qzl");
    let b = dfrotz_probe(&golden);
    let _ = std::fs::remove_file(&bab);

    assert!(
        a.contains("North of House") && a.contains("leaflet"),
        "dfrotz reading lanthorn's save must reveal point-P state (non-vacuous guard):\n{a}"
    );
    assert_eq!(
        a.trim(),
        b.trim(),
        "dfrotz reading lanthorn's save must match dfrotz reading its own golden save"
    );
}
