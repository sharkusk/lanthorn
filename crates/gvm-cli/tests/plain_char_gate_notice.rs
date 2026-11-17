//! Piped char input at a story-authored `[ MORE ]` gate is never silent.
//!
//! The story is synthetic but reproduces Pythos Mask's MORE routine (release 4
//! / serial 020223): it prints `[ MORE ]`, requests a char event, and ignores
//! every key except a space — re-requesting with no output at all. A piped
//! controller that answers with an empty line (= Glk Return) used to get
//! 0 bytes back and read the session as hung.
//!
//! Now the drive loop notices the silent re-request and emits a host line
//! naming the ignored key, so the controller knows to send another key.
//! A controller that sends a space straight away sees no notice (the gate
//! passes first try) and the transcript stays clean.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

// ── a tiny Glulx encoder (same shapes as gvm-cli's in-crate tests) ───────────

#[derive(Clone, Copy)]
enum E {
    Imm(u32),
    LocLoad(u8),
    LocStore(u8),
    MemLoad(u32),
    Push,
    Discard,
}
fn emode(e: E) -> u8 {
    match e {
        E::Imm(_) => 3,
        E::LocLoad(_) | E::LocStore(_) => 9,
        E::MemLoad(_) => 7,
        E::Push => 8,
        E::Discard => 0,
    }
}
fn edata(e: E) -> Vec<u8> {
    match e {
        E::Imm(v) | E::MemLoad(v) => v.to_be_bytes().to_vec(),
        E::LocLoad(o) | E::LocStore(o) => vec![o],
        E::Push | E::Discard => vec![],
    }
}
fn enc(op: u32, args: &[E]) -> Vec<u8> {
    let mut out = Vec::new();
    if op <= 0x7f {
        out.push(op as u8);
    } else {
        out.extend_from_slice(&((op | 0x8000) as u16).to_be_bytes());
    }
    let mut modes = vec![0u8; args.len().div_ceil(2)];
    for (i, &a) in args.iter().enumerate() {
        let m = emode(a);
        if i % 2 == 0 {
            modes[i / 2] |= m;
        } else {
            modes[i / 2] |= m << 4;
        }
    }
    out.extend_from_slice(&modes);
    for &a in args {
        out.extend(edata(a));
    }
    out
}

fn print_str(b: &mut Vec<u8>, s: &str) {
    for ch in s.bytes() {
        b.extend(enc(0x70, &[E::Imm(ch as u32)])); // streamchar
    }
}

/// The MORE-gate program. Opens a TextBuffer (local0), prints `[ MORE ]`,
/// then loops: request_char_event, glk_select into the event struct at 0x200
/// (the start of RAM), and unless val1 (0x208) is a space (32) re-requests
/// with NO output — the exact shape of Pythos Mask's MORE routine, which
/// ignores Return. On a space it prints `done` and quits.
fn story_bytes() -> Vec<u8> {
    use E::*;
    let mut body = enc(0x149, &[Imm(2), Imm(0)]); // setiosys glk
    for v in [Imm(0), Imm(3), Imm(0), Imm(0), Imm(0)] {
        body.extend(enc(0x40, &[v, Push])); // rock, wintype=3, size, method, split
    }
    body.extend(enc(0x130, &[Imm(0x23), Imm(5), LocStore(0)])); // window_open → local0
    body.extend(enc(0x40, &[LocLoad(0), Push]));
    body.extend(enc(0x130, &[Imm(0x2f), Imm(1), Discard])); // set_window
    print_str(&mut body, "[ MORE ]");

    // The gate loop: request a char, select, and jump back unless it was a space.
    let mut gate = Vec::new();
    gate.extend(enc(0x40, &[LocLoad(0), Push]));
    gate.extend(enc(0x130, &[Imm(0xd2), Imm(1), Discard])); // request_char_event
    gate.extend(enc(0x40, &[Imm(0x200), Push]));
    gate.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select(@0x200)
    // jne val1, 32, back-to-top. The jne is 1 opcode + 2 mode bytes + 12 data
    // bytes = 15; branch target = (addr after instruction) + offset - 2.
    let jne_len = 15i32;
    let offset = -(gate.len() as i32) - jne_len + 2;
    let jne = enc(0x25, &[MemLoad(0x208), Imm(32), Imm(offset as u32)]);
    assert_eq!(jne.len() as i32, jne_len, "jne encoding length drifted");
    gate.extend(jne);
    body.extend(gate);

    print_str(&mut body, "done\n");
    body.extend(enc(0x120, &[])); // quit

    // Wrap as a one-local start function in a minimal image. RAM begins at
    // 0x200 and holds the event struct glk_select fills (writing it into ROM
    // would fault).
    let mut func = vec![0xC1u8, 0x04, 0x01, 0x00, 0x00];
    func.extend(body);
    let (ramstart, endmem) = (0x200u32, 0x300u32);
    let mut img = vec![0u8; ramstart as usize];
    img[0..4].copy_from_slice(b"Glul");
    img[0x04..0x08].copy_from_slice(&0x0003_0102u32.to_be_bytes());
    img[0x08..0x0C].copy_from_slice(&ramstart.to_be_bytes());
    img[0x0C..0x10].copy_from_slice(&ramstart.to_be_bytes());
    img[0x10..0x14].copy_from_slice(&endmem.to_be_bytes());
    img[0x14..0x18].copy_from_slice(&0x1000u32.to_be_bytes());
    img[0x18..0x1C].copy_from_slice(&0x24u32.to_be_bytes());
    assert!(0x24 + func.len() <= ramstart as usize, "program overruns ROM");
    img[0x24..0x24 + func.len()].copy_from_slice(&func);
    img
}

fn story_file() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir()
        .join(format!("lanthorn-gvm-moregate-{}-{n}.ulx", std::process::id()));
    std::fs::write(&path, story_bytes()).expect("write story");
    path
}

fn run(script: &[u8]) -> Output {
    let path = story_file();
    let mut child = Command::new(env!("CARGO_BIN_EXE_gvm-cli"))
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Pin a real TERM so plain mode is not also selected by TERM=dumb;
        // piped stdin alone must be enough to get the notice.
        .env("TERM", "xterm")
        .env("NO_COLOR", "1")
        .spawn()
        .expect("gvm-cli spawns");
    child.stdin.take().unwrap().write_all(script).unwrap();
    let out = child.wait_with_output().expect("gvm-cli runs");
    let _ = std::fs::remove_file(&path);
    out
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A piped empty line is Glk Return; the gate ignores it and re-waits with no
/// output. The host must say so instead of leaving the pipe silent.
#[test]
fn ignored_return_at_a_more_gate_is_announced() {
    let out = run(b"\n \n");
    assert!(
        out.status.success(),
        "exit {:?}\nstderr: {}\nstdout: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
        stdout_of(&out)
    );
    let text = stdout_of(&out);
    let notices = text.matches("ignored Return").count();
    assert_eq!(
        notices, 1,
        "one Return was ignored, so exactly one notice must be emitted:\n{text}"
    );
    let notice_at = text.find("ignored Return").unwrap();
    let done_at = text.find("done").expect("the space passed the gate");
    assert!(
        notice_at < done_at,
        "the notice breaks the silence BEFORE the gate passes:\n{text}"
    );
    assert!(text.contains("[ MORE ]"), "the gate's own prompt is on stdout:\n{text}");
}

/// A key the gate accepts produces no notice: the transcript of a controller
/// that already sends a space stays byte-identical to what the game wrote.
#[test]
fn an_accepted_key_produces_no_notice() {
    let out = run(b" \n");
    assert!(
        out.status.success(),
        "exit {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let text = stdout_of(&out);
    assert!(!text.contains("ignored"), "no key was ignored, so no notice:\n{text}");
    assert!(text.contains("done"), "the space passed the gate:\n{text}");
}
