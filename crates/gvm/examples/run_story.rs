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
//! It also answers the Glk fileref/file-VFS seam — [`StepResult::NeedFilename`],
//! [`StepResult::SaveRequest`]/[`StepResult::RestoreRequest`], and the VFS
//! sidecar — the minimum a host needs so a story's files and its `@save`
//! survive between runs. See
//! `docs/internals/gvm-fileref-seam.md` for the full contract; this example
//! is a single-slot simplification of it (one save file, no distinction
//! between the player's SAVE verb and a game's own fixed-name saves — see
//! `gvm-cli`/lanthorn for that).
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

    // The Glk file VFS sidecar: `<story>.glkvfs`. Load it before the run
    // (a story may read a file it wrote last session during its own boot),
    // and clear the dirty flag right after — loading is not a game
    // mutation, so nothing needs writing back yet.
    let vfs_path = format!("{path}.glkvfs");
    if let Ok(bytes) = fs::read(&vfs_path) {
        m.load_vfs(&bytes);
        m.clear_vfs_dirty();
    }
    // The single fixed save slot this minimal host offers, regardless of
    // which fileref the game opened or how (`by_prompt` or not) — see the
    // module doc above for what a full host distinguishes instead.
    let save_path = format!("{path}.glksave.qzl");

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
            // @save / @restore. A memory-stream or resource-stream target
            // resolves inside gvm without ever reaching here (SQ-1427); what
            // arrives here always names an external file the host owns.
            StepResult::SaveRequest => {
                let ok = fs::write(&save_path, m.save_quetzal()).is_ok();
                if ok {
                    eprintln!("run_story: saved to {save_path}");
                } else {
                    eprintln!("run_story: save failed");
                }
                m.complete_save(ok);
            }
            StepResult::RestoreRequest => match fs::read(&save_path) {
                Ok(bytes) if m.complete_restore_quetzal(&bytes) => {
                    eprintln!("run_story: restored from {save_path}");
                }
                _ => {
                    eprintln!("run_story: restore failed (no save at {save_path})");
                    m.complete_restore_failure();
                }
            },
            // glk_fileref_create_by_prompt. SavedGame usage is already
            // host-intercepted above (@save/@restore never resolve into a
            // VFS slot for it — see the seam doc), so a save-file prompt
            // here would be redundant; auto-name it and keep going. Any
            // other usage (Data/Transcript/InputRecord) is a genuine
            // external file the player should name.
            StepResult::NeedFilename { usage, .. } => {
                if usage & 0x0f == 0x01 {
                    m.supply_filename(Some(format!("__prompt_{}__", usage & 0x0f)));
                } else {
                    drain(&mut m, &mut out);
                    let _ = write!(out, "Filename (blank to cancel): ");
                    let _ = out.flush();
                    let mut line = String::new();
                    let _ = stdin.lock().read_line(&mut line);
                    let name = line.trim();
                    m.supply_filename(if name.is_empty() { None } else { Some(name.to_string()) });
                }
            }
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
        // Flush the VFS sidecar whenever a game mutation dirtied it, so a
        // story's own files (scores, preferences, `@save`d-to-VFS states via
        // a Data-usage fileref) survive even if this process is killed
        // mid-session, exactly as the SavedGame slot above survives via
        // fs::write on every @save.
        if m.vfs_dirty() {
            let _ = fs::write(&vfs_path, m.vfs_bytes());
            m.clear_vfs_dirty();
        }
    }
    drain(&mut m, &mut out);
}
