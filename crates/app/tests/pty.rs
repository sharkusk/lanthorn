//! Group binary: the pty / emitted-stream harnesses (SQ-0762, SQ-0764).
//!
//! Each member below is one of the suites that used to be its own test binary;
//! it now lives in `tests/suites/` (which cargo does not auto-build) and is
//! pulled in here as a module. `cargo nextest run pty_oracle` still selects a
//! single suite, because the module path carries the old filename.
//!
//! WHY THESE THREE ARE ALONE. All three want `tests/pty_stream/` — ~2900 lines
//! of driver, decoder and oracle. Declared per-suite it would be compiled once
//! per member into the same binary, which is exactly the cost this grouping
//! exists to remove, so it is declared ONCE here and the members reach it
//! through `super::pty_stream`.

#![allow(dead_code, unused_imports)]

#[path = "pty_stream/mod.rs"]
mod pty_stream;

#[path = "suites/cast_manifest.rs"]
mod cast_manifest;
#[path = "suites/docs_reference.rs"]
mod docs_reference;
#[path = "suites/gallery_manifest.rs"]
mod gallery_manifest;
#[path = "suites/pty_emitted_stream.rs"]
mod pty_emitted_stream;
#[path = "suites/pty_flank_alpha_seam.rs"]
mod pty_flank_alpha_seam;
#[path = "suites/pty_kitty_compression.rs"]
mod pty_kitty_compression;
#[path = "suites/pty_oracle.rs"]
mod pty_oracle;
#[path = "suites/pty_query_replies.rs"]
mod pty_query_replies;
