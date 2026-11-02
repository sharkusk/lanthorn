//! A hand-built Version 6 story: packed `main`, print, `@read`, `@quit`.
//!
//! This is the committed v6 fixture. Freely licensed IF Archive titles
//! (`advent.z6`, `sunburst.z6`) are fetched, not vendored; see
//! `scripts/fixtures.manifest`.

use zvm::cpu::exec::{BootConfig, Machine, StepResult};
use zvm::io::BufferOutput;
use zvm::memory::Memory;

/// Version 6 story whose `main` prints `READY>`, reads a line, prints `OK`, quits.
fn v6_ready_story() -> Vec<u8> {
    let mut b = vec![0u8; 0x800];
    b[0x00] = 6;
    {
        let mut word = |o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_be_bytes());
        word(0x04, 0x0500); // high memory
        word(0x06, 0x0010); // packed main -> byte 0x40
        word(0x08, 0x0480); // dictionary
        word(0x0A, 0x0100); // objects
        word(0x0C, 0x0200); // globals
        word(0x0E, 0x0500); // static memory
        word(0x18, 0x0180); // abbreviations
        word(0x1A, 0x0100); // declared length (0x800 / 8)
    }

    b[0x0480] = 0; // no word separators
    b[0x0481] = 6; // entry length
    b[0x03E0] = 80; // text-buffer max
    b[0x0440] = 8; // parse-buffer max words

    b[0x40] = 0; // main: 0 locals
    let mut pc = 0x41;
    for ch in b"READY>" {
        b[pc] = 0xE5; // print_char
        b[pc + 1] = 0x7F; // one small operand
        b[pc + 2] = *ch;
        pc += 3;
    }
    b[pc] = 0xE4; // read
    b[pc + 1] = 0x0F; // two large constants
    b[pc + 2] = 0x03;
    b[pc + 3] = 0xE0; // text buffer
    b[pc + 4] = 0x04;
    b[pc + 5] = 0x40; // parse buffer
    b[pc + 6] = 0x00; // store on stack
    pc += 7;
    for ch in b"OK" {
        b[pc] = 0xE5;
        b[pc + 1] = 0x7F;
        b[pc + 2] = *ch;
        pc += 3;
    }
    b[pc] = 0xBA; // quit
    b
}

fn output_of(m: &Machine) -> String {
    m.output()
        .as_any()
        .downcast_ref::<BufferOutput>()
        .expect("BufferOutput")
        .buf
        .clone()
}

#[test]
fn packed_main_prints_reads_and_quits() {
    let mem = Memory::new(v6_ready_story()).expect("v6 header");
    assert_eq!(mem.version(), 6);
    let mut m = Machine::boot(mem, Box::new(BufferOutput::new()), BootConfig::new());
    assert!(
        m.screen.v6.is_some(),
        "v6 boot installs the eight-window table"
    );

    let mut steps = 0u32;
    loop {
        match m.step() {
            StepResult::NeedLine { .. } => break,
            StepResult::Continue => {
                steps += 1;
                assert!(steps < 10_000, "did not reach @read");
            }
            other => panic!("expected NeedLine, got {other:?}\n{}", output_of(&m)),
        }
    }
    assert!(
        output_of(&m).contains("READY>"),
        "packed main should print the prompt, got {:?}",
        output_of(&m)
    );

    m.supply_line("look", 13);
    steps = 0;
    loop {
        match m.step() {
            StepResult::Quit => break,
            StepResult::Continue => {
                steps += 1;
                assert!(steps < 10_000, "did not reach @quit");
            }
            other => panic!(
                "expected Quit after the line, got {other:?}\n{}",
                output_of(&m)
            ),
        }
    }
    assert!(
        output_of(&m).contains("OK"),
        "story should continue after @read, got {:?}",
        output_of(&m)
    );
}

#[test]
fn picture_data_without_resources_does_not_invent_pictures() {
    let mem = Memory::new(v6_ready_story()).expect("v6 header");
    let m = Machine::boot(mem, Box::new(BufferOutput::new()), BootConfig::new());
    assert_eq!(m.picture_count(), 0);
    assert_eq!(m.picture_dims(1), None);
}
