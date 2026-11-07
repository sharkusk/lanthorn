//! Piped and `--plain` `@read_char` re-emits an unchanged upper window.
//!
//! The story is synthetic. It paints a two-item menu in the upper window,
//! prints `press space` in the lower window, then:
//!
//! 1. `@read_char` (the menu is emitted)
//! 2. `@read_char` again, with the upper window untouched (the menu is
//!    emitted a second time)
//! 3. prints `continued`, then `@read` (line input does not emit the menu
//!    a third time)
//! 4. prints `after line` and quits
//!
//! Upper-window text is not copied to the transcript stream, so every
//! occurrence of the menu on stdout is a `ScreenView` frame.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

fn var_small(code: &mut Vec<u8>, opcode: u8, op: u8) {
    code.push(0xE0 | opcode);
    code.push(0x7F); // one small constant, the rest omitted
    code.push(op);
}

fn print_str(code: &mut Vec<u8>, s: &str) {
    for b in s.bytes() {
        var_small(code, 0x05, b); // print_char
    }
}

fn print_line(code: &mut Vec<u8>, s: &str) {
    print_str(code, s);
    code.push(0xBB); // new_line
}

/// Minimal v5 image. Header layout matches the other hand-built stories in
/// this crate: program at `0x40`, dynamic memory through `0x400`.
fn story_bytes() -> Vec<u8> {
    let mut code = Vec::new();
    var_small(&mut code, 0x0A, 2); // split_window 2
    var_small(&mut code, 0x0B, 1); // set_window 1
                                   // Two item lines at the same column, marker on the first. That is a menu
                                   // to `cli_host::MenuBlock`, which is the `--plain` path.
    print_line(&mut code, "> Alpha");
    print_str(&mut code, "  Beta");
    var_small(&mut code, 0x0B, 0); // set_window 0
    print_line(&mut code, "press space");
    // read_char device 1, store G0. Twice, with nothing drawn between them.
    var_small(&mut code, 0x16, 1);
    code.push(0x10);
    var_small(&mut code, 0x16, 1);
    code.push(0x10);
    print_line(&mut code, "continued");
    // v5 @read text=0x0200 parse=0x0280, store G0.
    code.push(0xE4);
    code.push(0x0F); // two large constants
    code.extend_from_slice(&[0x02, 0x00, 0x02, 0x80]);
    code.push(0x10);
    print_line(&mut code, "after line");
    code.push(0xBA); // quit

    assert!(
        code.len() < 0x140,
        "program overlaps the dictionary at 0x0180 ({} bytes)",
        code.len()
    );

    let mut buf = vec![0u8; 0x0800];
    buf[0x00] = 5;
    buf[0x04] = 0x04; // high_mem_base
    buf[0x05] = 0x00;
    buf[0x06] = 0x00; // initial_pc
    buf[0x07] = 0x40;
    buf[0x08] = 0x01; // dictionary
    buf[0x09] = 0x80;
    buf[0x0A] = 0x01; // object table (unused)
    buf[0x0B] = 0xC0;
    buf[0x0C] = 0x03; // globals
    buf[0x0D] = 0x00;
    buf[0x0E] = 0x04; // static memory
    buf[0x0F] = 0x00;
    buf[0x18] = 0x03; // abbreviations (unused)
    buf[0x19] = 0x60;
    buf[0x40..0x40 + code.len()].copy_from_slice(&code);

    // Empty dictionary: no separators, 6-byte entries, no entries.
    buf[0x0180] = 0;
    buf[0x0181] = 6;
    buf[0x0182] = 0;
    buf[0x0183] = 0;
    // v5 text buffer: byte 0 is the max length.
    buf[0x0200] = 40;
    buf[0x0280] = 8; // parse buffer: max words
    buf
}

fn story_file() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("lanthorn-zvm-readchar-{n}.z5"));
    std::fs::write(&path, story_bytes()).expect("write story");
    path
}

/// ` \n` answers the first `@read_char`, `x\n` the second, `look\n` the
/// `@read`. Each char read consumes one cooked line.
const SCRIPT: &[u8] = b" \nx\nlook\n";

fn run(extra: &[&str]) -> Output {
    let path = story_file();
    let mut child = Command::new(env!("CARGO_BIN_EXE_zvm-cli"))
        .args(extra)
        .args(["--sound", "off", "--aux", "off", "--pager", "off"])
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // `TERM=dumb` is itself a plain-mode request. Pin a real term so the
        // default-mode case is not rewritten.
        .env("TERM", "xterm")
        .env("NO_COLOR", "1")
        .spawn()
        .expect("zvm-cli spawns");
    child.stdin.take().unwrap().write_all(SCRIPT).unwrap();
    let out = child.wait_with_output().expect("zvm-cli runs");
    let _ = std::fs::remove_file(&path);
    out
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn assert_relisted(out: &Output, mode: &str, menu_hint: bool) {
    assert!(
        out.status.success(),
        "{mode}: exit {:?}\nstderr: {}\nstdout: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
        stdout_of(out)
    );
    let text = stdout_of(out);
    let menus = text.matches("Alpha").count();
    assert_eq!(
        menus, 2,
        "{mode}: the upper window must be emitted at each @read_char and not \
         again at the following @read (saw {menus}).\n{text}"
    );
    let hints = text.matches("[menu").count();
    if menu_hint {
        assert_eq!(
            hints, 2,
            "{mode}: --plain numbers the menu on each re-list (saw {hints} hints).\n{text}"
        );
    } else {
        assert_eq!(
            hints, 0,
            "{mode}: menu recognition is off, so the raw block has no hint.\n{text}"
        );
    }
    assert!(
        text.contains("press space"),
        "{mode}: lower window missing:\n{text}"
    );
    assert!(
        text.contains("continued"),
        "{mode}: second key was not accepted:\n{text}"
    );
    assert!(
        text.contains("after line"),
        "{mode}: line input did not complete:\n{text}"
    );
    let after = text.split("continued").nth(1).unwrap_or("");
    assert!(
        !after.contains("Alpha"),
        "{mode}: line input re-emitted the unchanged menu:\n{after}"
    );
}

#[test]
fn plain_read_char_relists_unchanged_upper_window() {
    assert_relisted(&run(&["--plain"]), "--plain", true);
}

#[test]
fn piped_read_char_relists_unchanged_upper_window() {
    assert_relisted(&run(&[]), "piped", false);
}
