//! Minimal stdin/stdout host for a Scott Adams (ScottFree `.dat`) story.
//!
//! No dependencies beyond the standard library. Loads the story named on the
//! command line, then loops: print the room, prompt, read a command line,
//! step the VM, print what it says, repeat until the game quits or stdin
//! runs out. A real host (lanthorn's `scott-cli`, for instance) also handles
//! `/save` and `/restore` via [`Vm::snapshot`] / [`Vm::restore`], paging, and
//! terminal raw-mode input — all left out here for clarity.
//!
//! ```text
//! cargo run -p lanthorn-scott --example run_story -- path/to/story.dat
//! ```

use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::process;

use scott::{Database, StepResult, Vm};

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: run_story <story.dat>");
        process::exit(2);
    });

    let src = fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("run_story: cannot read {path}: {e}");
        process::exit(1);
    });

    // A cheap sniff before the real parse — useful when a host is guessing
    // among several engines from a file's bytes alone.
    if !scott::looks_like_scott(&src) {
        eprintln!("run_story: {path} does not look like a Scott Adams .dat");
        process::exit(1);
    }

    let db = Database::parse(&src).unwrap_or_else(|e| {
        eprintln!("run_story: invalid .dat: {e:?}");
        process::exit(1);
    });

    let mut vm = Vm::new(db);
    let stdin = io::stdin();
    let mut out = io::stdout();

    // Any output the opening occurrence pass produced before the first prompt.
    print!("{}", vm.take_output());

    loop {
        print!("\n{}\n\n> ", vm.room_block());
        out.flush().expect("flush stdout");

        let mut line = String::new();
        if stdin.lock().read_line(&mut line).expect("read stdin") == 0 {
            println!();
            break; // EOF
        }

        vm.supply_line(line.trim());
        match vm.step() {
            StepResult::NeedLine => {}
            StepResult::Quit => {
                print!("{}", vm.take_output());
                break;
            }
            // StepResult is #[non_exhaustive]; scott has no third outcome
            // today, but a future variant should not fail to build here.
            _ => {}
        }
        print!("{}", vm.take_output());
    }
}
