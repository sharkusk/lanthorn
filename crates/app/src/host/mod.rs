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
//! - [`turn`] — apply a finished turn: transcript, map, sound, pager, saves.
//! - [`sound`] — the [`SoundSink`](sound::SoundSink) a host plays sound through,
//!   and [`sound_finished`](sound::sound_finished) for reporting one back.
//! - [`ingame_io`] — the game's own SAVE/RESTORE and filename requests.
//! - [`settings`] — apply a changed config to a running session, as the
//!   settings screen's Save does (SQ-1559).
//! - [`assist`] — the Guiding Light's per-game switch, and the command band's
//!   data (SQ-1549). Completion and the reveal's text-in/words-out variant are
//!   pure enough that they live beside what they're twins of instead —
//!   [`crate::complete`] and [`crate::reveal::arm_from_text`].

pub mod assist;
pub mod boot;
pub mod clock;
pub mod ingame_io;
pub mod persist;
pub mod probe;
pub mod reset;
pub mod screen;
pub mod settings;
pub mod sound;
pub mod turn;

pub use assist::{refresh_band_data, set_guidance, BandData};
pub use boot::{
    boot_story, random_seed_line, resolve_pict_blorb, story_screen_in, BootError, BootHooks,
    BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts,
};
pub use turn::{
    apply_game_driven_result, finish_command_turn, finish_resumed_turn, Paging, TurnOutcome,
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
