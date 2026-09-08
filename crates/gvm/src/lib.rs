//! `gvm` — a zero-dependency Glulx virtual machine. Structured like `zvm`: a
//! [`memory::Memory`] over the loaded image and the [`Machine`] execution
//! engine. Text output flows through the [`glk`] window/stream model to a
//! pluggable [`glk::GlkBackend`] display.
//!
//! All opcode numbers, addressing modes, the header/call-frame layout, and the
//! Glk window/stream/style model are transcribed from the Glulx and Glk
//! specifications into `GLULX_NOTES.md`, and the code is implemented against
//! that file.

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
