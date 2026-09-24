//! The session host: the story-playing rules the TUI runs on, as a library an
//! embedding host that is NOT a terminal can drive too (SQ-1537 onward).
//!
//! lanthorn's front end is a terminal, but booting a story, applying a turn,
//! firing a game clock and saving a resume point are not terminal questions —
//! they are rules about the game, and a GUI, a network server or a mobile
//! binding needs exactly the same ones. They used to live in the binary's own
//! modules (`startup.rs`, `turn.rs`, …), written against `ratatui` and
//! `crossterm`, so nothing outside the TUI could reach them without copying
//! them. This module is where they live now; the TUI is one caller of it and
//! runs on the same code, so there is one copy of each rule.
//!
//! What stays in the binary is what only a terminal can do: raw mode, the
//! alternate screen, the `ratatui::Terminal`, the image-protocol and OSC colour
//! probes, the fd-2 redirect and the panic hook. Where boot needs one of those
//! answers, the request carries it as a plain value ([`TerminalFacts`]) that a
//! headless host leaves at its default.
//!
//! - [`boot`] — boot a story to its first prompt and hand back a ready session.

pub mod boot;

pub use boot::{
    boot_story, random_seed_line, resolve_pict_blorb, story_screen_in, BootError, BootHooks,
    BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts,
};

use crate::engine::Engine;

/// Drain the engine's `screen` trace and, when `on`, append it to trace.log.
/// Always drains (so the buffer never grows while the section is off between a
/// runtime toggle). (trace feature)
pub fn flush_screen_trace(user_dir: &std::path::Path, session: &mut dyn Engine, on: bool) {
    let lines = session.take_screen_trace();
    if on {
        crate::trace::write(user_dir, crate::trace::Section::Screen, &lines);
    }
}

/// When `on` and the story is v6, append this turn's `v6` window/picture-canvas
/// state snapshot to trace.log. Unlike `flush_screen_trace`, there is no buffer
/// to drain — the snapshot reads live state directly — so this is skipped
/// entirely (no snapshot built) when the section is off. (trace feature)
pub fn flush_v6_trace(user_dir: &std::path::Path, session: &mut dyn Engine, on: bool) {
    if !on {
        return;
    }
    if let Some(lines) = session.v6_snapshot() {
        crate::trace::write(user_dir, crate::trace::Section::V6, &lines);
    }
}
