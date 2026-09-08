//! `gvm` — a zero-dependency virtual machine for Glulx (versions 2 and 3),
//! the 32-bit virtual machine Inform 7 and modern Inform 6 stories compile
//! to. Structured like `zvm`, the Z-machine counterpart it sits beside in
//! this workspace: a [`memory::Memory`] over the loaded image and the
//! [`Machine`] execution engine. Text output flows through the [`glk`]
//! window/stream model — the interactive-fiction subset of Andrew Plotkin's
//! Glk spec 0.7.5 — to a pluggable [`glk::GlkBackend`] display that the host
//! supplies.
//!
//! All opcode numbers, addressing modes, the header/call-frame layout, and the
//! Glk window/stream/style model are transcribed from the Glulx and Glk
//! specifications into `GLULX_NOTES.md`, and the code is implemented against
//! that file. Like `zvm` and `scott`, this crate takes **zero external
//! dependencies**, so it can sit behind any host's own I/O policy — lanthorn
//! (a terminal interactive-fiction player) is one such host, used as a
//! worked example below, but this crate does not know it exists.
//!
//! # Loading and driving a session
//!
//! [`Memory::new`] validates a Glulx image (the `Glul` magic, a supported
//! major version, a consistent memory map) and returns a [`GError`] naming
//! what's wrong when it doesn't fit. [`Machine::with_glk`] then takes that
//! memory plus a `Box<dyn GlkBackend>` and builds a machine sitting at the
//! start function, ready to run.
//!
//! From there the host drives an explicit, pull-based protocol — nothing
//! runs on a callback or a background thread:
//!
//! * [`Machine::step`] runs until the machine needs something from the host,
//!   returning a [`StepResult`]: [`StepResult::NeedLine`] /
//!   [`StepResult::NeedChar`] (supply input with [`Machine::supply_line`] /
//!   [`Machine::supply_char`], then keep stepping), [`StepResult::Quit`] (the
//!   story ended cleanly), or [`StepResult::Fault`] (a runtime fault halted
//!   it — distinct from `Quit` so a host that never reads
//!   [`Machine::diagnostics`] can still tell a crash from a clean exit;
//!   [`Machine::take_fault_trace`] gives the detail).
//! * Sound, timer, mouse, hyperlink and window-arrangement events are
//!   likewise pulled rather than pushed: the VM holds no clock or event
//!   queue of its own. [`Machine::glk_timer_interval`] reports the interval
//!   the game last requested (or `None`); the host is the one keeping wall
//!   time and calls [`Machine::deliver_timer`] when it elapses, and
//!   similarly for [`Machine::deliver_arrange`], [`Machine::deliver_mouse`],
//!   [`Machine::deliver_hyperlink`], [`Machine::deliver_sound_notify`] and
//!   [`Machine::deliver_volume_notify`].
//!
//! [`GlkBackend`] is the seam: 35 methods, all but two (`as_any`/`as_any_mut`)
//! defaulted to a no-op or "the host has no such facility", so a minimal
//! text-only backend implements almost nothing and gets windows, styled
//! text, graphics, sound, screen size and glyph metrics for free the moment
//! it wants them. [`TestBackend`] is a complete in-memory implementation
//! (built for this crate's own tests, and used in the example below); a
//! real host implements the trait itself the way lanthorn's `AppGlk` does.
//!
//! # Saving
//!
//! [`Machine::save_quetzal`] returns the spec-conformant Quetzal `IFhd` /
//! `CMem` / `Stks` / `MAll` blob any Glulx interpreter can read back with
//! [`Machine::complete_restore_quetzal`] — this is what `@save`/`@restore`
//! in the story itself produce and consume, via [`StepResult::SaveRequest`]
//! / [`StepResult::RestoreRequest`]. [`Machine::save_state`] adds this
//! crate's own `GReg`/`Glk ` chunks (Glk state the story didn't ask to
//! save, needed for a host that wants to snapshot a session anywhere rather
//! than only at the game's own save points); [`Machine::complete_restore_success`]
//! reads either shape back. Quetzal carries no screen contents by design —
//! the story is expected to repaint — so a host snapshotting a live display
//! keeps that state itself.
//!
//! # Example
//!
//! This sketches the full loop against a real story file (a Glulx `.gblorb`
//! or bare `.ulx` image), using [`TestBackend`] as the minimal available
//! [`GlkBackend`]; see `examples/run_story.rs` for a complete, runnable
//! stdin/stdout host.
//!
//! ```no_run
//! use gvm::{GlkBackend, Machine, Memory, StepResult, TestBackend};
//!
//! let image = std::fs::read("story.ulx").expect("read story file");
//! let mem = Memory::new(image).expect("valid Glulx image");
//! let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
//!
//! loop {
//!     match m.step() {
//!         StepResult::Continue => {}
//!         StepResult::NeedLine { .. } => {
//!             let mut line = String::new();
//!             std::io::stdin().read_line(&mut line).expect("read stdin");
//!             m.supply_line(line.trim());
//!         }
//!         StepResult::NeedChar { .. } => m.supply_char(' ' as u32),
//!         StepResult::Quit => break,
//!         StepResult::Fault => {
//!             eprintln!("story faulted: {:?}", m.take_fault_trace());
//!             break;
//!         }
//!         // A real host also answers SaveRequest/RestoreRequest and the
//!         // other StepResult variants; see examples/run_story.rs.
//!         _ => {}
//!     }
//! }
//! ```

pub mod accel;
#[cfg(test)]
mod asm;
pub(crate) mod decode;
pub mod disasm;
pub mod error;
pub mod exec;
pub mod glk;
#[cfg(feature = "grammar")]
pub mod grammar;
pub mod header;
// `i7map` and `world` read the object/room model through
// `objects::ParseNames`, which in turn locates the dictionary through
// `grammar::locate` — there is no way to find either without a grammar
// table reader, so all three live behind the same feature.
#[cfg(feature = "grammar")]
pub mod i7map;
pub mod memory;
#[cfg(feature = "grammar")]
pub mod objects;
pub mod trace;
mod unicode_norm;
#[rustfmt::skip]
mod unicode_norm_tables;
pub mod veneer;
#[cfg(feature = "grammar")]
pub mod world;

pub use error::GError;
pub use exec::{Machine, SaveLoadRequest, StepResult};
pub use glk::{GlkBackend, GlkStyle, Model as GlkModel, TestBackend, WinType};
pub use memory::{Memory, WriteFault};
pub use trace::{StackTrace, TraceFrame};
pub use veneer::{CrossCheck, VeneerReport};
