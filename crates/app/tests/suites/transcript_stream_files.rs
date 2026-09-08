//! SQ-1420: the Z-machine's output streams 2 and 4 and its input stream 1,
//! landing on lanthorn's per-game files.
//!
//! `<game_dir>/transcript.txt` is the STORY's transcript — ZMSD §7.1.1, "the
//! game transcript, usually printed to a printer or a file" — plain text,
//! exactly what the story printed. It is not `transcript.json`, the archive's
//! record of lanthorn's own scrollback; the two carry different things and only
//! one of them is the Z-machine's.
//!
//! `<game_dir>/commands.txt` is output stream 4 (§7.1.2: "a script file of the
//! player's whole commands and of individual keypresses as read by `read_char`")
//! and is the same file input stream 1 reads back, because §10.2.1 requires one
//! format for both.
//!
//! Zork I (v3) is gitignored, so every case here skips vacuously when it is
//! absent (see CLAUDE.md).

use std::path::PathBuf;

use app::engine::Engine;
use app::session::{GameSession, StreamFiles};

fn zork1() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/zork1-r88-s840726.z3");
    match std::fs::read(&p) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", p.display());
            None
        }
    }
}

/// Boot Zork I and point its stream files at a fresh scratch directory —
/// `app::scratch_dir` because it is unique per CALL, which is what keeps two
/// cases in one process off each other's files (CLAUDE.md, SQ-1131/SQ-1163).
fn booted(tag: &str) -> Option<(GameSession, PathBuf)> {
    let bytes = zork1()?;
    let mut session = GameSession::new(bytes, false, false, None).expect("Zork I boots");
    let dir = app::scratch_dir(tag);
    session.set_stream_files(&dir);
    let _ = session.take_transcript(); // drop the banner
    Some((session, dir))
}

#[test]
fn set_transcript_on_writes_the_next_two_turns_to_transcript_txt() {
    let Some((mut session, dir)) = booted("sq1420-transcript") else { return };
    let path = StreamFiles::transcript_path(&dir);
    assert!(!path.exists(), "naming the directory must not create the file");

    // What `/set-transcript on` does: ZMSD §7.4's other route into stream 2.
    let named = session.set_transcript(true);
    assert_eq!(named.as_deref(), Some(path.as_path()), "the notice can name the file");
    assert!(session.transcript_on());

    let a = session.submit("look");
    let b = session.submit("north");
    assert!(a.fault.is_none() && b.fault.is_none(), "Zork I faulted mid-turn");

    let written = std::fs::read_to_string(&path).expect("transcript.txt exists once text flows");
    assert!(
        written.contains("West of House"),
        "the room the first turn described is missing: {written:?}"
    );
    assert!(
        written.contains("North of House"),
        "the room the second turn described is missing: {written:?}"
    );
    // §7.1.1.1: "In Versions 1 to 5, the player's input to the read opcode
    // should be echoed to output streams 1 and 2 … so that text typed in appears
    // in any transcript."
    assert!(written.contains("look"), "the player's own commands belong in the transcript");
    assert!(written.contains("north"));

    // …and stopping it stops it. The file keeps what it has (it is APPENDED to,
    // never truncated — the transcript is the game's, not the run's).
    session.set_transcript(false);
    assert!(!session.transcript_on());
    let len = std::fs::metadata(&path).unwrap().len();
    session.submit("south");
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        len,
        "a stopped transcript takes nothing further"
    );
}

#[test]
fn the_command_record_round_trips_through_input_stream_1() {
    // Stream 4 out, input stream 1 back in, one file, one format (ZMSD §10.2.1).
    let Some((mut session, dir)) = booted("sq1420-record") else { return };
    let path = StreamFiles::commands_path(&dir);

    session.machine.set_command_record(true);
    session.submit("look");
    session.submit("north");
    let recorded = std::fs::read_to_string(&path).expect("commands.txt exists");
    assert_eq!(recorded, "look\nnorth\n", "one record per line, no escaping needed here");

    // A second session replays that same file. ZMSD §10.2.2 lets the
    // interpreter select the stream itself — "An interpreter is free to change
    // the input stream whenever it likes (e.g. at the player's request) or,
    // indeed, to run the entire game under input stream 1 (for testing
    // purposes)" — which is what `Machine::set_input_stream` is for.
    let Some((mut replayed, replay_dir)) = booted("sq1420-replay") else { return };
    std::fs::write(StreamFiles::commands_path(&replay_dir), &recorded).expect("seed commands.txt");
    replayed.machine.set_input_stream(1);

    // One turn of the player's own — the reads AFTER it are answered from the
    // file without ever suspending, so both recorded turns arrive in one go.
    let out = replayed.submit("");
    assert!(out.fault.is_none(), "replay faulted");
    assert!(
        out.transcript.contains("North of House"),
        "input stream 1 never fed the recorded 'north': {:?}",
        out.transcript
    );
    // §10.2's end of file: the machine is back on the keyboard.
    assert_eq!(
        replayed.machine.streams.input_stream, 0,
        "an exhausted command file must revert to the keyboard"
    );

    // And a replay is not re-recorded into the file it came from (Frotz's
    // `ostream_record && !istream_replay`, `stream.c`).
    assert!(
        !StreamFiles::commands_path(&replay_dir).metadata().is_ok_and(|m| m.len() as usize > recorded.len()),
        "replaying grew the very file being replayed"
    );
}
