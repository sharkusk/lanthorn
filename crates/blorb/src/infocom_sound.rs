//! Infocom's own sampled-sound files, as they sit on a release disk (SQ-0907).
//!
//! Two Infocom games use sound: *The Lurking Horror* and *Sherlock*. On the Amiga
//! their release disks carry a `Sound/` directory holding one THREE-file set per
//! Z-machine effect number — `sN.dat` (the sample), `sN.mid` (eleven bytes of
//! MIDI-shaped Note-On/Note-Off giving the pitch it is played at) and `sN.nam` (a
//! manifest naming the other two). The numbering is the standard's own: ZMSD §9
//! reserves effects 1 and 2 for the interpreter's bleeps, so a game's sounds start
//! at 3.
//!
//! # The `.dat` container
//!
//! A ten-byte big-endian header, then 8-bit mono PCM — signed here, offset binary on
//! the Macintosh, which the header does not say and [`Encoding`] explains:
//!
//! | offset | field |
//! |---|---|
//! | 0 | byte count of everything after this field — the file's length less 2 |
//! | 2 | repeat count — Paula's `ioa_Cycles`; 0 or 1 on every file |
//! | 3 | the note this sample is recorded at; 50 or 60 on every file |
//! | 4 | sample rate in Hz |
//! | 6 | zero on every file measured |
//! | 8 | frame count |
//! | 10 | `frames` bytes of samples |
//!
//! Bytes 2 and 3 were recorded as one opaque `flags` word until SQ-0923 read the
//! interpreter and found it writing the first into the audio request and comparing the
//! second against the pitch file's note. `flags` is still exposed as a word because
//! that is how it was measured, with [`InfocomSound::cycles`] and
//! [`InfocomSound::base_note`] over it.
//!
//! **Verified against a reference rendition rather than reverse-engineered alone.**
//! `stories/Lurking.blb` is the same fourteen effects wrapped as Blorb `Snd `
//! resources in AIFF, and on all fourteen — 3, 4, 6–13 and 15–18 — the frame count
//! here equals the AIFF's `COMM` frame count and the sample bytes are
//! **byte-identical** to its `SSND` payload. The length field equals the file size
//! less two on every one, which is what makes it a usable signature.
//!
//! Eight of the fourteen rates differ from the Blorb's, always with the Blorb reading
//! LOWER, and every one of those is the pitch this header does not state on its own:
//! [`InfocomSound::rate`] reports the disk's figure and [`InfocomSound::effective_rate`]
//! the one it sounds at. See [`Pitch`].
//!
//! # The Macintosh lays the same three things out as two files
//!
//! `/MAC/SOUND` on *Lost Treasures* disc 2 carries `S<n>` — the same `.dat` header,
//! no extension — and, for the four effects whose pitch is not the sample's own,
//! `M<n>`: the eleven-byte pitch blob with the index appended, so one file does the
//! work of the Amiga's `.mid` and `.nam` together. An effect with no `M<n>` plays
//! `S<n>` as itself — and the four that have one are exactly the four whose Amiga
//! `.mid` does not read note 74, which is the note that bends nothing.
//!
//! **The header is the same; the payload underneath it is not.** The Macintosh writes
//! its samples as offset binary, silence at `0x80`, and nothing in the ten bytes
//! records that — see [`Encoding`], which is also where the evidence lives. It also
//! opens and closes each sample with a ramp to `0x00`, the rest position of a unipolar
//! output, which a bipolar one has to undo rather than reproduce. Four of *Sherlock*'s
//! fifteen effects are a half-rate decimation on this disc as well, so a Macintosh
//! sample is not simply its Amiga twin in another sign convention.
//!
//! # Why AIFF comes back out
//!
//! [`InfocomSound::to_aiff`] wraps the samples in the container the host already
//! decodes, so a disk-native sound reaches the mixer through exactly the path a
//! Blorb one does — same volume scaling, same repeats, same finish routine. Nothing
//! downstream learns that this format exists.

/// What a `sN.nam` says: which sample file and which pitch file are effect N.
///
/// **This index is not optional, and a naming convention is not a substitute.** On
/// *The Lurking Horror* every entry happens to name `sN.dat` and `sN.mid`, so the
/// numbering looks like it lives in the filenames. On *Sherlock* it does not: its
/// samples are called `armor`, `growl`, `splash`, `violin.bin`, and three separate
/// effects — 11, 12 and 13 — all name the SAME sample, `heart`, against three
/// different pitch files. A reader that matched `sN.dat` would find nothing on that
/// disk at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundIndex {
    /// The sample file, relative to the `Sound` directory.
    pub sample: String,
    /// The pitch file, relative to the same directory, or empty when the layout
    /// carries the pitch inline. See [`Pitch`].
    pub midi: String,
}

impl SoundIndex {
    /// Parse a `sN.nam`: a two-byte prefix, then the sample's NUL-terminated name,
    /// then the pitch file's, then zero padding to the block.
    pub fn parse(raw: &[u8]) -> Option<SoundIndex> {
        let body = raw.get(2..)?;
        let mut names = body.split(|&b| b == 0).filter(|s| !s.is_empty());
        let sample = std::str::from_utf8(names.next()?).ok()?.to_string();
        let midi = names.next().and_then(|m| std::str::from_utf8(m).ok()).unwrap_or("").to_string();
        (!sample.is_empty()).then_some(SoundIndex { sample, midi })
    }
}


/// The Note-On in a `sN.mid` — or, on the Macintosh, in the head of an `M<n>`.
///
/// Eleven bytes of MIDI, the same shape on every file of both games:
///
/// ```text
/// 00 09   9c <note> 40   FF 00 <nn>   9c <note> 00
/// ```
///
/// `00 09` is the byte count of what follows; `9c n 40` is a Note-On on channel `c`
/// at velocity 64; `FF 00 nn` is a meta event whose payload varies (`00 01`, `00 04`,
/// `00 07`, `80 00`); `9c n 00` is the matching Note-Off. Every file of both games is
/// channel 0.
///
/// **The second note is the Note-Off, not a reference pitch**, and reading it as one
/// was the whole of SQ-0912's error — see below. The reference the sample is played
/// against lives in the SAMPLE's header, and is [`InfocomSound::base_note`].
///
/// # What the interpreter does with it, read out of the 68000 it shipped
///
/// Hunk 8 of `The Lurking Horror` off *Lost Treasures* disk 4 (42,224 bytes) holds one
/// MIDI-event routine spanning hunk offsets `0x000`–`0x640`, and a pitch routine at
/// `0x642` whose only caller is the `bsr.w` at `0x4c2`. The pitch routine carries three
/// doubles as immediates — **1.05946309** (2^(1/12)) at `0x6a6`, **1000.0** at `0x6d8`
/// and **3579.49** at `0x6ea`, the NTSC Amiga colour-burst clock in kHz — and computes
///
/// ```text
/// ioa_Period = (3579.49 / (rate / 1000)) / 2^((noteA - noteB) / 12)
/// ```
///
/// Period is inversely proportional to frequency, so the sample sounds at
/// `rate · 2^((noteA − noteB)/12)`. The call site says what the two notes are:
///
/// ```text
/// 0x4a0  movea.l $40.l, a0      ; the loaded pitch buffer
/// 0x4aa  move.b  (a0), d2       ;   noteA = pitchbuf[cursor + 1]
/// 0x4ac  movea.l $c(a7), a0
/// 0x4b0  adda.l  #$78, a0       ; a per-channel byte array
/// 0x4b8  move.b  (a0), d3       ;   noteB = bases[channel]
/// ```
///
/// and `bases[channel]` was loaded eighty instructions earlier, at `0x41a`, from
/// `desc[1]` — **byte 1 of the loaded sample**, which is the `.dat` header's flags
/// word less the two-byte length the loader strips, i.e. the flags' LOW byte. The
/// same routine writes `desc[0]`, the flags' high byte, straight into `ioa_Cycles`,
/// and takes `ioa_Data` from `desc + 8` and `ioa_Length` from the word at `desc + 6`.
/// So the header's flags field, recorded for years as not understood, is two bytes:
/// **repeat count, then the sample's own base note.**
///
/// The note is also transposed before use. At `0x43c` and `0x45c` the routine
/// subtracts `0x18` from `pitchbuf[cursor + 1]` when the status byte is `0x90` or
/// `0x91`, and `0x0c` when it is `0x92` — two octaves down for channels 0 and 1, one
/// for channel 2 — writing it back in place.
///
/// # This overturns SQ-0912, and the corpus says so in 27 places
///
/// SQ-0912 read the blob's second note as the reference and concluded that every
/// Amiga pitch file is a unison pair that bends nothing. It is a Note-Off; it equals
/// the Note-On because that is what a Note-Off does.
///
/// With the real model — `(note − transposition) − base_note` — the decoded rate
/// matches `stories/Lurking.blb` and `stories/Sherlock.blb` **exactly on 27 of the 29
/// effects the two Blorbs carry**, including all eighteen the model leaves unbent and
/// all nine it bends. The two it misses are the two documented anomalies: *Lurking
/// Horror*'s effect 17, whose disk rate of 32910 Hz is already past what Paula can
/// clock before any bend is applied, and *Sherlock*'s effect 13, where the Blorb
/// carries a differently-trimmed take (13,989 frames against its siblings' 13,999).
///
/// The unison reading agreed with the Blorb on the eighteen unbent effects and
/// disagreed on all nine bent ones, which is exactly the shape of a model that is
/// right about nothing except when the answer is zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pitch {
    /// The Note-On's status byte, `0x90..=0x9F`. The channel in its low nibble is
    /// what picks the transposition, so it is data, not a signature.
    pub status: u8,
    /// The note the effect is asked to sound, before transposition.
    pub note: u8,
}

/// The pitch blob's fixed length, and the offset the Macintosh index follows it at.
const PITCH: usize = 11;

/// The fastest Paula can clock a channel, from the interpreter's own 3579.49 kHz and
/// the minimum period of 124 the Amiga Hardware Reference Manual states for DMA audio.
///
/// **This clamp is ours, not Infocom's** — the routine at `0x642` writes whatever it
/// computed straight into `ioa_Period` with no check, and the hardware is what refuses
/// to go faster. It bites on exactly one sound in either game, *Lurking Horror*'s
/// effect 17, whose stated 32910 Hz already needs a period of 109.
const PAULA_CEILING_HZ: u32 = 3_579_490 / 124;

impl Pitch {
    /// Parse the eleven-byte blob, or `None` when the bytes are not one.
    ///
    /// Trailing bytes are ignored rather than refused, because the Macintosh form is
    /// this blob with the index appended.
    pub fn parse(raw: &[u8]) -> Option<Pitch> {
        let ev = raw.get(..PITCH)?;
        let on = ev[2];
        (be16(ev, 0) == 9 && on & 0xF0 == 0x90 && ev[8] & 0xF0 == 0x90)
            .then_some(Pitch { status: on, note: ev[3] })
    }

    /// The octaves the interpreter drops the note by before comparing it, from the
    /// Note-On's channel: two for channels 0 and 1, one for channel 2, none beyond.
    pub fn transposition(&self) -> i32 {
        match self.status {
            0x90 | 0x91 => 24,
            0x92 => 12,
            _ => 0,
        }
    }

    /// How far the sample is bent, in semitones, against the note it was recorded at.
    ///
    /// `base` is the SAMPLE's, not this file's — see [`InfocomSound::base_note`].
    pub fn semitones(&self, base: u8) -> i32 {
        i32::from(self.note) - self.transposition() - i32::from(base)
    }

    /// `rate` bent by [`Pitch::semitones`].
    pub fn scale(&self, rate: u32, base: u8) -> u32 {
        match self.semitones(base) {
            0 => rate,
            n => (f64::from(rate) * 2f64.powf(f64::from(n) / 12.0)).round() as u32,
        }
    }
}

/// How a container's payload encodes silence.
///
/// The two machines disagree, and **nothing in the header says which** — the only
/// field that could have carried it overlaps: all thirteen `/MAC/SOUND` samples read
/// `0x0032` or `0x0132` in the flags word, and both of those appear on the Amiga
/// floppies too. The layout is the discriminator instead, and [`from_volume`] already
/// knows it: three files with extensions is AmigaDOS, two bare `S<n>`/`M<n>` is the
/// Macintosh.
///
/// # How this was settled, and why it is not a judgement call
///
/// *Sherlock*'s effect 8 is **byte-identical across the two media once the Macintosh
/// payload is XORed with `0x80`** — zero differing bytes in 25,820 — and the chain
/// carries on to a third rendition: those Amiga bytes are in turn byte-identical to
/// `stories/Sherlock.blb`'s `SSND` payload on fourteen of fifteen effects, and an
/// 8-bit AIFF is signed by definition. So Amiga signed, Macintosh offset-binary, with
/// a reference rendition anchoring the sign rather than a guess about it.
///
/// The other same-length effects (3, 4, 5, 6, 9, 15, 16) agree after the same XOR to
/// within 0.3–0.6% of their bytes, and effects 7, 10, 11–13 and 14 are the Macintosh
/// shipping a half-rate decimation — a different master, not a different encoding.
/// Across all fifteen the statistics settle it on their own: read as offset binary the
/// Macintosh payload's mean lands within 2 of zero and its RMS within 2% of the
/// Amiga's, and read as signed its RMS is two to five times too large and its
/// sample-to-sample roughness three to eight times too high. That is what reached the
/// user as "very distorted and crunchy", and why a quiet tail played as full-scale
/// noise made the sounds seem to run long (SQ-0921).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// Silence at `0x00`, as AmigaDOS writes it and as an 8-bit AIFF defines it.
    Signed,
    /// Silence at `0x80`, as `/MAC/SOUND` writes it — while the channel is driven.
    /// A Macintosh sample also ramps to and from `0x00`, the machine's true rest
    /// level, at each end; `to_signed` is where that is undone and evidenced.
    OffsetBinary,
}

/// One decoded Infocom sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfocomSound {
    /// Playback rate in Hz, as the disk states it.
    pub rate: u32,
    /// The header's flags word: repeat count in the high byte, the sample's own base
    /// note in the low one. See [`InfocomSound::cycles`] and
    /// [`InfocomSound::base_note`], and [`Pitch`] for where that was read out of.
    pub flags: u16,
    /// Signed 8-bit mono PCM, `frames` bytes — always signed, whatever the disk
    /// wrote, because [`InfocomSound::parse`] converts an [`Encoding::OffsetBinary`]
    /// payload on the way in.
    pub samples: Vec<u8>,
    /// The pitch its index pointed at, when there was one. `None` from
    /// [`InfocomSound::parse`], which reads the container alone; [`from_volume`] is
    /// what pairs a sample with its pitch file.
    pub pitch: Option<Pitch>,
}

/// The fixed header, before the samples.
const HEADER: usize = 10;

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

/// The Macintosh's anti-click ramp, as a fraction of a second at each end of a
/// sample. See [`to_signed`].
const MAC_RAMP_HZ: u64 = 150;

/// Turn an offset-binary payload into the signed PCM [`InfocomSound::samples`]
/// promises, undoing the Macintosh's ramp on the way.
///
/// Subtracting a flat `0x80` is only right in the middle. **A Macintosh sample has
/// two different silences**: `0x80` while the channel is driven, and `0x00` when it
/// is not — the machine's sound output is a unipolar PWM whose rest position is no
/// pulse at all, so a sample that began at its DC would slam the speaker off its rest
/// position and click. Thirteen of *Sherlock*'s fifteen effects therefore open at
/// `0x00`, ramp linearly up to full drive, play, and ramp back down.
///
/// Subtract a flat `0x80` from that and both ramps become an excursion to full
/// NEGATIVE — a pop at the start and another at the end, which is what a bipolar
/// output makes of a bias the Macintosh needed and we do not (SQ-0922). Subtracting
/// the ramp instead leaves `env·(sample − 0x80)`: the same fade, about silence.
///
/// # The ramp is 1/150 s, measured and not assumed
///
/// The Macintosh master is the Amiga master times a trapezoid, and that is checkable
/// because seven of *Sherlock*'s effects are the same recording at the same rate on
/// both discs. Fitting the ramp length against those seven puts `rate ÷ N` inside
/// `[149.97, 151.11]` on every one, so 150 is the only round figure that fits all
/// of them; effect 9 then reconstructs from the Amiga's bytes with **zero error**,
/// and the rest to within one or two counts in 256.
///
/// The tell that nothing else differs between the masters: read as a flat `0x80`, the
/// number of bytes by which a Macintosh effect disagrees with its Amiga twin is
/// exactly TWICE the fitted ramp length — 204 for effect 3 against `N` = 102, 128 for
/// effect 9 against 64 — and zero outside the ramps.
///
/// # Two effects have no ramp, and are left alone
///
/// Effects 8 and 14 open and close mid-signal (`0x93`, `0x9F`), so the mastering pass
/// missed them and they click on a Macintosh too. They are recognised by the only
/// signature that cannot fire by accident: a ramped file begins AND ends at exactly
/// full negative, which no recording does.
fn to_signed(samples: &mut [u8], rate: u32) {
    let n = samples.len();
    let ramped = n > 2 && samples[0] == 0 && samples[n - 1] == 0;
    let rate = u64::from(rate.max(1));
    for (i, b) in samples.iter_mut().enumerate() {
        // Flat 0x80 in the body, and in a ramped file a linear approach to it over
        // the first and last `rate / 150` samples. Integer throughout: the ramp is
        // `128 · min(i, n−1−i) / (rate / 150)`, rearranged to keep the division last.
        let bias = if ramped {
            let k = i.min(n - 1 - i) as u64;
            (128 * k * MAC_RAMP_HZ / rate).min(128) as i32
        } else {
            128
        };
        *b = (i32::from(*b) - bias).clamp(-128, 127) as i8 as u8;
    }
}

impl InfocomSound {
    /// Parse a `Sound/sN.dat` or a `/MAC/SOUND/S<n>`, or `None` when the bytes are
    /// not one.
    ///
    /// The signature is the length field agreeing with the file's own size, which
    /// no other file on these disks satisfies by accident, plus a frame count that
    /// fits in what is left.
    ///
    /// `encoding` is the caller's, not the file's: the header does not record it and
    /// the two machines disagree. See [`Encoding`], and note that the samples come
    /// back signed either way.
    pub fn parse(raw: &[u8], encoding: Encoding) -> Option<InfocomSound> {
        if raw.len() < HEADER || raw.len() > u16::MAX as usize + 2 {
            return None;
        }
        if be16(raw, 0) as usize != raw.len() - 2 {
            return None;
        }
        let frames = be16(raw, 8) as usize;
        if frames == 0 || HEADER + frames > raw.len() {
            return None;
        }
        let mut samples = raw[HEADER..HEADER + frames].to_vec();
        if encoding == Encoding::OffsetBinary {
            to_signed(&mut samples, u32::from(be16(raw, 4)));
        }
        Some(InfocomSound { rate: u32::from(be16(raw, 4)), flags: be16(raw, 2), samples, pitch: None })
    }

    /// The note this sample was recorded at — the flags word's LOW byte.
    ///
    /// The reference every pitch file is measured against, and the field that makes a
    /// bend computable. It reads 50 or 60 across both games. See [`Pitch`] for the
    /// disassembly that identifies it.
    pub fn base_note(&self) -> u8 {
        self.flags as u8
    }

    /// Paula's `ioa_Cycles` — the flags word's HIGH byte, 0 or 1 on every file here.
    ///
    /// Carried because it is what the interpreter writes into the audio request, not
    /// because anything reads it yet: the Z-machine's own `sound_effect` operand
    /// already says how many times to repeat, and that is what the host obeys.
    pub fn cycles(&self) -> u8 {
        (self.flags >> 8) as u8
    }

    /// The rate this actually plays at: the disk's own, bent by its [`Pitch`] against
    /// [`InfocomSound::base_note`], and capped at what Paula can clock.
    ///
    /// Nine of the twenty-nine effects across the two games are bent, and the cap
    /// bites on exactly one — see `PAULA_CEILING_HZ`.
    pub fn effective_rate(&self) -> u32 {
        let hz = self.pitch.map_or(self.rate, |p| p.scale(self.rate, self.base_note()));
        hz.min(PAULA_CEILING_HZ)
    }

    /// The same sound as an AIFF `FORM`, for the host's existing decoder.
    ///
    /// Mono, 8-bit, at [`InfocomSound::effective_rate`]. Nothing is resampled or
    /// requantised here — a pitch is a change to the rate the wrapper declares, and
    /// the payload survives unchanged either way.
    pub fn to_aiff(&self) -> Vec<u8> {
        let n = self.samples.len() as u32;
        let mut out = Vec::with_capacity(self.samples.len() + 64);
        // COMM (18) and SSND (8 + samples), each with an 8-byte chunk header, after
        // the four bytes of form type.
        let form_len = 4 + (8 + 18) + (8 + 8 + n);
        out.extend_from_slice(b"FORM");
        out.extend_from_slice(&form_len.to_be_bytes());
        out.extend_from_slice(b"AIFF");

        out.extend_from_slice(b"COMM");
        out.extend_from_slice(&18u32.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes()); // channels
        out.extend_from_slice(&n.to_be_bytes()); // frames
        out.extend_from_slice(&8u16.to_be_bytes()); // bits per sample
        out.extend_from_slice(&extended80(self.effective_rate()));

        out.extend_from_slice(b"SSND");
        out.extend_from_slice(&(8 + n).to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes()); // offset
        out.extend_from_slice(&0u32.to_be_bytes()); // block size
        out.extend_from_slice(&self.samples);
        // A FORM's chunks are word-aligned; SSND's payload may be odd.
        if n % 2 == 1 {
            out.push(0);
        }
        out
    }
}

/// An integer sample rate as AIFF's 80-bit IEEE 754 extended float.
///
/// The format is a 15-bit biased exponent, a sign bit, and an **explicit** leading
/// mantissa bit — unlike IEEE single/double, where it is implied. So a normalised
/// value has the mantissa's top bit set, and the bias is 16383.
fn extended80(rate: u32) -> [u8; 10] {
    let mut out = [0u8; 10];
    if rate == 0 {
        return out;
    }
    let top = 31 - rate.leading_zeros(); // floor(log2(rate))
    let exp = 16383 + top;
    let mantissa = u64::from(rate) << (63 - top);
    out[0..2].copy_from_slice(&(exp as u16).to_be_bytes());
    out[2..10].copy_from_slice(&mantissa.to_be_bytes());
    out
}

/// Every sound a mounted volume's sound directory offers, by Z-machine effect
/// number, as `(sample name, decoded sample)`.
///
/// `files` is whatever the medium reported — [`crate::medium::MountedDisk::contents`]
/// or any equivalent — as `(path, bytes)`. Lives here rather than in a host so the
/// TUI and `zvm-cli` cannot drift: both mount their own medium and hand it over.
///
/// **The effect number comes from the INDEX, never from a sample's filename.** On
/// *The Lurking Horror* every `sN.nam` happens to name `sN.dat`, so the numbering
/// looks like a convention; on *Sherlock* the samples are `armor`, `growl`, `splash`,
/// `violin.bin`, and effects 11, 12 and 13 all name the same `heart`. Keying on
/// filenames finds nothing on that disk. The Macintosh's `M<n>` is the same index
/// with its pitch inlined, and there a sample is genuinely shared under one name:
/// `M11` and `M13` both point at `S12`.
///
/// Two passes, because an index outranks a bare sample: `S12` is effect 12 in its own
/// right AND the sample `M11` plays, and only the index knows the second fact.
///
/// Effects below 3 are dropped: ZMSD §9 reserves 1 and 2 for the interpreter's own
/// bleeps, which are synthesised and never sampled.
pub fn from_volume<'a, I>(files: I) -> std::collections::BTreeMap<u16, (String, InfocomSound)>
where
    I: IntoIterator<Item = (&'a str, &'a [u8])>,
{
    let mut by_path: std::collections::BTreeMap<String, (&str, &[u8])> = Default::default();
    for (p, b) in files {
        by_path.insert(p.to_ascii_lowercase(), (p, b));
    }
    let dir_of = |path: &str| path.rsplit_once('/').map_or(String::new(), |(d, _)| format!("{d}/"));
    let mut out = std::collections::BTreeMap::new();

    // Pass 1: the indices, which are what carry a sample name and a pitch.
    for (path, (_, raw)) in &by_path {
        let dir = dir_of(path);
        let (effect, idx, pitch, encoding) = match sound_entry(path) {
            Some(Entry::AmigaIndex(n)) => {
                let Some(idx) = SoundIndex::parse(raw) else { continue };
                let pitch = by_path
                    .get(&format!("{dir}{}", idx.midi.to_ascii_lowercase()))
                    .and_then(|(_, m)| Pitch::parse(m));
                (n, idx, pitch, Encoding::Signed)
            }
            Some(Entry::MacIndex(n)) => {
                let Some(pitch) = Pitch::parse(raw) else { continue };
                let Some(idx) = raw.get(PITCH..).and_then(SoundIndex::parse) else { continue };
                (n, idx, Some(pitch), Encoding::OffsetBinary)
            }
            _ => continue,
        };
        let Some((_, sample)) = by_path.get(&format!("{dir}{}", idx.sample.to_ascii_lowercase()))
        else {
            continue;
        };
        let Some(snd) = InfocomSound::parse(sample, encoding) else { continue };
        out.insert(effect, (idx.sample.clone(), InfocomSound { pitch, ..snd }));
    }

    // Pass 2: a Macintosh sample no index claimed plays as itself, unbent.
    for (path, (orig, raw)) in &by_path {
        let Some(Entry::MacSample(n)) = sound_entry(path) else { continue };
        if out.contains_key(&n) {
            continue;
        }
        let Some(snd) = InfocomSound::parse(raw, Encoding::OffsetBinary) else { continue };
        let name = orig.rsplit('/').next().unwrap_or(orig).to_string();
        out.insert(n, (name, snd));
    }
    out
}

/// What a path in a sound directory is, and the effect number it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    /// `Sound/sN.nam` — the AmigaDOS index, naming a sample file and a pitch file.
    AmigaIndex(u16),
    /// `SOUND/MN` — the Macintosh index, which inlines the pitch ahead of the name.
    MacIndex(u16),
    /// `SOUND/SN` — a Macintosh sample, played as itself when no `MN` claims it.
    MacSample(u16),
}

/// Classify a path inside a sound directory, or `None`.
///
/// Matched on the PATH, because the directory is what makes it a sound file rather
/// than any other `s3.nam` on the volume — and because until SQ-0908 an AmigaDOS
/// mount reported no directory at all, so this could not have been written.
///
/// The extension is what separates the two layouts, and it has to: `sound/s3.dat` is
/// an Amiga sample, which its own index already names, while `SOUND/S3` is a
/// Macintosh one, which nothing else does.
/// Could the file at `path` matter to [`from_volume`]? Only a file under a
/// `sound/` directory can: every index and every bare sample is one, and a
/// sample an index names lives in the index's own directory.
///
/// A caller that would have to READ each file to hand it over asks this first,
/// so a 300 MB CD is not read whole to find a few kilobytes of sound (SQ-1761).
pub fn on_a_sound_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.starts_with("sound/") || lower.contains("/sound/")
}

fn sound_entry(lower_path: &str) -> Option<Entry> {
    if !lower_path.starts_with("sound/") && !lower_path.contains("/sound/") {
        return None;
    }
    let file = lower_path.rsplit('/').next()?;
    let entry = match file.strip_suffix(".nam") {
        Some(stem) => Entry::AmigaIndex(stem.strip_prefix('s')?.parse().ok()?),
        None if file.contains('.') => return None,
        None if file.starts_with('m') => Entry::MacIndex(file[1..].parse().ok()?),
        None => Entry::MacSample(file.strip_prefix('s')?.parse().ok()?),
    };
    let n = match entry {
        Entry::AmigaIndex(n) | Entry::MacIndex(n) | Entry::MacSample(n) => n,
    };
    (n >= 3).then_some(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.dat` of `frames` samples, built the way the disk builds one.
    fn dat(rate: u16, flags: u16, samples: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&((HEADER + samples.len() - 2) as u16).to_be_bytes());
        v.extend_from_slice(&flags.to_be_bytes());
        v.extend_from_slice(&rate.to_be_bytes());
        v.extend_from_slice(&0u16.to_be_bytes());
        v.extend_from_slice(&(samples.len() as u16).to_be_bytes());
        v.extend_from_slice(samples);
        v
    }

    /// The index, in both the shapes the two sound games use, and both layouts.
    #[test]
    fn an_index_path_yields_its_effect_number() {
        assert_eq!(sound_entry("sound/s3.nam"), Some(Entry::AmigaIndex(3)));
        assert_eq!(sound_entry("sound/s18.nam"), Some(Entry::AmigaIndex(18)));
        assert_eq!(sound_entry("lurking/sound/s7.nam"), Some(Entry::AmigaIndex(7)), "nested under the game");
        assert_eq!(sound_entry("mac/sound/m11"), Some(Entry::MacIndex(11)), "Macintosh, pitch inlined");
        assert_eq!(sound_entry("mac/sound/s12"), Some(Entry::MacSample(12)), "Macintosh, bare sample");
    }

    /// Everything that is NOT one, which is the half that keeps this off the rest of
    /// a disk. The extension is what tells the two layouts apart, so an Amiga sample
    /// must not read as a Macintosh one.
    #[test]
    fn anything_else_is_not_an_index() {
        assert_eq!(sound_entry("sound/s10.dat"), None, "an Amiga sample, which its index names");
        assert_eq!(sound_entry("sound/s10.mid"), None, "its pitch file");
        assert_eq!(sound_entry("s3.nam"), None, "outside the Sound directory");
        assert_eq!(sound_entry("sound/story.nam"), None, "not numbered");
        assert_eq!(sound_entry("sound/s1.nam"), None, "1 and 2 are the interpreter's bleeps");
        assert_eq!(sound_entry("sound/s2.nam"), None);
        assert_eq!(sound_entry("mac/sound/sherlock"), None, "a name that merely starts with s");
        assert_eq!(sound_entry("mac/sound/desktop"), None);
    }

    /// A whole volume, in the shape Sherlock's disk has: descriptive sample names,
    /// and one sample shared by more than one effect.
    #[test]
    fn a_volume_is_indexed_by_effect_number() {
        let heart = dat(18430, 0, &[1, 2, 3, 4]);
        let armor = dat(15360, 0, &[9, 9]);
        let files: Vec<(&str, &[u8])> = vec![
            ("Sound/s3.nam", b"\x01\x00armor\x00\x00fan.mid\x00"),
            ("Sound/armor", &armor),
            ("Sound/s11.nam", b"\x01\x00heart\x00\x00heart1.mid\x00"),
            ("Sound/s13.nam", b"\x01\x00heart\x00\x00heart3.mid\x00"),
            ("Sound/heart", &heart),
            ("Sound/s2.nam", b"\x01\x00armor\x00\x00fan.mid\x00"),
            ("Story.Data", b"not a sound"),
        ];
        let got = from_volume(files);
        assert_eq!(got.keys().copied().collect::<Vec<_>>(), vec![3, 11, 13], "2 is a bleep, not a sample");
        assert_eq!(got[&3].0, "armor");
        assert_eq!(got[&11].0, "heart");
        assert_eq!(got[&13].0, "heart", "two effects, one sample, different pitches");
        assert_eq!(got[&11].1.samples, got[&13].1.samples);
        assert_eq!(got[&3].1.rate, 15360);
    }

    #[test]
    fn a_nam_names_its_sample_and_its_pitch_file() {
        // The Lurking Horror, where the names echo the effect number.
        let lh = SoundIndex::parse(b"\x01\x00s10.dat\x00\x00s10.mid\x00\x00\x00\x00\x00\x00").expect("parses");
        assert_eq!(lh, SoundIndex { sample: "s10.dat".into(), midi: "s10.mid".into() });
        // Sherlock, where they do not — and where three effects share one sample.
        let s11 = SoundIndex::parse(b"\x01\x00heart\x00\x00heart1.mid\x00\x00").expect("parses");
        let s13 = SoundIndex::parse(b"\x01\x00heart\x00\x00heart3.mid\x00\x00").expect("parses");
        assert_eq!(s11.sample, "heart");
        assert_eq!(s13.sample, "heart", "the same sample, at a different pitch");
        assert_ne!(s11.midi, s13.midi);
        assert_eq!(
            SoundIndex::parse(b"\x01\x00violin.bin\x00\x00clk.mid\x00\x00\x00").map(|i| i.sample),
            Some("violin.bin".to_string()),
            "a sample name may carry its own extension",
        );
        assert_eq!(SoundIndex::parse(b""), None);
        assert_eq!(SoundIndex::parse(b"\x01\x00"), None, "no name at all");
    }

    /// The eleven bytes, in the exact shape both games ship them.
    ///
    /// `heart3.mid` is copied out of the *Sherlock* floppy and `M13` out of
    /// `/MAC/SOUND` on *Lost Treasures* disc 2 — the SAME effect on the two platforms,
    /// and the SAME bend, which is the point. SQ-0912 had them differing because it
    /// read the Note-Off as a reference pitch; the Macintosh's simply was not updated
    /// when someone copied `heart1.mid` to make `M13`, which is why it reads 68.
    #[test]
    fn a_pitch_file_is_a_note_and_its_note_off() {
        // Sherlock's `heart` states base note 50 in its own header.
        let amiga = Pitch::parse(b"\x00\x09\x90\x4f\x40\xff\x00\x04\x90\x4f\x00").expect("parses");
        assert_eq!(amiga, Pitch { status: 0x90, note: 79 }, "Sherlock's heart3.mid");
        assert_eq!(amiga.transposition(), 24, "channel 0 drops two octaves");
        assert_eq!(amiga.semitones(50), 5, "79 - 24 - 50");
        assert_eq!(amiga.scale(18430, 50), 24601);

        let mac = Pitch::parse(b"\x00\x09\x90\x4f\x40\xff\x00\x04\x90\x44\x00\x01\x00S12\x00")
            .expect("the index that follows is ignored, not refused");
        assert_eq!(mac, Pitch { status: 0x90, note: 79 }, "/MAC/SOUND/M13 asks for the same note");
        assert_eq!(mac.semitones(50), amiga.semitones(50), "the stale Note-Off changes nothing");
        // The Macintosh master is half-rate, so the same bend lands an octave down in
        // the number and at the same pitch to the ear.
        assert_eq!(mac.scale(9215, 50), 12301);
    }

    /// The transposition comes from the Note-On's CHANNEL, which is why the status
    /// byte is data rather than a signature.
    #[test]
    fn the_channel_picks_the_transposition() {
        let of = |st: u8| Pitch { status: st, note: 74 }.transposition();
        assert_eq!((of(0x90), of(0x91)), (24, 24), "channels 0 and 1 drop two octaves");
        assert_eq!(of(0x92), 12, "channel 2 drops one");
        assert_eq!(of(0x93), 0, "no channel beyond those is transposed");
        // Every file of both games is channel 0, so 74 is the note that bends nothing
        // against a base of 50 — which is what eighteen of the twenty-nine effects say.
        assert_eq!(Pitch { status: 0x90, note: 74 }.semitones(50), 0);
    }

    #[test]
    fn bytes_that_are_not_a_pitch_file_are_refused() {
        assert_eq!(Pitch::parse(b""), None);
        assert_eq!(Pitch::parse(b"\x00\x09\x90\x4f\x40\xff\x00\x04\x90\x4f"), None, "ten bytes");
        assert_eq!(
            Pitch::parse(b"\x00\x0b\x90\x4f\x40\xff\x00\x04\x90\x4f\x00"),
            None,
            "a length that is not nine",
        );
        assert_eq!(
            Pitch::parse(b"\x00\x09\x80\x4f\x40\xff\x00\x04\x90\x4f\x00"),
            None,
            "not a Note-On",
        );
    }

    /// **One sample, two pitch files, two different sounds** — the case SQ-0912 got
    /// wrong and SQ-0923 corrects.
    ///
    /// *Sherlock*'s `heart` is effects 11, 12 and 13 against pitch files reading 68,
    /// 74 and 79. The old reading made all three identical because it compared each
    /// file's Note-On to its own Note-Off; the reference is the SAMPLE's base note,
    /// which is the same for all three, so the three notes give three rates.
    #[test]
    fn one_sample_under_three_pitch_files_plays_three_ways() {
        // flags 0x0032: repeat count 0, base note 50 — Sherlock's `heart` exactly.
        let heart = dat(18430, 0x0032, &[1, 2, 3, 4]);
        let files: Vec<(&str, &[u8])> = vec![
            ("Sound/s11.nam", b"\x01\x00heart\x00\x00heart1.mid\x00"),
            ("Sound/heart1.mid", b"\x00\x09\x90\x44\x40\xff\x00\x04\x90\x44\x00"),
            ("Sound/s12.nam", b"\x01\x00heart\x00\x00growl.mid\x00"),
            ("Sound/growl.mid", b"\x00\x09\x90\x4a\x40\xff\x00\x04\x90\x4a\x00"),
            ("Sound/s13.nam", b"\x01\x00heart\x00\x00heart3.mid\x00"),
            ("Sound/heart3.mid", b"\x00\x09\x90\x4f\x40\xff\x00\x04\x90\x4f\x00"),
            ("Sound/heart", &heart),
        ];
        let got = from_volume(files);
        assert_eq!(got[&11].1.base_note(), 50, "the reference is in the sample's header");
        assert_eq!(got[&11].1.cycles(), 0, "and the other flags byte is the repeat count");
        assert_eq!(got[&11].1.pitch, Some(Pitch { status: 0x90, note: 68 }));
        assert_eq!(got[&12].1.pitch, Some(Pitch { status: 0x90, note: 74 }));
        assert_eq!(got[&13].1.pitch, Some(Pitch { status: 0x90, note: 79 }));

        assert_eq!(got[&11].1.effective_rate(), 13032, "68 - 24 - 50 = -6 semitones");
        assert_eq!(got[&12].1.effective_rate(), 18430, "74 is the note that bends nothing");
        assert_eq!(got[&13].1.effective_rate(), 24601, "79 - 24 - 50 = +5");
        assert_ne!(got[&11].1.effective_rate(), got[&13].1.effective_rate(), "not one sound, three");
    }

    /// Paula cannot be clocked past its minimum period, and one sound in either game
    /// asks it to be — before any bend is applied.
    #[test]
    fn the_rate_is_capped_at_what_paula_can_clock() {
        assert_eq!(PAULA_CEILING_HZ, 28866);
        // Lurking Horror's effect 17: 32910 Hz stated, base 50, note 90 -> +16.
        let s17 = dat(32910, 0x0032, &[1, 2, 3, 4]);
        let files: Vec<(&str, &[u8])> = vec![
            ("Sound/s17.nam", b"\x01\x00s17.dat\x00\x00s17.mid\x00"),
            ("Sound/s17.mid", b"\x00\x09\x90\x5a\x40\xff\x00\x04\x90\x5a\x00"),
            ("Sound/s17.dat", &s17),
        ];
        let got = from_volume(files);
        assert_eq!(got[&17].1.rate, 32910, "the disk's own figure survives untouched");
        assert_eq!(got[&17].1.effective_rate(), PAULA_CEILING_HZ, "and the hardware's is what plays");
    }

    /// The Macintosh layout, laid out as `/MAC/SOUND` is: bare `S<n>` samples, and an
    /// `M<n>` only for the effects whose pitch is not the sample's own.
    ///
    /// Effect 12 has no `M12`, so `S12` is effect 12 in its own right; `M11` and `M13`
    /// both redirect to that same `S12`, which is the whole reason the index cannot be
    /// skipped in favour of the filename.
    #[test]
    fn a_macintosh_volume_bends_the_effects_that_have_an_index() {
        let s12 = dat(9215, 0x0032, &[1, 2, 3, 4]);
        let s3 = dat(15360, 0x0132, &[9, 9]);
        let files: Vec<(&str, &[u8])> = vec![
            ("MAC/SOUND/S3", &s3),
            ("MAC/SOUND/S12", &s12),
            ("MAC/SOUND/M11", b"\x00\x09\x90\x44\x40\xff\x00\x04\x90\x44\x00\x01\x00S12\x00"),
            ("MAC/SOUND/M13", b"\x00\x09\x90\x4f\x40\xff\x00\x04\x90\x44\x00\x01\x00S12\x00"),
            ("MAC/SHERLOCK", b"not a sound"),
        ];
        let got = from_volume(files);
        assert_eq!(got.keys().copied().collect::<Vec<_>>(), vec![3, 11, 12, 13]);
        assert_eq!(got[&3].0, "S3", "a sample no index claimed keeps its own name and case");
        assert_eq!(got[&11].0, "S12", "the index redirects, and the filename would not have");
        assert_eq!(got[&12].0, "S12");
        assert_eq!(got[&13].0, "S12");
        assert_eq!(got[&11].1.samples, got[&13].1.samples, "one sample, three effects");
        // 0x09 as offset binary is 0x89 as two's complement; nothing else about the
        // container changes between the machines, so this is the whole difference.
        assert_eq!(got[&3].1.samples, vec![0x89, 0x89], "a Macintosh payload arrives signed");
        assert_eq!(got[&12].1.samples, vec![0x81, 0x82, 0x83, 0x84]);

        // Both samples state base note 50, as every Sherlock sample does.
        assert_eq!(got[&3].1.effective_rate(), 15360, "no index claims it, so it plays as itself");
        assert_eq!(got[&12].1.effective_rate(), 9215, "no M12 either");
        assert_eq!(got[&11].1.effective_rate(), 6516, "M11 asks for note 68: 68 - 24 - 50 = -6");
        assert_eq!(got[&13].1.effective_rate(), 12301, "M13 asks for note 79: +5");
        assert_eq!(got[&13].1.rate, 9215, "the disk's own figure is still there behind it");
    }

    /// The two machines disagree about where silence is, the header does not record
    /// it, and reading one as the other is what made `/MAC/SOUND` "distorted and
    /// crunchy" (SQ-0921).
    ///
    /// Offset binary and two's complement differ by exactly the sign bit, so the same
    /// bytes decode as two waveforms an octave apart in level: `0x80` is silence one
    /// way and full negative the other.
    #[test]
    fn a_macintosh_payload_is_offset_binary_and_an_amiga_one_is_not() {
        let raw = dat(15360, 0x0132, &[0x00, 0x80, 0xFF, 0x7F]);
        let signed = InfocomSound::parse(&raw, Encoding::Signed).expect("parses");
        let offset = InfocomSound::parse(&raw, Encoding::OffsetBinary).expect("parses");
        assert_eq!(signed.samples, vec![0x00, 0x80, 0xFF, 0x7F], "AmigaDOS wrote what it meant");
        assert_eq!(offset.samples, vec![0x80, 0x00, 0x7F, 0xFF], "silence moves from 0x80 to 0x00");
        assert_eq!(signed.rate, offset.rate, "the header is read the same either way");
        assert_eq!(signed.flags, offset.flags);
    }

    /// The Amiga path must not have moved, because its samples are the ones already
    /// pinned byte-identical to a Blorb's `SSND` payload.
    #[test]
    fn an_amiga_volume_still_hands_its_samples_back_untouched() {
        let heart = dat(18430, 0x013C, &[0x00, 0x80, 0xFF, 0x7F]);
        let files: Vec<(&str, &[u8])> = vec![
            ("Sound/s11.nam", b"\x00\x02heart\x00s11.mid\x00"),
            ("Sound/s11.mid", b"\x00\x09\x90\x44\x40\xff\x00\x04\x90\x44\x00"),
            ("Sound/heart", &heart),
        ];
        let got = from_volume(files);
        assert_eq!(got[&11].1.samples, vec![0x00, 0x80, 0xFF, 0x7F]);
    }

    /// A ramped Macintosh sample decodes to silence at both ends, not to a pop.
    ///
    /// Built the way the disc builds one — the signal times a trapezoid that opens and
    /// closes at the rest level — so what is asserted is that the decode inverts it:
    /// `env·(sample − 0x80)`, which is zero where `env` is.
    #[test]
    fn a_ramped_macintosh_sample_comes_back_faded_about_silence() {
        // 1500 Hz makes the ramp exactly 10 samples, so the arithmetic is checkable
        // by hand: 1500 / 150.
        const RATE: u16 = 1500;
        const N: i32 = 10;
        let n = 30i32;
        let signal = 20i32; // a constant +20 tone, so the envelope is all that varies
        let raw: Vec<u8> = (0..n)
            .map(|i| {
                let env = i.min(n - 1 - i).min(N);
                (((128 + signal) * env) / N) as u8
            })
            .collect();
        let s = InfocomSound::parse(&dat(RATE, 0x0132, &raw), Encoding::OffsetBinary).expect("parses");
        let out: Vec<i32> = s.samples.iter().map(|&b| i32::from(b as i8)).collect();

        assert_eq!(out[0], 0, "opens at silence, where a flat 0x80 would give -128");
        assert_eq!(out[n as usize - 1], 0, "and closes there");
        assert_eq!(out[15], signal, "the body is the signal itself, unscaled");
        // In between, the signal faded — the ramp the Macintosh wanted, about zero.
        for i in 0..N {
            assert_eq!(out[i as usize], signal * i / N, "sample {i} is the signal times the envelope");
        }
    }

    /// The two effects the mastering pass missed keep the flat conversion.
    ///
    /// A ramped file opens AND closes at exactly full negative; a recording does not,
    /// so this is what separates them. Getting it wrong the other way would push
    /// effects 8 and 14 off silence rather than onto it.
    #[test]
    fn an_unramped_macintosh_sample_is_converted_flat() {
        let raw = [0x93u8, 0x40, 0xC0, 0x85];
        let s = InfocomSound::parse(&dat(1500, 0x0032, &raw), Encoding::OffsetBinary).expect("parses");
        assert_eq!(s.samples, vec![0x13, 0xC0, 0x40, 0x05], "every byte is just its sign bit flipped");
    }

    #[test]
    fn the_header_is_read_and_the_samples_come_out_whole() {
        let s = InfocomSound::parse(&dat(15360, 0x003C, &[0x0F, 0x06, 0xFA, 0x0D]), Encoding::Signed).expect("parses");
        assert_eq!(s.rate, 15360);
        assert_eq!(s.flags, 0x003C);
        assert_eq!(s.samples, vec![0x0F, 0x06, 0xFA, 0x0D], "the payload is not touched");
    }

    /// The length field is the signature, so bytes that do not satisfy it are not
    /// one of these — which is what keeps the sniff off every other file on a disk.
    #[test]
    fn bytes_that_are_not_one_are_refused() {
        assert_eq!(InfocomSound::parse(b"", Encoding::Signed), None, "empty");
        assert_eq!(InfocomSound::parse(b"FORM\0\0\0\x08AIFF", Encoding::Signed), None, "an AIFF is not one");
        let mut wrong = dat(15360, 0, &[1, 2, 3, 4]);
        wrong[1] ^= 0xFF;
        assert_eq!(InfocomSound::parse(&wrong, Encoding::Signed), None, "a length that disagrees with the file");
        let mut over = dat(15360, 0, &[1, 2, 3, 4]);
        over[8..10].copy_from_slice(&9999u16.to_be_bytes());
        assert_eq!(InfocomSound::parse(&over, Encoding::Signed), None, "a frame count past the end");
    }

    /// AIFF's 80-bit extended float, against the value a real Blorb carries.
    ///
    /// Every expectation here is COPIED OUT of `stories/Lurking.blb`, not computed
    /// by hand — the first draft of this case invented the 18430 bytes and was wrong,
    /// which is the whole reason a constant gets read from a source (CLAUDE.md).
    ///
    /// | effect | rate | the Blorb's ten bytes |
    /// |---|---|---|
    /// | 10 | 15360 | `40 0C F0 00 …` |
    /// | 11 | 18430 | `40 0D 8F FC …` |
    /// | 15 | 11100 | `40 0C AD 70 …` |
    /// | 17 | 31250 | `40 0D F4 24 …` |
    ///
    /// The leading mantissa bit is EXPLICIT in this format, unlike IEEE single and
    /// double, which is the part that is easy to get wrong.
    #[test]
    fn a_rate_encodes_as_the_blorbs_own_bytes() {
        assert_eq!(extended80(15360), [0x40, 0x0C, 0xF0, 0x00, 0, 0, 0, 0, 0, 0]);
        assert_eq!(extended80(18430), [0x40, 0x0D, 0x8F, 0xFC, 0, 0, 0, 0, 0, 0]);
        assert_eq!(extended80(11100), [0x40, 0x0C, 0xAD, 0x70, 0, 0, 0, 0, 0, 0]);
        assert_eq!(extended80(31250), [0x40, 0x0D, 0xF4, 0x24, 0, 0, 0, 0, 0, 0]);
        assert_eq!(extended80(0), [0; 10], "no rate at all encodes as zero, not as a panic");
    }

    /// The wrapper is an AIFF the host's decoder reads, and the samples inside it
    /// are the ones that went in.
    #[test]
    fn the_aiff_wrapper_declares_mono_eight_bit_at_the_disks_rate() {
        let s = InfocomSound::parse(&dat(15360, 0, &[0x0F, 0x06, 0xFA, 0x0D]), Encoding::Signed).expect("parses");
        let a = s.to_aiff();
        assert_eq!(&a[0..4], b"FORM");
        assert_eq!(&a[8..12], b"AIFF");
        assert_eq!(u32::from_be_bytes([a[4], a[5], a[6], a[7]]) as usize, a.len() - 8, "FORM length");
        assert_eq!(&a[12..16], b"COMM");
        assert_eq!(be16(&a, 20), 1, "channels");
        assert_eq!(u32::from_be_bytes([a[22], a[23], a[24], a[25]]), 4, "frames");
        assert_eq!(be16(&a, 26), 8, "bits per sample");
        assert_eq!(&a[38..42], b"SSND");
        // id(4) + length(4) + offset(4) + blockSize(4) precede the frames.
        assert_eq!(&a[54..58], &[0x0F, 0x06, 0xFA, 0x0D], "the samples, unchanged");
    }

    /// An odd frame count still leaves a well-formed FORM: AIFF chunks are
    /// word-aligned, so the pad byte is outside `SSND`'s declared length.
    #[test]
    fn an_odd_sample_count_is_padded_without_being_counted() {
        let s = InfocomSound::parse(&dat(11100, 0, &[1, 2, 3]), Encoding::Signed).expect("parses");
        let a = s.to_aiff();
        assert_eq!(a.len() % 2, 0, "the form is word-aligned");
        let ssnd_len = u32::from_be_bytes([a[42], a[43], a[44], a[45]]);
        assert_eq!(ssnd_len, 8 + 3, "SSND counts its samples, not the pad");
    }
}
