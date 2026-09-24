//! Where a story's sounds go (SQ-1538).
//!
//! The engines REPORT sound — a Z-machine turn's `@sound_effect` calls
//! ([`crate::session::TurnResult::sounds`]) and a Glulx turn's Glk
//! sound-channel operations (`glulx_sound_ops`) — and the per-turn apply turns
//! those reports into playback. Which resource plays, on which channel, with
//! which finish routine or notify waiting on it, is a RULE, and lives in
//! [`AppState::play_turn_sounds`](crate::state::AppState::play_turn_sounds) /
//! [`play_glulx_sound_ops`](crate::state::AppState::play_glulx_sound_ops) for
//! every host. Only the last step — making noise — is the host's, through a
//! [`SoundSink`].
//!
//! The TUI's sink is the output device (`audio::AudioBackend`), opened lazily
//! on the first sound a story actually plays (SQ-1423). A host that delivers
//! sound elsewhere — a browser, a phone — installs its own sink on
//! `AppState::audio` before the first turn, records or forwards what it is
//! handed, and reports each sound that finishes back through
//! [`sound_finished`], which runs the story's finish routine or Glk
//! sound-notify exactly as the TUI does when its device reports one.

use mapper::mapper::Mapper;

use crate::engine::Engine;
use crate::engine_helpers::{glulx_session_opt_mut, zvm_session_opt_mut};
use crate::state::AppState;

pub use audio::{SoundFormat, SoundId};
// The device-free decoders (SQ-1541), for a host whose sink delivers sound
// somewhere that wants samples or a WAV file rather than a Blorb resource.
#[cfg(feature = "mod-music")]
pub use audio::render_mod;
pub use audio::{bleep, decode_aiff, tone, Pcm};

/// How loud a sampled sound starts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SampleLevel {
    /// A Z-machine `@sound_effect` volume, 1–8 (255 = loudest), ZMSD §15.
    ZVolume(u8),
    /// A Glk channel's linear gain, 1.0 = full (Glk 0.7.3 §8.3).
    Gain(f32),
}

/// A sampled sound to start: which resource it is, its bytes, and how to play them.
#[derive(Debug, Clone, Copy)]
pub struct SampleStart<'a> {
    /// The resource number the story named — the Z-machine sound number or the
    /// Glk sound resource. What a forwarding host sends along so the far end can
    /// fetch the resource itself.
    pub resource: u32,
    /// The resource's bytes, already resolved (medium first, then Blorb).
    pub bytes: &'a [u8],
    pub format: SoundFormat,
    pub level: SampleLevel,
    /// Plays: 1 = once, 255 = forever (ZMSD §15; a Glk `repeats` of -1 arrives as 255).
    pub repeats: u8,
}

/// Where sound goes: the host's end of playback.
///
/// Every id a sink hands back from [`SoundSink::play`] is its own; the per-turn
/// apply keeps what it means (which story sound, which finish routine) and
/// passes it back to stop, pause or re-gain that sound later.
pub trait SoundSink: std::fmt::Debug {
    /// A Z-machine bleep (§15 sound 1 = high, 2 = low) as a `freq_hz` tone.
    fn tone(&mut self, freq_hz: f32, ms: u32, z_volume: u8);
    /// Start a sampled sound; `None` when it could not be started, in which case
    /// no finish routine is ever waited on for it.
    fn play(&mut self, sound: SampleStart<'_>) -> Option<SoundId>;
    fn stop(&mut self, id: SoundId);
    fn pause(&mut self, id: SoundId);
    fn unpause(&mut self, id: SoundId);
    /// Set a playing sound's linear gain (a Glk volume change or ramp step).
    fn set_gain(&mut self, id: SoundId, gain: f32);
    fn stop_all(&mut self);
    /// The master volume, 0–100 (`[sound] volume`).
    fn set_volume(&mut self, volume: u8);
    /// Sounds that have finished since the last call. A sink that learns of
    /// finished sounds some other way may answer empty here and report them
    /// through [`sound_finished`] instead.
    fn finished(&mut self) -> Vec<SoundId>;
}

#[cfg(feature = "playback")]
impl SoundSink for audio::AudioBackend {
    fn tone(&mut self, freq_hz: f32, ms: u32, z_volume: u8) {
        self.play_tone(freq_hz, ms, z_volume);
    }
    fn play(&mut self, s: SampleStart<'_>) -> Option<SoundId> {
        match s.level {
            SampleLevel::ZVolume(v) => self.play_sample(s.bytes, s.format, v, s.repeats),
            SampleLevel::Gain(g) => self.play_sample_gain(s.bytes, s.format, g, s.repeats),
        }
    }
    fn stop(&mut self, id: SoundId) {
        audio::AudioBackend::stop(self, id);
    }
    fn pause(&mut self, id: SoundId) {
        audio::AudioBackend::pause(self, id);
    }
    fn unpause(&mut self, id: SoundId) {
        audio::AudioBackend::unpause(self, id);
    }
    fn set_gain(&mut self, id: SoundId, gain: f32) {
        self.set_sample_gain(id, gain);
    }
    fn stop_all(&mut self) {
        audio::AudioBackend::stop_all(self);
    }
    fn set_volume(&mut self, volume: u8) {
        audio::AudioBackend::set_volume(self, volume);
    }
    fn finished(&mut self) -> Vec<SoundId> {
        audio::AudioBackend::finished(self)
    }
}

/// The sink a state opens when a story first plays a sound and the host
/// installed none: the output device at `volume` — or, built without the
/// `playback` feature (SQ-1541), [`Silence`], since there is no device to open.
#[cfg(feature = "playback")]
pub fn default_sound_sink(volume: u8) -> Box<dyn SoundSink> {
    Box::new(audio::AudioBackend::new(volume))
}

/// The sink a state opens when a story first plays a sound and the host
/// installed none. Built without the `playback` feature (SQ-1541) there is no
/// device to open, so it is [`Silence`]; a host that wants the sound installs
/// its own sink on `AppState::audio` before the first turn.
#[cfg(not(feature = "playback"))]
pub fn default_sound_sink(_volume: u8) -> Box<dyn SoundSink> {
    Box::new(Silence)
}

/// A sink that plays nothing: every sound fails to start, so no finish routine
/// is ever left waiting on one — what a story hears from an interpreter with no
/// sound output at all.
#[derive(Debug, Default, Clone, Copy)]
pub struct Silence;

impl SoundSink for Silence {
    fn tone(&mut self, _freq_hz: f32, _ms: u32, _z_volume: u8) {}
    fn play(&mut self, _sound: SampleStart<'_>) -> Option<SoundId> {
        None
    }
    fn stop(&mut self, _id: SoundId) {}
    fn pause(&mut self, _id: SoundId) {}
    fn unpause(&mut self, _id: SoundId) {}
    fn set_gain(&mut self, _id: SoundId, _gain: f32) {}
    fn stop_all(&mut self) {}
    fn set_volume(&mut self, _volume: u8) {}
    fn finished(&mut self) -> Vec<SoundId> {
        Vec::new()
    }
}

/// The sound `id` finished: forget it, and run whatever the story left waiting on
/// it — a Z-machine finish routine (ZMSD §15, v5+) or a Glk sound-notify event.
/// Returns `true` if that routine ended the game (the caller exits, as it does
/// for any game-driven turn that quits).
///
/// The TUI calls this for every id its device's [`SoundSink::finished`]
/// reports; a host whose sink learns of finished sounds by message calls it
/// directly. `map_view` is as for [`crate::host::apply_game_driven_result`].
pub fn sound_finished(
    state: &mut AppState,
    mapper: &mut Mapper,
    session: &mut dyn Engine,
    id: SoundId,
    game_dir: &std::path::Path,
    map_view: Option<(u16, u16)>,
) -> bool {
    // Always forget the number->id mapping for a finished sound, even one
    // with no finish routine.
    state.sound_ids.retain(|_, v| *v != id);
    if let Some(routine) = state.sound_routines.remove(&id) {
        if routine != 0 {
            if let Some(zs) = zvm_session_opt_mut(session) {
                let result = zs.run_sound_finish(routine);
                if super::apply_game_driven_result(
                    state, mapper, &result, game_dir, map_view, &*session, crate::pager::Driver::Timeout,
                ).quit {
                    return true;
                }
            }
        }
    }
    // Glulx sound-notify: a finished channel delivers Evtype_SoundNotify.
    if let Some((snd, notify)) = state.glulx_sound_notify.remove(&id) {
        state.glulx_channels.retain(|_, v| *v != id);
        if let Some(gs) = glulx_session_opt_mut(session) {
            let result = gs.sound_notify(snd, notify);
            if super::apply_game_driven_result(
                state, mapper, &result, game_dir, map_view, &*session, crate::pager::Driver::Timeout,
            ).quit {
                return true;
            }
        }
    }
    false
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;

    /// With no device the sink starts nothing, so nothing is ever left waiting
    /// on a finish routine — the story hears an interpreter with no sound.
    #[test]
    fn silence_starts_nothing_and_reports_nothing_finished() {
        let mut s = Silence;
        s.tone(HIGH, 150, 8);
        let bytes = [0u8; 4];
        let started = s.play(SampleStart {
            resource: 3,
            bytes: &bytes,
            format: SoundFormat::Aiff,
            level: SampleLevel::ZVolume(8),
            repeats: 1,
        });
        assert_eq!(started, None);
        assert!(s.finished().is_empty());
    }

    const HIGH: f32 = audio::HIGH_BLEEP_HZ;
}
