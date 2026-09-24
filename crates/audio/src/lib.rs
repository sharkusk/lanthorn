//! Cross-platform host-side audio backend for lanthorn. Plays synthesised tones
//! (Z-machine bleeps) and decoded samples (Blorb `Snd ` resources) via `rodio`.
//! With the `playback` feature off, the backend is a compile-time no-op.
//!
//! **Decoding needs no device** (SQ-1541). [`decode_aiff`], [`tone`] /
//! [`bleep`] and (with `mod-music`) [`render_mod`] turn a sound into [`Pcm`] —
//! interleaved 16-bit samples — and [`Pcm::to_wav`] wraps it in a RIFF WAVE
//! file, so a host that delivers sound somewhere else (a browser, a phone) can
//! hand it to anything that plays WAV. None of it touches `rodio`, and all of it
//! builds with `default-features = false`. Ogg is left as the bytes it arrived
//! in: decoding it is what `rodio` is here for.

/// Identifies a playing sampled sound so the host can stop it or detect its end.
pub type SoundId = u32;

/// Sampled-sound container format, chosen by the host from the Blorb chunk type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SoundFormat {
    Aiff,
    Ogg,
    Mod,
}

const SAMPLE_RATE: u32 = 44100;

/// Decoded sound, ready for any output: `channels` interleaved signed 16-bit
/// samples at `rate` Hz (a stereo frame is `[left, right]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    pub channels: u16,
    pub rate: u32,
    pub samples: Vec<i16>,
}

impl Pcm {
    /// Sample frames — one sample per channel each.
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }

    /// The same sound as a RIFF WAVE file: 16-bit little-endian PCM
    /// (`WAVE_FORMAT_PCM`), the one format every WAV player reads.
    pub fn to_wav(&self) -> Vec<u8> {
        let data_len = (self.samples.len() * 2) as u32;
        let block_align = self.channels * 2;
        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk length
        out.extend_from_slice(&1u16.to_le_bytes()); // WAVE_FORMAT_PCM
        out.extend_from_slice(&self.channels.to_le_bytes());
        out.extend_from_slice(&self.rate.to_le_bytes());
        out.extend_from_slice(&(self.rate * u32::from(block_align)).to_le_bytes()); // byte rate
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for s in &self.samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }
}

/// The Z-machine's high bleep, sound 1 (ZMSD §15), as lanthorn plays it.
pub const HIGH_BLEEP_HZ: f32 = 800.0;
/// The Z-machine's low bleep, sound 2 (ZMSD §15).
pub const LOW_BLEEP_HZ: f32 = 400.0;
/// How long either bleep sounds.
pub const BLEEP_MS: u32 = 150;

/// A short decaying sine at `freq_hz` for `ms`, as mono 44100 Hz PCM at full
/// scale — the tone [`AudioBackend::play_tone`] plays, before volume.
pub fn tone(freq_hz: f32, ms: u32) -> Pcm {
    let samples = synth_tone(freq_hz, ms)
        .into_iter()
        .map(|v| (v.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)
        .collect();
    Pcm { channels: 1, rate: SAMPLE_RATE, samples }
}

/// The Z-machine bleep `number` — 1 high, 2 low (ZMSD §15) — or `None` for any
/// other sound number, which is a sampled resource rather than a tone.
pub fn bleep(number: u16) -> Option<Pcm> {
    match number {
        1 => Some(tone(HIGH_BLEEP_HZ, BLEEP_MS)),
        2 => Some(tone(LOW_BLEEP_HZ, BLEEP_MS)),
        _ => None,
    }
}

/// Render a ProTracker MOD to PCM (interleaved stereo, 44100 Hz), playing the
/// song through once — or stopping after `max_frames` frames, for a module that
/// loops on itself. `None` for a malformed module.
#[cfg(feature = "mod-music")]
pub fn render_mod(bytes: &[u8], max_frames: Option<usize>) -> Option<Pcm> {
    let source = mod_stream::ModSource::new(bytes, false, 1)?;
    let samples: Vec<i16> = match max_frames {
        Some(n) => source.take(n.saturating_mul(2)).collect(),
        None => source.collect(),
    };
    Some(Pcm { channels: 2, rate: SAMPLE_RATE, samples })
}

/// Master+Z-scale gain in 0.0..=1.0. Master is 0..=100; z_volume is the Z-machine
/// 1..=8 scale, with 0/255 meaning "loudest" (full).
#[cfg_attr(not(feature = "playback"), allow(dead_code))]
fn gain(master: u8, z_volume: u8) -> f32 {
    (master.min(100) as f32 / 100.0)
        * match z_volume {
            0 | 255 => 1.0,
            v => (v.min(8) as f32) / 8.0,
        }
}

/// A short decaying sine at `freq_hz` for `ms` at 44100 Hz (unit amplitude,
/// linear decay envelope). Volume is applied by the caller via the sink.
fn synth_tone(freq_hz: f32, ms: u32) -> Vec<f32> {
    let n = ((ms as f32 / 1000.0) * SAMPLE_RATE as f32) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let env = (1.0 - (i as f32 / n.max(1) as f32)).max(0.0);
        let s = (2.0 * std::f32::consts::PI * freq_hz * t).sin();
        out.push(s * env);
    }
    out
}

/// Decode a 10-byte 80-bit IEEE-754 extended float (AIFF sample rate) to u32 Hz.
/// Layout: 1 sign bit, 15 exponent bits (bias 16383), 64 mantissa bits with an
/// explicit integer bit. value = mantissa * 2^(exponent - 16383 - 63).
fn extended80_to_u32(b: &[u8; 10]) -> u32 {
    let exponent = (((b[0] & 0x7F) as u32) << 8) | b[1] as u32;
    let mantissa = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
    if exponent == 0 && mantissa == 0 {
        return 0;
    }
    let e = exponent as i32 - 16383 - 63;
    let mut val = mantissa as f64;
    if e >= 0 {
        val *= 2f64.powi(e);
    } else {
        val /= 2f64.powi(-e);
    }
    val as u32
}

/// Decode an IFF `FORM`/`AIFF` (or `AIFC`) container — a Blorb `Snd ` resource,
/// an Infocom disk sample (`blorb::infocom_sound::InfocomSound::to_aiff`) — into
/// [`Pcm`]. 8-bit samples are widened to 16. `None` on a malformed AIFF.
pub fn decode_aiff(bytes: &[u8]) -> Option<Pcm> {
    // `Blorb::sound` returns an AIFF resource as a full `FORM`...`AIFF`/`AIFC`
    // container (the `FORM`+len header is part of the resource per the Blorb
    // spec). Accept that, and also a bare payload starting directly at the form
    // type `AIFF`/`AIFC` (a headerless slice or a legacy caller).
    let mut pos = if bytes.len() >= 12
        && &bytes[0..4] == b"FORM"
        && (&bytes[8..12] == b"AIFF" || &bytes[8..12] == b"AIFC")
    {
        12
    } else if bytes.len() >= 4 && (&bytes[0..4] == b"AIFF" || &bytes[0..4] == b"AIFC") {
        4
    } else {
        return None;
    };
    let mut channels: u16 = 0;
    let mut sample_rate: u32 = 0;
    let mut sample_size: u16 = 0;
    // Pass 1: locate COMM. AIFF-1.3 places "no restrictions on the ordering of
    // local chunks within a FORM AIFF", so SSND may legally precede COMM — the
    // sample format must be known before any sound data is decoded, or a
    // 16-bit SSND-first file falls through the 8-bit branch as double-length
    // static (SQ-0630).
    let mut scan = pos;
    while scan + 8 <= bytes.len() {
        let id = [bytes[scan], bytes[scan + 1], bytes[scan + 2], bytes[scan + 3]];
        let len = u32::from_be_bytes([bytes[scan + 4], bytes[scan + 5], bytes[scan + 6], bytes[scan + 7]]) as usize;
        let data_start = scan + 8;
        if data_start + len > bytes.len() {
            break;
        }
        if &id == b"COMM" && len >= 18 {
            channels = u16::from_be_bytes([bytes[data_start], bytes[data_start + 1]]);
            sample_size = u16::from_be_bytes([bytes[data_start + 6], bytes[data_start + 7]]);
            let mut ext = [0u8; 10];
            ext.copy_from_slice(&bytes[data_start + 8..data_start + 18]);
            sample_rate = extended80_to_u32(&ext);
            break;
        }
        scan = data_start + len + (len & 1);
    }
    if channels == 0 || sample_rate == 0 {
        return None;
    }
    // Pass 2: decode SSND with the format known.
    let mut pcm: Vec<i16> = Vec::new();
    while pos + 8 <= bytes.len() {
        let id = [bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]];
        let len = u32::from_be_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]) as usize;
        let data_start = pos + 8;
        if data_start + len > bytes.len() {
            break;
        }
        if &id == b"SSND" && len >= 8 {
            // SSND layout (AIFF-1.3): offset u32, blockSize u32, soundData.
            // `offset` is the byte position within soundData where the first
            // sample frame starts (block-aligned writers use it) — honor it
            // rather than assuming 0 (SQ-0630). blockSize only describes the
            // writer's alignment and is not needed to read the frames.
            let offset = u32::from_be_bytes([
                bytes[data_start],
                bytes[data_start + 1],
                bytes[data_start + 2],
                bytes[data_start + 3],
            ]) as usize;
            let pcm_end = data_start + len;
            let mut i = (data_start + 8).saturating_add(offset);
            if sample_size <= 8 {
                while i < pcm_end {
                    let s = bytes[i] as i8;
                    pcm.push((s as i16) << 8);
                    i += 1;
                }
            } else {
                let bps = (sample_size as usize).div_ceil(8).max(2);
                while i + 1 < pcm_end {
                    pcm.push(i16::from_be_bytes([bytes[i], bytes[i + 1]]));
                    i += bps;
                }
            }
        }
        pos = data_start + len + (len & 1);
    }
    if pcm.is_empty() {
        return None;
    }
    Some(Pcm { channels, rate: sample_rate, samples: pcm })
}

/// A lazy, streaming ProTracker MOD source. Wraps an `XmrsPlayer` that renders
/// interleaved stereo `i16` samples on demand (one per `next()`), so playback
/// starts immediately instead of pre-rendering the whole track. `rodio` pulls
/// samples on its own audio thread, so building and appending this never blocks
/// the caller.
///
/// `XmrsPlayer<'a>` borrows the `Module`, but a `rodio::Source` appended to a
/// `Sink` must be `Send + 'static`. `self_cell` lets us own the `Module` and the
/// `XmrsPlayer` that borrows it in one `Send + 'static` value without leaking.
#[cfg(feature = "mod-music")]
pub mod mod_stream {
    use super::SAMPLE_RATE;
    use xmrs::prelude::Module;
    use xmrsplayer::prelude::XmrsPlayer;

    self_cell::self_cell!(
        struct PlayerCell {
            owner: Module,
            #[not_covariant]
            dependent: XmrsPlayer,
        }
    );

    pub struct ModSource {
        cell: PlayerCell,
    }

    impl ModSource {
        /// Parse `bytes` and build a streaming source. Returns `None` on a
        /// malformed module (`Module::load` returns `Err`) — no panic, no
        /// rendering. `forever` loops indefinitely (`set_max_loop_count(0)`);
        /// otherwise the song plays `count` times then the iterator ends.
        pub fn new(bytes: &[u8], forever: bool, count: u8) -> Option<Self> {
            let module = Module::load(bytes).ok()?;
            let cell = PlayerCell::new(module, |m| {
                let mut player = XmrsPlayer::new(m, SAMPLE_RATE, 0);
                player.set_max_loop_count(if forever { 0 } else { count as usize });
                player
            });
            Some(ModSource { cell })
        }
    }

    // `XmrsPlayer`'s iterator yields interleaved stereo `i16` samples — the same
    // values the old `render_mod` collected into a `Vec<i16>`.
    impl Iterator for ModSource {
        type Item = i16;
        fn next(&mut self) -> Option<i16> {
            self.cell.with_dependent_mut(|_owner, player| player.next())
        }
    }

    #[cfg(feature = "playback")]
    impl rodio::Source for ModSource {
        fn current_frame_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> u16 {
            2
        }
        fn sample_rate(&self) -> u32 {
            SAMPLE_RATE
        }
        fn total_duration(&self) -> Option<std::time::Duration> {
            None
        }
    }
}

/// Map the Z-machine repeat count to (loop_forever, finite_play_count).
/// 255 = forever; 0 (or omitted, which the engine records as 0) = play once;
/// 1..=254 = that many plays. (Matches de-facto interpreter behavior.)
#[cfg_attr(not(feature = "playback"), allow(dead_code))]
fn repeat_plan(repeats: u8) -> (bool, u8) {
    match repeats {
        255 => (true, 1),
        0 => (false, 1),
        n => (false, n),
    }
}

// ── Real backend (playback feature on) ────────────────────────────────────────

/// Per-sample volume: either the Z-machine 1..=8 scale (`0`/`255` = full) or a
/// linear pre-master gain fraction (the Glk channel model). Stored per playing
/// sample so a later master-volume change re-applies the correct formula.
#[cfg_attr(not(feature = "playback"), allow(dead_code))]
#[derive(Clone, Copy)]
enum SampleVol {
    Z(u8),
    Lin(f32),
}

/// Final pre-output gain for a sample: `master/100` times the sample's own level.
#[cfg_attr(not(feature = "playback"), allow(dead_code))]
fn vol_gain(master: u8, v: SampleVol) -> f32 {
    match v {
        SampleVol::Z(z) => gain(master, z),
        SampleVol::Lin(f) => (master.min(100) as f32 / 100.0) * f.max(0.0),
    }
}

#[cfg(feature = "playback")]
pub struct AudioBackend {
    stream: Option<(rodio::OutputStream, rodio::OutputStreamHandle)>,
    samples: std::collections::HashMap<SoundId, (rodio::Sink, SampleVol)>,
    tones: Vec<(rodio::Sink, u8)>,
    next_id: SoundId,
    master: u8,
}

#[cfg(feature = "playback")]
impl std::fmt::Debug for AudioBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioBackend").finish_non_exhaustive()
    }
}

/// When set, `AudioBackend::new` skips opening a real output device and returns
/// a silent backend. Purely for test suites: opening (and, on macOS, tearing
/// down) a real CoreAudio stream costs many seconds per backend, and a test that
/// constructs one blocks the whole test binary at drop. Never touched in
/// production — `main` never calls `disable_output_for_tests`.
#[cfg(feature = "playback")]
static TEST_SILENCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Make every subsequently-constructed `AudioBackend` silent (no real device).
/// Call this at the top of any test that constructs a backend — directly or via
/// a production path — so it neither opens nor tears down an audio device.
#[cfg(feature = "playback")]
pub fn disable_output_for_tests() {
    TEST_SILENCE.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(feature = "playback")]
impl AudioBackend {
    pub fn new(volume: u8) -> AudioBackend {
        let stream = if TEST_SILENCE.load(std::sync::atomic::Ordering::Relaxed) {
            None
        } else {
            match rodio::OutputStream::try_default() {
                Ok((s, h)) => Some((s, h)),
                Err(e) => {
                    eprintln!("audio: no output device ({e}); sound disabled");
                    None
                }
            }
        };
        AudioBackend {
            stream,
            samples: std::collections::HashMap::new(),
            tones: Vec::new(),
            next_id: 1,
            master: volume.min(100),
        }
    }

    pub fn play_tone(&mut self, freq_hz: f32, ms: u32, z_volume: u8) {
        let Some((_, handle)) = &self.stream else { return };
        let Ok(sink) = rodio::Sink::try_new(handle) else { return };
        sink.set_volume(gain(self.master, z_volume));
        sink.append(rodio::buffer::SamplesBuffer::new(1, SAMPLE_RATE, synth_tone(freq_hz, ms)));
        self.tones.push((sink, z_volume));
    }

    /// Build a sink that will play `bytes` (decoded per `format`) `repeats` times.
    /// Volume is NOT set here — the caller applies it. Returns None if there is
    /// no device, the format is unsupported, or decode fails.
    fn build_sample_sink(&self, bytes: &[u8], format: SoundFormat, repeats: u8) -> Option<rodio::Sink> {
        use rodio::Source;
        let (_, handle) = self.stream.as_ref()?;
        let sink = rodio::Sink::try_new(handle).ok()?;
        let (forever, count) = repeat_plan(repeats);
        match format {
            SoundFormat::Aiff => {
                let Pcm { channels, rate, samples: pcm } = decode_aiff(bytes)?;
                if forever {
                    sink.append(rodio::buffer::SamplesBuffer::new(channels, rate, pcm.clone()).repeat_infinite());
                } else {
                    for _ in 0..count {
                        sink.append(rodio::buffer::SamplesBuffer::new(channels, rate, pcm.clone()));
                    }
                }
            }
            SoundFormat::Ogg => {
                if forever {
                    let dec = rodio::Decoder::new(std::io::Cursor::new(bytes.to_vec())).ok()?;
                    sink.append(dec.repeat_infinite());
                } else {
                    for _ in 0..count {
                        if let Ok(dec) = rodio::Decoder::new(std::io::Cursor::new(bytes.to_vec())) {
                            sink.append(dec);
                        }
                    }
                }
            }
            SoundFormat::Mod => {
                #[cfg(feature = "mod-music")]
                {
                    let source = mod_stream::ModSource::new(bytes, forever, count)?;
                    sink.append(source);
                }
                #[cfg(not(feature = "mod-music"))]
                {
                    eprintln!("audio: unsupported sound format (MOD; mod-music feature off)");
                    return None;
                }
            }
        }
        Some(sink)
    }

    /// Decode `bytes` per `format`, play on a fresh sink at gain(master, z_volume),
    /// looping per `repeats` (see `repeat_plan`: 255 = forever, 0/omitted = once).
    /// Returns a SoundId to `stop`/track.
    /// Returns None if there is no device, the format is unsupported, or decode fails.
    pub fn play_sample(&mut self, bytes: &[u8], format: SoundFormat, z_volume: u8, repeats: u8) -> Option<SoundId> {
        let sink = self.build_sample_sink(bytes, format, repeats)?;
        sink.set_volume(vol_gain(self.master, SampleVol::Z(z_volume)));
        let id = self.next_id;
        self.next_id += 1;
        self.samples.insert(id, (sink, SampleVol::Z(z_volume)));
        Some(id)
    }

    /// Like `play_sample`, but with a linear pre-master `gain` fraction (the Glk
    /// channel volume model) instead of the Z-machine z-scale.
    pub fn play_sample_gain(&mut self, bytes: &[u8], format: SoundFormat, gain: f32, repeats: u8) -> Option<SoundId> {
        let sink = self.build_sample_sink(bytes, format, repeats)?;
        sink.set_volume(vol_gain(self.master, SampleVol::Lin(gain)));
        let id = self.next_id;
        self.next_id += 1;
        self.samples.insert(id, (sink, SampleVol::Lin(gain)));
        Some(id)
    }

    /// Set a live sample's linear pre-master gain (Glk `schannel_set_volume`).
    pub fn set_sample_gain(&mut self, id: SoundId, gain: f32) {
        if let Some((sink, v)) = self.samples.get_mut(&id) {
            *v = SampleVol::Lin(gain);
            sink.set_volume(vol_gain(self.master, SampleVol::Lin(gain)));
        }
    }

    pub fn stop(&mut self, id: SoundId) {
        if let Some((sink, _)) = self.samples.remove(&id) {
            sink.stop();
        }
    }

    /// Pause a live sample, retaining its position (Glk `schannel_pause`). A
    /// paused sink is not "empty", so `finished()` never reports it as done.
    pub fn pause(&mut self, id: SoundId) {
        if let Some((sink, _)) = self.samples.get(&id) {
            sink.pause();
        }
    }

    /// Resume a paused sample from where it left off (Glk `schannel_unpause`).
    pub fn unpause(&mut self, id: SoundId) {
        if let Some((sink, _)) = self.samples.get(&id) {
            sink.play();
        }
    }

    pub fn stop_all(&mut self) {
        for (_, (sink, _)) in self.samples.drain() {
            sink.stop();
        }
        for (sink, _) in self.tones.drain(..) {
            sink.stop();
        }
    }

    pub fn set_volume(&mut self, volume: u8) {
        self.master = volume.min(100);
        for (s, v) in self.samples.values() {
            s.set_volume(vol_gain(self.master, *v));
        }
        for (s, z_volume) in &self.tones {
            s.set_volume(gain(self.master, *z_volume));
        }
    }

    /// Drain completed sample ids (whose sink is empty) and prune finished tones.
    pub fn finished(&mut self) -> Vec<SoundId> {
        self.tones.retain(|(s, _)| !s.empty());
        let done: Vec<SoundId> = self
            .samples
            .iter()
            .filter(|(_, (s, _))| s.empty())
            .map(|(id, _)| *id)
            .collect();
        for id in &done {
            self.samples.remove(id);
        }
        done
    }
}

// ── No-op backend (playback feature off) ──────────────────────────────────────

#[cfg(not(feature = "playback"))]
#[derive(Debug)]
pub struct AudioBackend;

#[cfg(not(feature = "playback"))]
impl AudioBackend {
    pub fn new(_volume: u8) -> AudioBackend { AudioBackend }
    pub fn play_tone(&mut self, _freq_hz: f32, _ms: u32, _z_volume: u8) {}
    pub fn play_sample(&mut self, _bytes: &[u8], _format: SoundFormat, _z_volume: u8, _repeats: u8) -> Option<SoundId> { None }
    pub fn play_sample_gain(&mut self, _bytes: &[u8], _format: SoundFormat, _gain: f32, _repeats: u8) -> Option<SoundId> { None }
    pub fn set_sample_gain(&mut self, _id: SoundId, _gain: f32) {}
    pub fn pause(&mut self, _id: SoundId) {}
    pub fn unpause(&mut self, _id: SoundId) {}
    pub fn stop(&mut self, _id: SoundId) {}
    pub fn stop_all(&mut self) {}
    pub fn set_volume(&mut self, _volume: u8) {}
    pub fn finished(&mut self) -> Vec<SoundId> { Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vol_gain_linear_combines_with_master() {
        // Linear (Glk) per-sample gain multiplies master/100, independent of the
        // z-scale path.
        assert_eq!(vol_gain(100, SampleVol::Lin(1.0)), 1.0);
        assert_eq!(vol_gain(50, SampleVol::Lin(1.0)), 0.5);
        assert_eq!(vol_gain(100, SampleVol::Lin(0.5)), 0.5);
        assert_eq!(vol_gain(0, SampleVol::Lin(1.0)), 0.0);
    }

    #[test]
    fn vol_gain_z_matches_legacy_gain() {
        // The z-scale variant must equal the historical gain() for every input,
        // so the Z-machine path is byte-for-byte unchanged.
        for master in [0u8, 25, 50, 100] {
            for z in [0u8, 1, 4, 8, 255] {
                assert_eq!(vol_gain(master, SampleVol::Z(z)), gain(master, z));
            }
        }
    }

    #[test]
    fn gain_combines_master_and_z_volume() {
        assert_eq!(gain(100, 8), 1.0);        // full master, loudest z-scale
        assert_eq!(gain(50, 8), 0.5);         // half master
        assert_eq!(gain(100, 4), 0.5);        // z 4/8
        assert_eq!(gain(100, 0), 1.0);        // 0 -> treated as full
        assert_eq!(gain(100, 255), 1.0);      // 255 -> loudest
        assert_eq!(gain(0, 8), 0.0);          // muted master
    }

    #[test]
    fn repeat_plan_maps_counts() {
        assert_eq!(repeat_plan(0), (false, 1));
        assert_eq!(repeat_plan(255), (true, 1));
        assert_eq!(repeat_plan(5), (false, 5));
        assert_eq!(repeat_plan(1), (false, 1));
    }

    #[test]
    fn synth_tone_has_expected_length_and_energy() {
        let s = synth_tone(440.0, 100); // 100ms @ 44100 = 4410 samples
        assert_eq!(s.len(), 4410, "length = ms/1000 * 44100");
        assert!(s.iter().any(|v| v.abs() > 0.1), "tone must not be silent");
        // Decaying envelope: the first quarter is louder than the last quarter.
        let peak_early = s[..1000].iter().fold(0.0_f32, |a, v| a.max(v.abs()));
        let peak_late = s[3410..].iter().fold(0.0_f32, |a, v| a.max(v.abs()));
        assert!(peak_early > peak_late, "amplitude decays over time");
    }

    #[cfg(feature = "playback")]
    #[test]
    fn set_volume_preserves_per_sound_z_scale() {
        // A sound played at a reduced z_volume must keep its z-scale relationship
        // to master after a runtime volume change — set_volume must apply
        // gain(master, z_volume), not bare master/100.
        // Backend may have no device in CI; assert on the pure gain relationship
        // that set_volume now uses, so the test is deterministic without a sink:
        let quiet = gain(50, 4); // half master, mid z-scale
        let loud = gain(50, 8); // half master, full z-scale
        assert!(quiet < loud, "z_volume must still scale after a master change");
        assert_eq!(gain(50, 8), 0.5, "full z-scale at half master == master/100");
    }

    #[test]
    fn backend_no_device_paths_never_panic() {
        // The whole point is the no-device path — so force it, rather than open
        // (and slowly tear down) a real device on a machine that has one. (The
        // no-op backend without `playback` has no device to skip.)
        #[cfg(feature = "playback")]
        disable_output_for_tests();
        // Constructing a backend must succeed even with no output device (CI).
        let mut b = AudioBackend::new(100);
        b.play_tone(800.0, 50, 8);
        b.set_volume(50);
        b.stop(1);
        b.stop_all();
        let _ = b.finished();
    }

    /// Build a tiny 1-channel, 16-bit, 44100 Hz AIFF with two PCM frames.
    fn tiny_aiff() -> Vec<u8> {
        fn be_chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(id);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 { v.push(0); }
            v
        }
        // COMM: channels=1, numFrames=2, sampleSize=16, rate = 44100 as 80-bit ext.
        let mut comm = Vec::new();
        comm.extend_from_slice(&1u16.to_be_bytes());       // channels
        comm.extend_from_slice(&2u32.to_be_bytes());       // numSampleFrames
        comm.extend_from_slice(&16u16.to_be_bytes());      // sampleSize
        comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]); // 44100
        // SSND: offset=0, blockSize=0, then two BE i16 samples: 256, -256.
        let mut ssnd = Vec::new();
        ssnd.extend_from_slice(&0u32.to_be_bytes());       // offset
        ssnd.extend_from_slice(&0u32.to_be_bytes());       // blockSize
        ssnd.extend_from_slice(&256i16.to_be_bytes());
        ssnd.extend_from_slice(&(-256i16).to_be_bytes());

        let mut body = Vec::new();
        body.extend_from_slice(b"AIFF");
        body.extend_from_slice(&be_chunk(b"COMM", &comm));
        body.extend_from_slice(&be_chunk(b"SSND", &ssnd));

        let mut form = Vec::new();
        form.extend_from_slice(b"FORM");
        form.extend_from_slice(&(body.len() as u32).to_be_bytes());
        form.extend_from_slice(&body);
        form
    }

    #[test]
    fn extended80_decodes_44100() {
        assert_eq!(extended80_to_u32(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]), 44100);
        assert_eq!(extended80_to_u32(&[0; 10]), 0);
    }

    #[test]
    fn decode_aiff_parses_comm_and_ssnd() {
        let Pcm { channels, rate, samples: pcm } = decode_aiff(&tiny_aiff()).expect("valid AIFF");
        assert_eq!(channels, 1);
        assert_eq!(rate, 44100);
        assert_eq!(pcm, vec![256i16, -256i16]);
    }

    /// SQ-0630: AIFF-1.3 places "no restrictions on the ordering of local
    /// chunks", so SSND may legally precede COMM. The decoder must locate COMM
    /// first — otherwise sample_size is still 0 when SSND is met and 16-bit
    /// data decodes through the 8-bit branch as double-length static.
    #[test]
    fn decode_aiff_handles_ssnd_before_comm() {
        fn be_chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(id);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 { v.push(0); }
            v
        }
        let mut comm = Vec::new();
        comm.extend_from_slice(&1u16.to_be_bytes());
        comm.extend_from_slice(&2u32.to_be_bytes());
        comm.extend_from_slice(&16u16.to_be_bytes());
        comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]); // 44100
        let mut ssnd = Vec::new();
        ssnd.extend_from_slice(&0u32.to_be_bytes());
        ssnd.extend_from_slice(&0u32.to_be_bytes());
        ssnd.extend_from_slice(&256i16.to_be_bytes());
        ssnd.extend_from_slice(&(-256i16).to_be_bytes());

        let mut body = Vec::new();
        body.extend_from_slice(b"AIFF");
        body.extend_from_slice(&be_chunk(b"SSND", &ssnd)); // SSND FIRST
        body.extend_from_slice(&be_chunk(b"COMM", &comm));
        let mut form = Vec::new();
        form.extend_from_slice(b"FORM");
        form.extend_from_slice(&(body.len() as u32).to_be_bytes());
        form.extend_from_slice(&body);

        let Pcm { channels, rate, samples: pcm } = decode_aiff(&form).expect("SSND-first AIFF is legal");
        assert_eq!(channels, 1);
        assert_eq!(rate, 44100);
        assert_eq!(pcm, vec![256i16, -256i16], "16-bit data decoded as 16-bit");
    }

    /// SQ-0630: the SSND `offset` field is the byte position within soundData
    /// where the first sample frame starts (AIFF-1.3 Sound Data Chunk) — a
    /// block-aligning writer pads the front, and those pad bytes must be
    /// skipped, not decoded as samples.
    #[test]
    fn decode_aiff_honors_nonzero_ssnd_offset() {
        fn be_chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(id);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 { v.push(0); }
            v
        }
        let mut comm = Vec::new();
        comm.extend_from_slice(&1u16.to_be_bytes());
        comm.extend_from_slice(&2u32.to_be_bytes());
        comm.extend_from_slice(&16u16.to_be_bytes());
        comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]); // 44100
        // SSND: offset=2 -> two alignment-pad bytes precede the real frames.
        let mut ssnd = Vec::new();
        ssnd.extend_from_slice(&2u32.to_be_bytes());       // offset
        ssnd.extend_from_slice(&0u32.to_be_bytes());       // blockSize
        ssnd.extend_from_slice(&[0xEE, 0xEE]);             // pad bytes (not samples)
        ssnd.extend_from_slice(&256i16.to_be_bytes());
        ssnd.extend_from_slice(&(-256i16).to_be_bytes());

        let mut body = Vec::new();
        body.extend_from_slice(b"AIFF");
        body.extend_from_slice(&be_chunk(b"COMM", &comm));
        body.extend_from_slice(&be_chunk(b"SSND", &ssnd));
        let mut form = Vec::new();
        form.extend_from_slice(b"FORM");
        form.extend_from_slice(&(body.len() as u32).to_be_bytes());
        form.extend_from_slice(&body);

        let pcm = decode_aiff(&form).expect("valid AIFF with offset").samples;
        assert_eq!(pcm, vec![256i16, -256i16], "the offset pad bytes are skipped");
    }

    /// Build a blorb-shaped AIFF payload: the FORM chunk's PAYLOAD as `Blorb::sound`
    /// actually hands to `decode_aiff` — starts with the form type `AIFF` (no
    /// leading `FORM`+len header), COMM is 8-bit mono, SSND holds signed 8-bit
    /// samples. Reuses the same 80-bit extended sample-rate bytes as `tiny_aiff()`.
    fn blorb_aiff_payload_8bit() -> Vec<u8> {
        fn be_chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(id);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 { v.push(0); }
            v
        }
        // COMM: channels=1, numFrames=3, sampleSize=8, rate = 44100 as 80-bit ext
        // (same extended-float bytes tiny_aiff() uses).
        let mut comm = Vec::new();
        comm.extend_from_slice(&1u16.to_be_bytes());       // channels
        comm.extend_from_slice(&3u32.to_be_bytes());       // numSampleFrames
        comm.extend_from_slice(&8u16.to_be_bytes());       // sampleSize
        comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]); // 44100
        // SSND: offset=0, blockSize=0, then three signed 8-bit samples.
        let mut ssnd = Vec::new();
        ssnd.extend_from_slice(&0u32.to_be_bytes());       // offset
        ssnd.extend_from_slice(&0u32.to_be_bytes());       // blockSize
        ssnd.extend_from_slice(&[0x7f, 0x80, 0x00]);       // i8: +127, -128, 0

        let mut payload = Vec::new();
        payload.extend_from_slice(b"AIFF");
        payload.extend_from_slice(&be_chunk(b"COMM", &comm));
        payload.extend_from_slice(&be_chunk(b"SSND", &ssnd));
        payload
    }

    #[test]
    fn decode_aiff_accepts_blorb_payload_8bit() {
        let Pcm { channels, rate, samples: pcm } =
            decode_aiff(&blorb_aiff_payload_8bit()).expect("valid blorb AIFF payload");
        assert_eq!(channels, 1);
        assert_eq!(rate, 44100);
        assert_eq!(pcm, vec![32512i16, -32768i16, 0i16]);
    }

    #[cfg(not(feature = "playback"))]
    #[test]
    fn play_sample_returns_none_without_playback() {
        let mut b = AudioBackend::new(100);
        assert!(b.play_sample(&tiny_aiff(), SoundFormat::Aiff, 8, 1).is_none());
    }

    /// Smallest structurally-valid 4-channel ("M.K.") ProTracker MOD: 1084-byte
    /// header (20-byte title, 31 * 30-byte sample records, songlen, restart,
    /// 128-byte order table, "M.K."), then one 1024-byte (64-row * 4-ch * 4-byte)
    /// zero pattern, and no sample data (all sample lengths are 0). It is silent,
    /// so `render_mod` asserts structure (channels == 2, correct rate) and a
    /// non-empty rendered buffer rather than any particular audio content.
    #[cfg(feature = "mod-music")]
    fn minimal_mod() -> Vec<u8> {
        let mut v = vec![0u8; 20];          // title
        v.extend(std::iter::repeat_n(0u8, 31 * 30)); // 31 sample records
        v.push(1);                          // song length = 1 pattern in the order
        v.push(127);                        // restart position
        v.extend(std::iter::repeat_n(0u8, 128)); // order table (pattern 0)
        v.extend_from_slice(b"M.K.");       // 4-channel tag
        v.extend(std::iter::repeat_n(0u8, 64 * 4 * 4)); // one zero pattern
        v
    }

    #[cfg(feature = "mod-music")]
    #[test]
    fn mod_source_rejects_malformed_without_panic() {
        use mod_stream::ModSource;
        assert!(ModSource::new(&[0u8; 100], false, 1).is_none(), "short input rejected, not panicked on");
        assert!(ModSource::new(&[], false, 1).is_none(), "empty input rejected, not panicked on");
    }

    #[cfg(all(feature = "mod-music", feature = "playback"))]
    #[test]
    fn mod_source_reports_stereo_format() {
        use rodio::Source;
        let source = mod_stream::ModSource::new(&minimal_mod(), false, 1).expect("valid minimal MOD");
        assert_eq!(source.channels(), 2, "MOD is streamed as stereo");
        assert_eq!(source.sample_rate(), SAMPLE_RATE);
        assert!(source.current_frame_len().is_none());
        assert!(source.total_duration().is_none());
    }

    #[cfg(feature = "mod-music")]
    #[test]
    fn mod_source_streams_lazily_and_terminates() {
        use mod_stream::ModSource;
        // A first sample is available immediately (lazy: no full pre-render).
        let mut src = ModSource::new(&minimal_mod(), false, 1).expect("valid minimal MOD");
        assert!(src.next().is_some(), "source yields at least a first sample");
        // With a finite loop count, collecting the whole stream terminates and
        // is non-empty (mirrors the old render_mod_produces_stereo_buffer guarantee).
        let src = ModSource::new(&minimal_mod(), false, 1).expect("valid minimal MOD");
        let pcm: Vec<i16> = src.collect();
        assert!(!pcm.is_empty(), "one pattern still yields frames");
    }

    // ── SQ-1541: decode with no device ─────────────────────────────────────────

    /// Parse a RIFF WAVE of 16-bit PCM back into [`Pcm`] — the reader the
    /// round-trip below checks `to_wav` against, written from the RIFF/WAVE
    /// layout rather than from the encoder.
    fn read_wav(w: &[u8]) -> Pcm {
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, w.len() - 8, "RIFF size");
        assert_eq!(&w[8..12], b"WAVE");
        assert_eq!(&w[12..16], b"fmt ");
        assert_eq!(u32::from_le_bytes(w[16..20].try_into().unwrap()), 16);
        assert_eq!(u16::from_le_bytes([w[20], w[21]]), 1, "PCM");
        let channels = u16::from_le_bytes([w[22], w[23]]);
        let rate = u32::from_le_bytes(w[24..28].try_into().unwrap());
        let byte_rate = u32::from_le_bytes(w[28..32].try_into().unwrap());
        let block_align = u16::from_le_bytes([w[32], w[33]]);
        assert_eq!(u16::from_le_bytes([w[34], w[35]]), 16, "16-bit");
        assert_eq!(block_align, channels * 2);
        assert_eq!(byte_rate, rate * u32::from(block_align));
        assert_eq!(&w[36..40], b"data");
        let len = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
        assert_eq!(len, w.len() - 44);
        let samples = w[44..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect();
        Pcm { channels, rate, samples }
    }

    #[test]
    fn a_tone_is_mono_44100_pcm_of_the_length_asked_for() {
        let t = tone(440.0, 100);
        assert_eq!((t.channels, t.rate), (1, 44100));
        assert_eq!(t.frames(), 4410, "100ms at 44100 Hz");
        assert!(t.samples.iter().any(|s| s.unsigned_abs() > 3000), "a tone, not silence");
        let high = bleep(1).expect("sound 1 is the high bleep");
        let low = bleep(2).expect("sound 2 is the low bleep");
        assert_eq!(high.frames(), (BLEEP_MS as usize) * 441 / 10);
        assert_eq!(high.frames(), low.frames(), "both bleeps last as long");
        assert_ne!(high.samples, low.samples, "at different pitches");
        assert!(bleep(3).is_none(), "3 and up are sampled resources, not tones");
    }

    #[test]
    fn an_aiff_round_trips_through_the_wav_encoder() {
        for aiff in [tiny_aiff(), blorb_aiff_payload_8bit()] {
            let pcm = decode_aiff(&aiff).expect("valid AIFF");
            let wav = pcm.to_wav();
            assert_eq!(wav.len(), 44 + pcm.samples.len() * 2);
            assert_eq!(read_wav(&wav), pcm, "WAV carries exactly the decoded samples");
        }
        let tone = tone(800.0, 20);
        assert_eq!(read_wav(&tone.to_wav()), tone, "and a synthesized tone");
    }

    /// One 64-row pattern at ProTracker's defaults — speed 6 ticks a row, 125
    /// BPM, so a tick is 2.5 / 125 s = 20 ms, 882 frames at 44100 Hz — is
    /// 64 × 6 × 882 frames of stereo.
    #[cfg(feature = "mod-music")]
    #[test]
    fn a_mod_renders_to_the_frames_its_song_lasts() {
        let pcm = render_mod(&minimal_mod(), None).expect("valid minimal MOD");
        assert_eq!((pcm.channels, pcm.rate), (2, 44100));
        assert_eq!(pcm.frames(), 64 * 6 * 882, "one pattern at the default speed and tempo");
        let capped = render_mod(&minimal_mod(), Some(1000)).expect("valid minimal MOD");
        assert_eq!(capped.frames(), 1000, "a cap stops the render");
        assert!(render_mod(&[0u8; 100], None).is_none(), "a malformed module renders nothing");
        assert_eq!(read_wav(&capped.to_wav()), capped);
    }
}
