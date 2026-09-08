//! Minimal stdin/stdout host for a Glulx story — a bare `.ulx` image, or a
//! `.gblorb` archive with one packed inside (unwrapped here with
//! `lanthorn-blorb`, a zero-external-dependency sibling crate already a
//! dev-dependency of this one for gvm's own tests; `gvm` itself never reads
//! Blorb).
//!
//! Otherwise no dependencies beyond the standard library. [`StdioBackend`]
//! is the smallest usable [`GlkBackend`]: one text buffer window's output,
//! captured as plain text with styling discarded; every other Glk facility
//! (extra windows, graphics, sound, timers, mouse, hyperlinks) is left at
//! the trait's own "the host has no such facility" default. A real host
//! (lanthorn's `AppGlk`, for instance) implements far more of the trait to
//! render windows, colour and images.
//!
//! ```text
//! cargo run -p lanthorn-gvm --example run_story -- path/to/story.gblorb
//! ```

use std::any::Any;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::process;

use gvm::{GlkBackend, GlkStyle, Machine, Memory, StepResult};

/// Captures every `put_text` call as plain text, in print order, discarding
/// which window or style produced it — enough for a single-window story.
#[derive(Default)]
struct StdioBackend {
    output: String,
}

impl GlkBackend for StdioBackend {
    fn put_text(&mut self, _win: u32, _style: GlkStyle, s: &str) {
        self.output.push_str(s);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Write and clear whatever the backend has accumulated since the last drain.
fn drain(m: &mut Machine, out: &mut impl Write) {
    let backend = m
        .backend_mut()
        .as_any_mut()
        .downcast_mut::<StdioBackend>()
        .expect("run_story always installs a StdioBackend");
    if !backend.output.is_empty() {
        let _ = write!(out, "{}", backend.output);
        backend.output.clear();
    }
}

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: run_story <story.ulx>");
        process::exit(2);
    });

    let bytes = fs::read(&path).unwrap_or_else(|e| {
        eprintln!("run_story: cannot read {path}: {e}");
        process::exit(1);
    });

    // Unwrap a .gblorb; a bare .ulx image passes through unchanged.
    let image = if blorb::Blorb::is_blorb(&bytes) {
        let archive = blorb::Blorb::parse(bytes).unwrap_or_else(|e| {
            eprintln!("run_story: invalid Blorb archive: {e:?}");
            process::exit(1);
        });
        let (kind, exec) = archive.executable().unwrap_or_else(|e| {
            eprintln!("run_story: no executable chunk in archive: {e:?}");
            process::exit(1);
        });
        if kind != blorb::ExecKind::Glulx {
            eprintln!("run_story: {path} is not a Glulx story");
            process::exit(1);
        }
        exec.to_vec()
    } else {
        bytes
    };

    let mem = Memory::new(image).unwrap_or_else(|e| {
        eprintln!("run_story: not a valid Glulx image: {e:?}");
        process::exit(1);
    });

    let mut m = Machine::with_glk(mem, Box::new(StdioBackend::default()));
    let stdin = io::stdin();
    let mut out = io::stdout();

    loop {
        match m.step() {
            StepResult::Continue => {}
            StepResult::NeedLine { .. } => {
                drain(&mut m, &mut out);
                let _ = write!(out, "> ");
                let _ = out.flush();
                let mut line = String::new();
                if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
                    println!();
                    break; // EOF
                }
                m.supply_line(line.trim());
            }
            StepResult::NeedChar { .. } => {
                drain(&mut m, &mut out);
                m.supply_char(' ' as u32); // "press any key" — we press space
            }
            StepResult::Quit => {
                drain(&mut m, &mut out);
                break;
            }
            StepResult::Fault => {
                drain(&mut m, &mut out);
                eprintln!("run_story: story faulted: {:?}", m.take_fault_trace());
                break;
            }
            // Save state has nowhere to go from a stdin/stdout host, so this
            // one always declines. A real host writes/reads a file here
            // (see gvm-cli for the Quetzal-plus-Glk-state shape).
            StepResult::SaveRequest => m.complete_save(false),
            StepResult::RestoreRequest => m.complete_restore_failure(),
            StepResult::NeedFilename { .. } => m.supply_filename(None),
            StepResult::NeedEvent { .. } => {
                drain(&mut m, &mut out);
                eprintln!(
                    "run_story: story is waiting on a timer/mouse/hyperlink event, \
                     which this minimal host does not supply"
                );
                break;
            }
            // StepResult is #[non_exhaustive]; a future variant should not
            // fail to build here, only fall through unhandled.
            _ => {}
        }
    }
    drain(&mut m, &mut out);
}
