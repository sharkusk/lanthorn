//! `zvm` — a zero-dependency virtual machine for the Z-machine, the format
//! Infocom's own compiler (and, later, Inform) produced. Covers every
//! published Z-machine version, 1 through 8, including the graphical Version
//! 6 that Infocom shipped only on *Zork Zero*, *Arthur*, *Shogun* and
//! *Journey*. Structured like `gvm` and `scott`, its Glulx and Scott Adams
//! counterparts in this workspace: a [`memory::Memory`] over the loaded story
//! image and a [`cpu::exec::Machine`] execution engine.
//!
//! Like `gvm` and `scott`, this crate takes **zero external dependencies** —
//! all parsing, text codecs, and Quetzal save/restore encoding are
//! hand-rolled — so it can sit behind any host's own I/O policy. lanthorn (a
//! terminal interactive-fiction player) is one such host, used as a worked
//! example below, but this crate does not know it exists.
//!
//! Two Cargo features: `grammar` (on by default) gates `pub mod grammar`, a
//! reader for the story's own verb/syntax tables — an optional dependency on
//! the workspace's `grammar-model` crate, so `--no-default-features` takes no
//! dependency at all. `fixtures` (off by default) gates a test-fixture loader
//! meant only for this crate's own integration tests, never for a normal
//! dependent — see that module's docs.
//!
//! # Loading a story
//!
//! [`memory::Memory::new`] validates a story image (header shape, a
//! supported version, a consistent memory map) and returns an
//! [`error::ZError`] naming what's wrong when it doesn't fit.
//!
//! # Booting
//!
//! [`cpu::exec::BootConfig`] carries every fact a host has an opinion about —
//! whether to honour the story's own colour requests, a PRNG seed, the
//! default page/ink pair, the interpreter number and version to advertise,
//! the palette, the Version 6 character cell and picture-space scale, the
//! screen size — and [`cpu::exec::Machine::boot`] applies all of them in the
//! one order that is correct. A dozen individually-reasonable setters have a
//! real ordering constraint the crate used to leave entirely to the host to
//! discover; see [`cpu::exec::BootConfig`]'s own module docs for why the
//! order matters and which defect taught each clause of it. A bare
//! `BootConfig::new()` states no opinion and boots exactly as the crate
//! always has. The returned machine is stopped at the story's first
//! instruction — Version 6's packed `main` routine (ZMSD §5.4) or the
//! header's `initial_pc` everywhere else — ready for its first `step()`; it
//! does not run the story, so a host can trace, instrument or abandon that
//! first run.
//!
//! # Driving the story: the `StepResult` protocol
//!
//! The host drives an explicit, pull-based protocol — nothing runs on a
//! callback or a background thread. [`cpu::exec::Machine::step`] runs one
//! instruction and returns a [`cpu::exec::StepResult`] that is `Continue`
//! until the machine needs something from the host:
//!
//! * [`cpu::exec::StepResult::NeedLine`] / [`cpu::exec::StepResult::NeedChar`]
//!   — the story wants a line of text or a single keypress. Answer with
//!   [`cpu::exec::Machine::supply_line`] / [`cpu::exec::Machine::supply_char`],
//!   then keep stepping.
//! * [`cpu::exec::StepResult::Restart`] — the `restart` opcode. Answer with
//!   [`cpu::exec::Machine::restart`], never by rebuilding the machine from the
//!   original bytes: ZMSD §6.1.3 requires the two game-writable `Flags 2`
//!   bits to survive a restart, along with every mid-session host setting —
//!   a rebuilt machine loses both.
//! * [`cpu::exec::StepResult::SaveRequest`] / [`cpu::exec::StepResult::RestoreRequest`]
//!   — the `save`/`restore` opcodes. Answer with
//!   [`cpu::exec::Machine::complete_save`] and either
//!   [`cpu::exec::Machine::complete_restore_success`] or
//!   [`cpu::exec::Machine::complete_restore_failure`].
//! * [`cpu::exec::StepResult::Quit`] — the story ended cleanly; stop stepping.
//! * [`cpu::exec::StepResult::Fault`] — a runtime fault halted the machine,
//!   distinct from `Quit` so a host that never reads diagnostics can still
//!   tell a crash from a clean exit; [`cpu::exec::Machine::take_fault_trace`]
//!   gives the detail.
//!
//! `StepResult` is `#[non_exhaustive]`: match it with a wildcard arm so a
//! future variant does not fail to build.
//!
//! # Rendering
//!
//! [`cpu::exec::Machine`]'s `screen` field is a public [`screen::ScreenState`]
//! — window layout, cursor, text style, and (for Version 6) the
//! pixel-addressed window model — a host walks it directly to render a
//! frame. Text itself flows through the pluggable [`io::Output`] sink a host
//! supplies at boot: [`io::BufferOutput`] is a minimal accumulating sink for
//! tests and headless use, and a host rendering a real screen implements the
//! trait itself, the way lanthorn's `CaptureSink` drives a scrolling
//! terminal transcript.
//!
//! # Draining
//!
//! Several independent queues accumulate host-facing events between drains,
//! and nothing clears them automatically — a host renders a frame (or ends a
//! turn) and then takes each queue it cares about:
//!
//! * [`cpu::exec::Machine::take_paint_events`] — every Version 6
//!   `draw_picture`/`erase_picture`/`erase_window` fill the story issued,
//!   merged onto ONE timeline in issue order. A game interleaves pictures and
//!   fills freely, so replaying two separate lists would reorder them.
//! * [`cpu::exec::Machine::take_pending_sounds`] — `sound_effect` events.
//! * [`cpu::exec::Machine::take_diagnostics`] — host-facing diagnostic lines
//!   (unimplemented opcodes, sampled sounds); the engine never prints these
//!   itself.
//! * [`cpu::exec::Machine::take_screen_trace`] — a step-by-step screen-model
//!   trace, populated only once a host turns tracing on.
//! * [`cpu::exec::Machine::take_fault_trace`] — the stack trace behind a
//!   [`cpu::exec::StepResult::Fault`].
//!
//! # Feeding input
//!
//! [`cpu::exec::Machine::supply_line`] answers a suspended `NeedLine`;
//! [`cpu::exec::Machine::supply_char`] answers a suspended `NeedChar`.
//!
//! # Picture resources
//!
//! A Version 6 story asks `picture_data` (ZMSD §15) how many pictures its
//! archive holds and how big each one is. [`resources::Resources`] answers
//! those questions — implement it for a host whose archive is streamed or
//! lazily decoded, or use [`resources::PictureTable`] (what
//! [`cpu::exec::Machine::set_picture_dims`] and
//! [`cpu::boot::BootConfig::with_picture_dims`] build) for a host that
//! already has the whole table. Install either with
//! [`cpu::exec::Machine::set_resources`] or
//! [`cpu::boot::BootConfig::with_resources`].
//!
//! # Saving
//!
//! [`cpu::exec::Machine::save_quetzal`] returns the spec-conformant Quetzal
//! `IFhd`/`CMem`/`Stks` blob any Z-machine interpreter can read back with
//! [`cpu::exec::Machine::restore_quetzal`] — this is what `@save`/`@restore`
//! in the story itself produce and consume. **Quetzal carries no screen
//! state by design** — the standard assumes the story repaints after a
//! restore — so a host snapshotting a live display (a "Save State" outside
//! the story's own save points) has to keep the screen itself; today that is
//! entirely the host's responsibility, with nothing in this crate to help.
//!
//! # Example
//!
//! A minimal story built by hand — no story file needed — loaded, booted,
//! and driven to its first (and only) instruction, `quit`:
//!
//! ```rust
//! use zvm::cpu::exec::{BootConfig, Machine, StepResult};
//! use zvm::io::BufferOutput;
//! use zvm::memory::Memory;
//!
//! // A structurally valid v3 header over a 0x400-byte image, laid out the
//! // way every unit test in this crate builds one: high memory, dictionary,
//! // object table, globals and abbreviations each pointed at a distinct,
//! // otherwise-empty region, and a single `quit` instruction (0OP:0xBA) at
//! // the initial PC.
//! let mut image = vec![0u8; 0x400];
//! image[0x00] = 3; // version 3
//! image[0x04..0x06].copy_from_slice(&0x0400u16.to_be_bytes()); // high_mem_base
//! image[0x06..0x08].copy_from_slice(&0x0040u16.to_be_bytes()); // initial_pc
//! image[0x08..0x0A].copy_from_slice(&0x0200u16.to_be_bytes()); // dictionary
//! image[0x0A..0x0C].copy_from_slice(&0x0100u16.to_be_bytes()); // object_table
//! image[0x0C..0x0E].copy_from_slice(&0x0300u16.to_be_bytes()); // global_vars
//! image[0x0E..0x10].copy_from_slice(&0x0400u16.to_be_bytes()); // static_mem_base
//! image[0x18..0x1A].copy_from_slice(&0x0040u16.to_be_bytes()); // abbrev_table
//! image[0x40] = 0xBA; // quit
//!
//! let mem = Memory::new(image).expect("valid header");
//! let mut m = Machine::boot(mem, Box::new(BufferOutput::new()), BootConfig::new());
//! assert_eq!(m.step(), StepResult::Quit);
//! ```
//!
//! See `examples/run_story.rs` for a complete, runnable stdin/stdout host
//! that loads a real story file and plays it.

pub mod cpu;
pub mod dictionary;
pub mod error;
#[cfg(feature = "fixtures")]
pub mod fixtures;
#[cfg(feature = "grammar")]
pub mod grammar;
pub mod header;
pub mod ifid;
pub mod interpreter;
pub mod io;
pub mod location;
pub mod machines;
pub mod memory;
pub mod objects;
pub mod paint_log;
pub mod quetzal;
pub mod resources;
pub mod screen;
pub mod screen_snapshot;
pub mod text;
pub mod world;

pub use objects::ObjectSnapshot;
