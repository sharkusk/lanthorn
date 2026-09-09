//! SQ-1420 real-game smoke: a story's own SCRIPT command must start a transcript.
//!
//! The Inform 6 library's `ScriptOnSub` issues `@output_stream 2` and then reads
//! `Flags 2` bit 0 straight back:
//!
//! ```inform
//! @output_stream 2;
//! if ((0-->8) & 1 == 0) "Attempt to begin transcript failed.";
//! ```
//!
//! ZMSD §7.4 is what makes that a fair question of the interpreter: "In Versions
//! 3 and later, all four output streams can be selected or deselected using the
//! `output_stream` opcode. In addition, stream 2 can be selected or deselected by
//! setting or clearing bit 0 of 'Flags 2'. Whichever method is used, the
//! interpreter must ensure that this flag holds the current status of stream 2.
//! ('A Mind Forever Voyaging' requires this.)" §11.1.2 says it again from the
//! header's side.
//!
//! zvm used to record the selection in a bool and leave the flag alone — and
//! CLEAR it at every `read` besides — so **nine of thirteen** corpus stories
//! driven to a SCRIPT command answered "Attempt to begin transcript failed."
//! (the SQ-1014 audit's gap 3). Those nine are the table below.
//!
//! AMFV is the other half of the same rule and the reason §7.4 names it: it turns
//! transcription on by POKING the bit rather than by issuing the opcode, so it
//! exercises the read-time sync instead of the opcode. Both halves are needed and
//! neither covers the other — reverting the opcode's flag write leaves AMFV
//! working and fails all nine; reverting the sync does the reverse.
//!
//! Every story here is gitignored (see CLAUDE.md), so each case skips cleanly
//! when its fixture is absent.

use std::path::PathBuf;

use zvm::cpu::exec::{Machine, StepResult};
use zvm::io::BufferOutput;
use zvm::memory::Memory;
use zvm::text::input::ZsciiInput;

/// The nine stories the SQ-1014 audit measured as failing, plus AMFV.
const STORIES: &[&str] = &[
    "gostak.z5",
    "Tangle.z5",
    "anchor.z8",
    "suvehnux.z5",
    "vespers.z8",
    "MakeItGood.z8",
    "Savoir-Faire.zblorb",
    "Wallpaper.zblorb",
    "WeirdCityInterloper.zblorb",
];

fn story(name: &str) -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    match std::fs::read(&p) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", p.display());
            None
        }
    }
}

/// Boot `name` the way a host does, drive past its title cards with blank input,
/// and park at a line prompt. `None` when the fixture is absent.
fn booted(name: &str) -> Option<Machine> {
    let bytes = story(name)?;
    // A .zblorb carries the story inside an IFRS wrapper; `Memory::new` wants the
    // bare code, so take the ZCOD chunk when one is there.
    let bytes = extract_zcode(bytes);
    let mut m = Machine::with_output(Memory::new(bytes).ok()?, Box::new(BufferOutput::new()));
    m.init_caps();
    m.set_screen_dims(24, 80);
    // Three blank answers clears the "press SPACE" cards and any restore
    // question these games open with; the fourth prompt is the game's.
    for _ in 0..4 {
        match run_to_input(&mut m) {
            StepResult::NeedLine { .. } => m.supply_line("", 13),
            StepResult::NeedChar => m.supply_char(ZsciiInput::NEWLINE),
            other => panic!("{name} stopped at {other:?} before its first prompt"),
        }
    }
    run_to_input(&mut m);
    Some(m)
}

/// Pull the ZCOD chunk out of a Blorb, or hand back a bare story unchanged.
/// Deliberately hand-rolled rather than taken from `lanthorn-blorb`: `zvm` has
/// zero dependencies, and its tests keep that promise too.
fn extract_zcode(bytes: Vec<u8>) -> Vec<u8> {
    if bytes.len() < 12 || &bytes[0..4] != b"FORM" || &bytes[8..12] != b"IFRS" {
        return bytes;
    }
    let mut at = 12usize;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_be_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        let body = at + 8;
        if id == b"ZCOD" && body + len <= bytes.len() {
            return bytes[body..body + len].to_vec();
        }
        at = body + len + (len & 1); // chunks are word-aligned
    }
    bytes
}

fn run_to_input(m: &mut Machine) -> StepResult {
    for _ in 0..200_000_000u64 {
        match m.step() {
            r @ (StepResult::NeedChar | StepResult::NeedLine { .. } | StepResult::Quit) => {
                return r
            }
            StepResult::Fault => panic!("story faulted: {:?}", m.take_fault_trace()),
            _ => {}
        }
    }
    panic!("story never asked for input");
}

/// Everything the sink has taken on stream 1 since the last drain.
fn screen(m: &mut Machine) -> String {
    let sink = m
        .output_mut()
        .as_any_mut()
        .downcast_mut::<BufferOutput>()
        .expect("BufferOutput sink");
    std::mem::take(&mut sink.buf)
}

fn transcript(m: &Machine) -> &str {
    &m.buffer_output().expect("BufferOutput sink").transcript
}

#[test]
fn typing_script_starts_a_transcript_in_every_story_that_used_to_refuse() {
    let mut seen = 0usize;
    for name in STORIES {
        let Some(mut m) = booted(name) else { continue };
        seen += 1;

        let _ = screen(&mut m); // drop the title card
        m.supply_line("script", 13);
        run_to_input(&mut m);
        let reply = screen(&mut m);
        assert!(
            !reply.to_lowercase().contains("attempt to begin transcript failed"),
            "{name}: SCRIPT still refuses — {reply:?}"
        );
        assert!(m.transcript_on(), "{name}: stream 2 is not actually selected");
        assert_eq!(
            m.mem.read_word(0x10) & 1,
            1,
            "{name}: ZMSD §7.4 — the game must be able to read the bit back as set"
        );

        // …and the transcript sink receives the next room description.
        let before = transcript(&m).len();
        m.supply_line("look", 13);
        run_to_input(&mut m);
        assert!(
            transcript(&m).len() > before + 20,
            "{name}: nothing reached the transcript after SCRIPT"
        );
    }
    if seen == 0 {
        eprintln!("SKIP: none of the SQ-1420 corpus stories are present");
    }
}

#[test]
fn amfv_pokes_flags2_directly_and_the_interpreter_must_follow() {
    // ZMSD §7.4: "('A Mind Forever Voyaging' requires this.)" — r77 turns
    // transcription on by SETTING the bit, never by issuing `output_stream 2`,
    // and §7.1.1.2 warns that it "is turned off and on again several times in
    // quick succession".
    let Some(mut m) = booted("amfv-r77-s850814.z4") else { return };
    let _ = screen(&mut m);
    m.supply_line("script", 13);
    run_to_input(&mut m);
    assert!(m.transcript_on(), "AMFV's SCRIPT verb did not reach stream 2");
    let before = transcript(&m).len();
    m.supply_line("look", 13);
    run_to_input(&mut m);
    assert!(transcript(&m).len() > before + 20, "nothing reached AMFV's transcript");
    // §7.1.1.1: a Version 4 story echoes the player's line into the transcript.
    assert!(
        transcript(&m).contains("look"),
        "the player's own command belongs in the transcript"
    );
}
