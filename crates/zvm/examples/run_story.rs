//! Minimal stdin/stdout host for a Z-machine story (versions 1-5, 7-8;
//! Version 6, the graphical variant, needs a real renderer and is refused).
//!
//! No dependencies beyond the standard library. Loads the story named on the
//! command line, boots it with [`BootConfig::new`] and a plain stdout sink,
//! then drives the [`Machine::step`]/[`StepResult`] protocol until the game
//! quits or stdin runs out. A real host (lanthorn, `zvm-cli`) also handles
//! `@save`/`@restore` files, paging, styled/coloured text and terminal
//! raw-mode input — all left out here for clarity.
//!
//! ```text
//! cargo run -p lanthorn-zvm --example run_story -- path/to/story.z5
//! ```

use std::any::Any;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::process;

use zvm::cpu::exec::{BootConfig, Machine, StepResult};
use zvm::io::Output;
use zvm::memory::Memory;

/// Prints everything the story emits straight to stdout — no wrapping,
/// paging or styling. The simplest possible [`Output`] sink.
#[derive(Default)]
struct StdoutSink;

impl Output for StdoutSink {
    fn print(&mut self, s: &str) {
        print!("{s}");
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// ZSCII 13: the "Enter" terminator every ordinary line of typed input ends
/// with (ZMSD §3.8, §15 `read`). A real host also watches for the story's own
/// terminating-characters table (header `$2E`, v5+) to support function-key
/// commands; this example never does.
const ENTER: u8 = 13;

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: run_story <story-file>");
        process::exit(2);
    });

    let bytes = fs::read(&path).unwrap_or_else(|e| {
        eprintln!("run_story: cannot read {path}: {e}");
        process::exit(1);
    });

    let mem = Memory::new(bytes).unwrap_or_else(|e| {
        eprintln!("run_story: invalid story file: {e:?}");
        process::exit(1);
    });

    if mem.version() == 6 {
        eprintln!("run_story: {path} is a Version 6 (graphical) story; this text-only example refuses it");
        process::exit(1);
    }

    let mut m = Machine::boot(mem, Box::new(StdoutSink), BootConfig::new());
    let stdin = io::stdin();

    loop {
        match m.step() {
            StepResult::Continue => {}
            StepResult::Quit => break,
            // ZMSD §6.1.3: answer with `restart()`, never by rebuilding the
            // machine — that would lose the two game-writable Flags 2 bits.
            StepResult::Restart => m.restart(),
            StepResult::NeedLine { .. } => {
                io::stdout().flush().ok();
                let mut line = String::new();
                if stdin.lock().read_line(&mut line).expect("read stdin") == 0 {
                    println!();
                    break; // EOF
                }
                m.supply_line(line.trim_end_matches(['\n', '\r']), ENTER);
            }
            StepResult::NeedChar => {
                io::stdout().flush().ok();
                let mut line = String::new();
                if stdin.lock().read_line(&mut line).expect("read stdin") == 0 {
                    break; // EOF
                }
                let ch = line.chars().next().unwrap_or(ENTER as char) as u32;
                m.supply_char(u8::try_from(ch).unwrap_or(ENTER));
            }
            // No file I/O in this minimal example — decline every save and
            // report failure for every restore, exactly as a host with
            // nowhere to write must.
            StepResult::SaveRequest => m.complete_save(false),
            StepResult::RestoreRequest => m.complete_restore_failure(),
            StepResult::Fault => {
                eprintln!("\nrun_story: story faulted: {:?}", m.take_fault_trace());
                break;
            }
            // StepResult is #[non_exhaustive]; a future variant should not
            // fail to build here.
            _ => {}
        }
    }
}
