//! Full-story acceleration on/off equivalence + speed proof.
//!
//! These tests load real Inform Glulx stories from the gitignored `stories/`
//! directory (a local symlink present in dev worktrees but absent in CI and
//! fresh clones). They are the primary anti-divergence guarantee: with
//! acceleration ON, the transcript must be byte-identical to acceleration OFF,
//! and the opcode count must drop substantially (accelerated calls bypass
//! `step_once`, so their opcodes vanish from `insn_count`).
//!
//! Reaching a prompt means being a host, not just a stepper: a game may run
//! its OWN fixed-name `@save`/`@restore` during startup (Counterfeit Monkey
//! writes a `_Counterfeit_Monkey-startup-data` init cache before it ever asks
//! for input), and those are serviced here the way `app`'s
//! `glulx_session::drive_auto` services one against a read-only store — a clean
//! failure, nothing written. Only the *player's* prompted SAVE/RESTORE verb is
//! a defect this early.
//!
//! Two families of case live here:
//!
//! * The named cases (`counterfeit_monkey_…`, `take_…`) pin one story each and
//!   are `#[ignore]`d, because the largest of them is minutes of debug-build
//!   interpretation.
//! * [`fingerprinted_stories_are_transcript_identical_on_and_off`] sweeps every
//!   Glulx story under `stories/` that never calls `@accelfunc` for itself —
//!   the games SQ-1209's bytecode fingerprinting exists for — drives a command
//!   battery through each, and asserts the transcript and the game-managed
//!   save/restore sequence are identical with fingerprinted acceleration on and
//!   off. It skips vacuously when `stories/` is absent and prints what it ran.
//!
//! Run the ignored ones manually with:
//!
//! ```sh
//! cargo test -p gvm --test accel_story_equivalence -- --ignored --nocapture
//! ```
//!
//! (add `--release` if the debug build is too slow for CounterfeitMonkey).

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use gvm::{Machine, Memory, StepResult, TestBackend};

/// A story's own startup output, used as a non-vacuity anchor: the transcript
/// compared on/off must actually contain this, so a boot that stops early (or a
/// harness that reaches the prompt without the game having spoken) fails loudly
/// instead of comparing two empty strings.
const CM_OPENING: &str = "Can you hear me?";
/// The same anchor for `TAKE.gblorb` (see [`CM_OPENING`]).
const TAKE_OPENING: &str = "One joke, until expiration, by Amelia Pinnolla";

/// The repo-root `stories/` directory (gitignored symlink), resolved relative
/// to this crate's manifest so the tests work regardless of `cargo test`'s
/// working directory.
fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Ceiling on opcode steps while driving to the first prompt. If a story
/// never reaches a `NeedLine`/`NeedChar` within this many steps, something is
/// wrong (infinite loop, VM bug, or a story that never asks for input) —
/// panic loudly rather than hang the test run.
const MAX_STEPS: u64 = 300_000_000;

/// Extract the Glulx executable from Blorb-wrapped `bytes` (or pass through
/// plain `.ulx` bytes unchanged), mirroring `gvm-cli`'s `extract_executable`.
fn extract_glulx(bytes: Vec<u8>) -> Vec<u8> {
    if !blorb::Blorb::is_blorb(&bytes) {
        return bytes;
    }
    let b = blorb::Blorb::parse(bytes).expect("valid Blorb");
    match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => data.to_vec(),
        Ok((blorb::ExecKind::ZCode | blorb::ExecKind::Scott, _)) => {
            panic!("expected a Glulx Blorb")
        }
        Err(e) => panic!("Blorb has no executable: {e:?}"),
    }
}

/// What one on/off run produced.
struct Run {
    text: String,
    ops: u64,
    saveloads: Vec<String>,
}

/// Ceiling on the number of *input events* one `play` answers — commands plus
/// keypresses plus timer deliveries. This, not a step count, is what bounds the
/// run: the two runs being compared must stop at the same point in the STORY,
/// and a step budget stops the accelerated one later in the game than the
/// interpreted one, which would make every transcript differ.
const MAX_INPUTS: usize = 60;

/// Keys answered to a `NeedChar`, in order and then cycling. A space clears a
/// "press SPACE to continue" card; `s` picks "Start the story - without the
/// tutorial" out of King of Shreds and Patches' opening menu, without which the
/// battery below never reaches the game; a newline covers everything else. Both
/// runs answer identically, so a key that means something odd in some story
/// costs coverage, never correctness.
const KEYS: &[u8] = b" s\n";

/// Build a machine over `image`, set acceleration to `accel`, drive it to the
/// first input prompt, then feed `commands` one at a time, driving to the next
/// prompt after each. Returns the full text-buffer transcript, the opcode
/// count, and the game-managed `@save`/`@restore` requests serviced on the way
/// (acceleration must not change *those* either).
fn play_named(name: &str, image: Vec<u8>, accel: bool, commands: &[&str]) -> Run {
    let mem = Memory::new(image).expect("valid Glulx image");
    let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
    m.set_acceleration(accel);

    let mut steps = 0u64;
    let mut saveloads = Vec::new();
    let mut next = 0usize;
    let mut inputs = 0usize;
    let mut keys = 0usize;
    loop {
        match m.step() {
            StepResult::Continue => {
                steps += 1;
                assert!(
                    steps < MAX_STEPS,
                    "{name}: runaway, {MAX_STEPS} steps without reaching an input prompt \
                     (accel={accel})"
                );
            }
            StepResult::NeedLine { .. } => {
                let Some(cmd) = commands.get(next) else { break };
                next += 1;
                inputs += 1;
                m.supply_line(cmd);
            }
            StepResult::NeedChar { .. } => {
                // Intro cards, "press any key" gates and opening menus. A story
                // that only ever wants keys (a menu that never returns to a line
                // prompt) is bounded by `MAX_INPUTS` rather than driven forever.
                if inputs >= MAX_INPUTS {
                    break;
                }
                m.supply_char(u32::from(KEYS[keys % KEYS.len()]));
                keys += 1;
                inputs += 1;
            }
            StepResult::NeedEvent { timer_ms: Some(_), .. } => {
                if inputs >= MAX_INPUTS {
                    break;
                }
                inputs += 1;
                m.deliver_timer()
            }
            StepResult::NeedEvent { .. } => {
                panic!("{name}: unexpected non-timer event wait (accel={accel})")
            }
            StepResult::Quit => break,
            // A game's OWN fixed-name save is startup, not a defect: Counterfeit
            // Monkey `@save`s a `_Counterfeit_Monkey-startup-data` init cache
            // before it ever asks for input, so a host that panics here can
            // never reach the story's first prompt. Answer it exactly the way
            // `app`'s `glulx_session::drive_auto` answers one against a
            // read-only store — a clean failure, nothing written — which is the
            // answer any story's very first run already gets and handles. Only
            // the PLAYER's prompted SAVE/RESTORE verb is unexpected here.
            StepResult::SaveRequest | StepResult::RestoreRequest => {
                let req = m.pending_saveload_request().unwrap_or_default();
                assert!(
                    !req.by_prompt,
                    "{name}: unexpected player-prompted @save/@restore (accel={accel})"
                );
                saveloads.push(format!(
                    "{} {}",
                    if req.restore { "restore" } else { "save" },
                    req.name
                ));
                if req.restore {
                    m.complete_restore_failure();
                } else {
                    m.complete_save(false);
                }
            }
            StepResult::NeedFilename { .. } => {
                panic!("{name}: unexpected filename prompt (accel={accel})")
            }
        }
    }

    let text = m
        .backend_mut()
        .as_any_mut()
        .downcast_mut::<TestBackend>()
        .unwrap()
        .all_text();
    Run { text, ops: m.insn_count(), saveloads }
}

/// Drive to the first prompt only — the original speed/equivalence shape.
fn run_to_first_prompt(image: Vec<u8>, accel: bool) -> (String, u64, Vec<String>) {
    let r = play_named("story", image, accel, &[]);
    (r.text, r.ops, r.saveloads)
}

/// Load and run one on/off equivalence + speed comparison for the Blorb at
/// `path`, returning `(ops_on, ops_off)` after asserting transcript equality
/// and a material opcode reduction. `opening` is a substring the story is known
/// to print before its first prompt — the non-vacuity guard, without which two
/// runs that both stopped early would compare equal and pass.
fn check_equivalence_and_speed(name: &str, opening: &str) -> (u64, u64) {
    let path = stories_dir().join(name);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("local story (gitignored) missing at {}: {e}", path.display()));
    let image = extract_glulx(bytes);

    let (out_on, ops_on, saves_on) = run_to_first_prompt(image.clone(), true);
    let (out_off, ops_off, saves_off) = run_to_first_prompt(image, false);

    assert_eq!(out_on, out_off, "acceleration changed the transcript to first prompt for {name}");
    assert_eq!(
        saves_on, saves_off,
        "acceleration changed the game-managed @save/@restore sequence for {name}"
    );
    assert!(
        out_on.contains(opening),
        "{name} reached a prompt without printing {opening:?} — the transcript comparison is \
         vacuous; got {out_on:?}"
    );
    assert!(
        ops_on * 3 < ops_off,
        "accel not materially faster for {name}: on={ops_on} off={ops_off}"
    );

    (ops_on, ops_off)
}

/// The headline proof: CounterfeitMonkey (a large, real-world Inform 7 game)
/// produces an identical transcript to first prompt with acceleration on vs.
/// off, and acceleration cuts the dispatched-opcode count by more than 3x
/// (Task 0's baseline measured ~88.8% of interpreted opcodes inside
/// accel-candidate functions).
#[test]
#[ignore = "needs local gitignored stories/CounterfeitMonkey-11.gblorb; run with `cargo test -p gvm --test accel_story_equivalence -- --ignored`"]
fn counterfeit_monkey_accel_matches_interpreted_and_is_faster() {
    let (ops_on, ops_off) = check_equivalence_and_speed("CounterfeitMonkey-11.gblorb", CM_OPENING);
    eprintln!("CounterfeitMonkey-11: ops_on={ops_on} ops_off={ops_off} ratio={:.2}x", ops_off as f64 / ops_on as f64);
}

/// A smaller, faster secondary confirmation on another Inform Glulx title
/// present under `stories/` — the same on/off transcript equivalence check,
/// without the speed margin assertion (a tiny story may not do enough
/// accel-eligible work to clear the 3x bar, but transcript identity must
/// still hold).
#[test]
#[ignore = "needs local gitignored stories/TAKE.gblorb; run with `cargo test -p gvm --test accel_story_equivalence -- --ignored`"]
fn take_accel_matches_interpreted() {
    let path = stories_dir().join("TAKE.gblorb");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("local story (gitignored) missing at {}: {e}", path.display()));
    let image = extract_glulx(bytes);

    let (out_on, ops_on, saves_on) = run_to_first_prompt(image.clone(), true);
    let (out_off, ops_off, saves_off) = run_to_first_prompt(image, false);

    assert_eq!(out_on, out_off, "acceleration changed the transcript to first prompt for TAKE.gblorb");
    assert_eq!(
        saves_on, saves_off,
        "acceleration changed the game-managed @save/@restore sequence for TAKE.gblorb"
    );
    assert!(
        out_on.contains(TAKE_OPENING),
        "TAKE.gblorb reached a prompt without printing {TAKE_OPENING:?} — the transcript \
         comparison is vacuous; got {out_on:?}"
    );
    eprintln!("TAKE: ops_on={ops_on} ops_off={ops_off}");
}

// ─── SQ-1209: the fingerprinted (never-declared) stories ──────────────────────

/// Every Glulx story under `stories/`, smallest first so a failure reports on
/// the cheap ones before the ten-megabyte ones. Returns an empty list when
/// `stories/` is absent (the CI path). Classification reads the file header and
/// walks Blorb chunk headers by seeking, so the several hundred megabytes under
/// `stories/` are not pulled through memory to answer "is this Glulx?".
fn glulx_stories() -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(stories_dir()) else { return Vec::new() };
    let mut out: Vec<(u64, PathBuf)> = Vec::new();
    for entry in dir.flatten() {
        let p = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_file() && is_glulx_file(&p) {
            out.push((meta.len(), p));
        }
    }
    out.sort();
    out.into_iter().map(|(_, p)| p).collect()
}

/// Header-only Glulx test: a bare `.ulx`, or a Blorb with a `GLUL` chunk.
fn is_glulx_file(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut head = [0u8; 12];
    if f.read_exact(&mut head).is_err() {
        return false;
    }
    if &head[0..4] == b"Glul" {
        return true;
    }
    if &head[0..4] != b"FORM" || &head[8..12] != b"IFRS" {
        return false;
    }
    let form_len = u32::from_be_bytes([head[4], head[5], head[6], head[7]]) as u64;
    let mut pos = 12u64;
    let mut hdr = [0u8; 8];
    while pos < form_len + 8 {
        if f.seek(SeekFrom::Start(pos)).is_err() || f.read_exact(&mut hdr).is_err() {
            return false;
        }
        if &hdr[0..4] == b"GLUL" {
            return true;
        }
        let len = u32::from_be_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as u64;
        pos += 8 + len + (len & 1);
    }
    false
}

/// The command battery. Blank lines and single keys clear intro cards; the
/// verbs after them are ones every Inform library answers, so a story that
/// refuses one still produces the *same* refusal on and off.
/// Short on purpose: with acceleration OFF these turns are interpreted opcode
/// by opcode, and King of Shreds' `inventory` alone is 43 million of them (the
/// SQ-1205 case). Two turns is enough to compare a room description and an
/// inventory listing — both heavy users of the property veneer — without making
/// the default `-p gvm` tier minutes long.
const BATTERY: &[&str] = &["look", "inventory"];

/// Boot `image` far enough to see whether the story announces its own
/// accelerated functions. `@accelfunc` runs in Inform 7's startup rules, long
/// before the first prompt (2 783 opcodes into BlueLacuna), so a short budget
/// is plenty — and the probe stops the moment the story asks for input.
fn declares_own_accel(image: &[u8]) -> bool {
    let mem = Memory::new(image.to_vec()).expect("valid Glulx image");
    let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
    for _ in 0..2_000_000 {
        if m.declares_own_accel() {
            return true;
        }
        match m.step() {
            StepResult::Continue => {}
            StepResult::NeedEvent { timer_ms: Some(_), .. } => m.deliver_timer(),
            StepResult::SaveRequest | StepResult::RestoreRequest => {
                let req = m.pending_saveload_request().unwrap_or_default();
                if req.restore {
                    m.complete_restore_failure();
                } else {
                    m.complete_save(false);
                }
            }
            _ => break,
        }
    }
    m.declares_own_accel()
}

/// Fingerprinted acceleration changes nothing a player can see. For every
/// Glulx story present that never declares its own accelerated functions —
/// the Inform 6 and pre-6E59 Inform 7 builds SQ-1209 targets — drive the
/// command battery twice, once with acceleration on and once off, and require
/// the transcripts and the game-managed save/restore sequence to be identical.
#[test]
fn fingerprinted_stories_are_transcript_identical_on_and_off() {
    let stories = glulx_stories();
    if stories.is_empty() {
        eprintln!("stories/ absent — skipping (this is the CI path)");
        return;
    }
    let mut ran: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for path in stories {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let image = extract_glulx(std::fs::read(&path).expect("readable story"));
        if declares_own_accel(&image) {
            skipped.push(name);
            continue;
        }
        let on = play_named(&name, image.clone(), true, BATTERY);
        let off = play_named(&name, image, false, BATTERY);
        assert_eq!(
            on.text, off.text,
            "fingerprinted acceleration changed the transcript for {name}"
        );
        assert_eq!(
            on.saveloads, off.saveloads,
            "fingerprinted acceleration changed the @save/@restore sequence for {name}"
        );
        assert!(
            on.text.len() > 200,
            "{name} produced only {} characters — the comparison is vacuous",
            on.text.len()
        );
        ran.push(format!(
            "{name}: {} chars, ops on={} off={} ({:.2}x)",
            on.text.len(),
            on.ops,
            off.ops,
            off.ops as f64 / on.ops.max(1) as f64
        ));
    }
    eprintln!("fingerprinted stories exercised ({}):", ran.len());
    for r in &ran {
        eprintln!("  {r}");
    }
    eprintln!("stories that declare their own accelfuncs (not this sweep's subject): {}", skipped.len());
}

/// The fingerprint must agree with the ground truth wherever there is one: for
/// every story that declares its own `@accelfunc`/`@accelparam`, the addresses
/// and the nine parameters derived from the bytecode alone must be exactly what
/// the game goes on to announce. This is the falsification the templates and
/// the parameter slots need, and it runs against every such story present.
#[test]
fn fingerprint_agrees_with_every_story_that_declares_its_own() {
    let stories = glulx_stories();
    if stories.is_empty() {
        eprintln!("stories/ absent — skipping (this is the CI path)");
        return;
    }
    let mut checked = 0usize;
    let mut never_declared: Vec<String> = Vec::new();
    for path in &stories {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let image = extract_glulx(std::fs::read(path).expect("readable story"));
        let Some((derived_funcs, derived_params)) = fingerprint_of(&image) else {
            never_declared.push(format!("{name} (no template match)"));
            continue;
        };
        // Boot far enough for the story to register, if it ever does.
        let mem = Memory::new(image).expect("valid Glulx image");
        let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
        let mut declared = false;
        for _ in 0..2_000_000 {
            match m.step() {
                StepResult::Continue => {}
                StepResult::NeedEvent { timer_ms: Some(_), .. } => m.deliver_timer(),
                StepResult::SaveRequest | StepResult::RestoreRequest => {
                    let req = m.pending_saveload_request().unwrap_or_default();
                    if req.restore {
                        m.complete_restore_failure();
                    } else {
                        m.complete_save(false);
                    }
                }
                _ => break,
            }
            if m.declares_own_accel() {
                declared = true;
                break;
            }
        }
        if !declared {
            never_declared.push(format!("{name} (never declares)"));
            continue;
        }
        let mut announced: Vec<(u32, u32)> =
            m.accel_funcs().iter().map(|(a, n)| (*n, *a)).collect();
        announced.sort();
        assert_eq!(
            derived_funcs, announced,
            "{name}: fingerprinted addresses differ from the ones the story registered"
        );
        for (i, want) in derived_params.iter().enumerate() {
            assert_eq!(
                m.accel_param(i as u32),
                Some(*want),
                "{name}: accelparam {i} derived as {want:#x}, story declared {:?}",
                m.accel_param(i as u32)
            );
        }
        checked += 1;
    }
    eprintln!("fingerprint agreed with {checked} stories that declare their own accelfuncs");
    eprintln!("stories with no ground truth to compare against: {never_declared:?}");
    assert!(
        checked > 0 || !Path::new(&stories_dir()).exists(),
        "no story in stories/ declared its own accelfuncs — this check was vacuous"
    );
}

/// `(accel number, address)` pairs in accel-number order, plus the nine
/// parameters — what one story's fingerprint amounts to.
type Derived = (Vec<(u32, u32)>, Vec<u32>);

/// The addresses and parameters fingerprinting derives for `image`, before the
/// story has run a single opcode.
fn fingerprint_of(image: &[u8]) -> Option<Derived> {
    let mem = Memory::new(image.to_vec()).expect("valid Glulx image");
    let m = Machine::with_glk(mem, Box::new(TestBackend::new()));
    if !m.veneer_accel().installed() {
        return None;
    }
    let mut funcs: Vec<(u32, u32)> = m.accel_funcs().iter().map(|(a, n)| (*n, *a)).collect();
    funcs.sort();
    Some((funcs, m.veneer_accel().params.to_vec()))
}
