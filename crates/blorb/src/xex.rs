//! The **Atari 8-bit binary-load executable** — `.xex`, and the one "medium"
//! here that is not a disk at all (SQ-1458).
//!
//! # Sources
//!
//! Public format documentation only:
//!
//! * The *Atari DOS 2.0S Reference Manual* (Atari, 1980) and the *Atari OS
//!   Manual*, both scanned at <http://atariarchives.org/> — the binary-load file
//!   format DOS's `L` command reads, and the `RUNAD`/`INITAD` vectors at `$02E0`
//!   and `$02E2` that a loaded file may set.
//! * This repository's own clean-room summary,
//!   [`docs/internals/scott-dialects-spec.md`](../../../docs/internals/scott-dialects-spec.md)
//!   §7.3 and §7.5, which record that the Atari 8-bit path has **no fixed memory
//!   base** and that the S.A.G.A. Hulk release arrives "in a container the spec
//!   has not specified" — which is this one.
//!
//! **No GPL interpreter source was read for this module**, per
//! `docs/internals/clean-room.md`.
//!
//! # The format
//!
//! A binary-load file is a list of **segments**, each one saying where in memory
//! it goes:
//!
//! ```text
//!   FF FF              the magic, once at the front
//!   <start> <end>      two little-endian words, INCLUSIVE addresses
//!   <end − start + 1>  that many bytes
//!   …                  the next segment, optionally preceded by another FF FF
//! ```
//!
//! `FF FF` before a later segment is allowed and ignored — DOS accepts it so
//! that two files can be concatenated — which is why the parser below skips it
//! wherever it appears rather than only at offset 0. A segment's length is
//! `end − start + 1` because both addresses are inclusive; `start > end` is
//! malformed and refused.
//!
//! # `RUNAD` and `INITAD` are vectors, not payload
//!
//! A segment that loads into `$02E0..$02E1` is **`RUNAD`**, the address DOS jumps
//! to when the load finishes; one that loads into `$02E2..$02E3` is **`INITAD`**,
//! run as soon as it is written. They are two-byte pokes at fixed OS locations,
//! not part of the program's image, and treating them as payload is the trap
//! this module exists to avoid: `The Hulk.xex` ends with a `RUNAD` segment, and a
//! reader that spanned every segment would report a **37,457-byte** image based
//! at `$02E0` — 15,648 bytes of hole in front of a game that actually occupies
//! `$4000..$9530`. So they are read as vectors, recorded on
//! [`Xex::run_address`] and [`Xex::init_address`], and left out of the image.
//!
//! # What comes out
//!
//! One entry: the memory the file loads, from the lowest address any payload
//! segment writes to the highest, with the gaps between segments zero-filled and
//! later segments overwriting earlier ones — which is what the machine's own
//! loader does. [`Xex::base`] is that lowest address, so a caller can turn an
//! address into an index without knowing anything about the container.
//!
//! Measured on the one specimen, `stories/scott-dialects/atari/The Hulk.xex`:
//! a 21,821-byte file holding a payload segment at `$4000..$9530` — **21,809
//! bytes** — and a `RUNAD` of `$9500`. The plain `AUTO\0GO\0` dictionary
//! signature (`scott-dialects-spec.md` §4.1) is at file offset `0x2CC4`, which is
//! image offset `0x2CBE` — six bytes of magic and segment header in front — and
//! therefore **address `$6CBE`**, which is the number a loader wants and the one
//! only a base read off the file can produce.

/// The binary-load magic, little-endian `$FFFF`.
pub const MAGIC: [u8; 2] = [0xff, 0xff];

/// `RUNAD`, the address DOS jumps to when the load finishes.
pub const RUNAD: u16 = 0x02e0;

/// `INITAD`, run as soon as a segment writes it.
pub const INITAD: u16 = 0x02e2;

/// Errors that can arise while parsing a binary-load file.
#[derive(Debug, PartialEq, Eq)]
pub enum XexError {
    /// Not a binary-load file this reader recognises: no magic, a malformed
    /// segment header, a segment that runs off the end, or trailing bytes that
    /// are not a segment.
    NotAXex,
}

/// One load record, as it appears in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// The first address it writes.
    pub start: u16,
    /// The last address it writes — **inclusive**.
    pub end: u16,
    /// Where its bytes begin in the file.
    pub at: usize,
}

impl Segment {
    /// How many bytes it carries.
    pub fn len(&self) -> usize {
        usize::from(self.end) - usize::from(self.start) + 1
    }

    /// A segment always carries at least one byte, so this is always false. It
    /// is here because clippy asks for it beside a `len`.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Is this segment one of the two OS vectors rather than program bytes?
    fn is_vector(&self) -> bool {
        (self.start == RUNAD || self.start == INITAD) && self.len() == 2
    }
}

/// A parsed binary-load file.
#[derive(Debug)]
pub struct Xex {
    segments: Vec<Segment>,
    base: u16,
    image: Vec<u8>,
    run: Option<u16>,
    init: Option<u16>,
}

/// Is `raw` an Atari binary-load file? The magic, and segments that parse and
/// account for **every** byte of it.
///
/// The exactness is what makes a two-byte signature safe: a file whose segment
/// arithmetic leaves a remainder, or overruns, is refused. No Z-machine story
/// can reach here — byte 0 of one is a version number in `1..=8` — and no file
/// in `stories/` opens `FF FF` at all.
pub fn looks_like_xex(raw: &[u8]) -> bool {
    parse(raw).is_some()
}

/// Every segment `raw` holds, or `None` when it is not a binary-load file.
///
/// The single place the container is recognised, so [`looks_like_xex`] and
/// [`Xex::parse`] cannot disagree.
fn parse(raw: &[u8]) -> Option<Vec<Segment>> {
    if raw.len() < 6 || raw[0..2] != MAGIC {
        return None;
    }
    let mut segments = Vec::new();
    let mut at = 2;
    while at < raw.len() {
        // A repeated `FF FF` between segments is allowed and carries nothing.
        if raw.len() - at >= 2 && raw[at..at + 2] == MAGIC {
            at += 2;
            continue;
        }
        if raw.len() - at < 4 {
            return None;
        }
        let start = u16::from_le_bytes([raw[at], raw[at + 1]]);
        let end = u16::from_le_bytes([raw[at + 2], raw[at + 3]]);
        if end < start {
            return None;
        }
        at += 4;
        let segment = Segment { start, end, at };
        let len = segment.len();
        if raw.len() - at < len {
            return None;
        }
        at += len;
        segments.push(segment);
    }
    // A file that is only a magic word, or only vectors, loads no program.
    segments.iter().any(|s| !s.is_vector()).then_some(segments)
}

impl Xex {
    /// Cheap sniff — see [`looks_like_xex`].
    pub fn looks_like_xex(raw: &[u8]) -> bool {
        looks_like_xex(raw)
    }

    /// Parse a binary-load file and assemble the memory it loads.
    pub fn parse(raw: &[u8]) -> Result<Xex, XexError> {
        let segments = parse(raw).ok_or(XexError::NotAXex)?;
        let payload: Vec<Segment> =
            segments.iter().copied().filter(|s| !s.is_vector()).collect();
        let base = payload.iter().map(|s| s.start).min().expect("parse kept one payload segment");
        let top = payload.iter().map(|s| s.end).max().expect("…and therefore one end");
        let mut image = vec![0u8; usize::from(top) - usize::from(base) + 1];
        // In file order, so a later segment overwrites an earlier one exactly as
        // the machine's own loader leaves memory.
        for s in &payload {
            let at = usize::from(s.start) - usize::from(base);
            image[at..at + s.len()].copy_from_slice(&raw[s.at..s.at + s.len()]);
        }
        let vector = |which: u16| {
            segments
                .iter()
                .rfind(|s| s.is_vector() && s.start == which)
                .map(|s| u16::from_le_bytes([raw[s.at], raw[s.at + 1]]))
        };
        Ok(Xex { base, image, run: vector(RUNAD), init: vector(INITAD), segments })
    }

    /// Every load record in the file, vectors included, in file order.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// The lowest address any payload segment writes — where [`Xex::image`]'s
    /// byte 0 lives. There is no fixed base for this platform (§7.5); it is read
    /// from the file, every time.
    pub fn base(&self) -> u16 {
        self.base
    }

    /// The loaded memory, `base()` upward, holes zero-filled.
    pub fn image(&self) -> &[u8] {
        &self.image
    }

    /// `RUNAD`, when the file sets it — `$9500` on `The Hulk.xex`.
    pub fn run_address(&self) -> Option<u16> {
        self.run
    }

    /// `INITAD`, when the file sets it. The last one wins, because a file may
    /// set it more than once and each is run as it is written.
    pub fn init_address(&self) -> Option<u16> {
        self.init
    }

    /// How this medium names its one entry: `$4000-$9530`, the range it loads.
    ///
    /// The same convention [`crate::d64`] and [`crate::infocom_boot`] use for a
    /// container with no filenames in it — where the payload is, because that is
    /// the only thing this medium knows about it.
    pub fn entry_name(&self) -> String {
        format!("${:04X}-${:04X}", self.base, usize::from(self.base) + self.image.len() - 1)
    }

    /// The one entry: its name and the loaded memory.
    pub fn contents(&self) -> Vec<(String, Vec<u8>)> {
        vec![(self.entry_name(), self.image.clone())]
    }

    /// One entry by the name a caller was shown, case-insensitively.
    pub fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        name.eq_ignore_ascii_case(&self.entry_name()).then(|| self.image.clone())
    }

    /// The Z-machine story this file loads, if what it loads is one.
    ///
    /// Nothing in the corpus is — the one specimen is a Scott Adams memory
    /// image — but the question is this crate's one test for what a story is,
    /// and a `.xex` carrying Z-code would be found by it like any other medium.
    pub fn story(&self) -> Option<(String, Vec<u8>)> {
        crate::adf::looks_like_story(&self.image)
            .then(|| (self.entry_name(), self.image.clone()))
    }

    /// **No artwork**, on the ground [`crate::atr`] states: no Version 6 game was
    /// pressed for an Atari 8-bit, so there is no evidence about where such a
    /// file would keep an archive, and scanning for one would be a guess.
    pub fn pictures(&self) -> Option<(String, crate::infocom_pics::InfocomPics)> {
        None
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a binary-load file from the format documentation.
    ///
    /// `pub(crate)` for [`crate::medium`]'s census, which needs a mountable
    /// sample of every format it names.
    pub(crate) fn build(segments: &[(u16, &[u8])]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        for (start, data) in segments {
            let end = start + (data.len() as u16) - 1;
            out.extend_from_slice(&start.to_le_bytes());
            out.extend_from_slice(&end.to_le_bytes());
            out.extend_from_slice(data);
        }
        out
    }

    /// One segment at `$4000` holding `data` — [`crate::medium`]'s census
    /// sample, in the shape the one real specimen has.
    pub(crate) fn sample_binary(data: &[u8]) -> Vec<u8> {
        build(&[(0x4000, data)])
    }

    /// The header arithmetic, on a file built to the documentation: inclusive
    /// end addresses, a base read out of the file, and the entry named for the
    /// range it loads.
    #[test]
    fn a_single_segment_loads_where_it_says_it_does() {
        let raw = sample_binary(&[0xaa; 16]);
        assert_eq!(raw.len(), 2 + 4 + 16);
        assert_eq!(&raw[..6], &[0xff, 0xff, 0x00, 0x40, 0x0f, 0x40]);
        let xex = Xex::parse(&raw).expect("it parses");
        assert_eq!(xex.base(), 0x4000);
        assert_eq!(xex.image(), &[0xaa; 16]);
        assert_eq!(xex.entry_name(), "$4000-$400F");
        assert_eq!(xex.segments().len(), 1);
        assert_eq!(xex.segments()[0].len(), 16, "end is inclusive");
        assert_eq!(xex.read_named("$4000-$400f").map(|b| b.len()), Some(16), "case-insensitive");
        assert_eq!(xex.read_named("$0000-$FFFF"), None);
        assert_eq!(xex.run_address(), None);
        assert_eq!(xex.init_address(), None);
    }

    /// **Vectors are not payload.** A `RUNAD` segment must not drag the base
    /// down to `$02E0` and open a fifteen-kilobyte hole — the exact shape the one
    /// real specimen would produce.
    #[test]
    fn a_run_address_segment_is_a_vector_and_not_part_of_the_image() {
        let raw = build(&[(0x4000, &[1, 2, 3, 4]), (RUNAD, &[0x00, 0x95])]);
        let xex = Xex::parse(&raw).expect("it parses");
        assert_eq!(xex.base(), 0x4000, "not $02E0");
        assert_eq!(xex.image(), &[1, 2, 3, 4]);
        assert_eq!(xex.run_address(), Some(0x9500));
        assert_eq!(xex.segments().len(), 2, "the vector is still a record in the file");

        // …and the same for INITAD, which the machine runs as it is written.
        let raw = build(&[(INITAD, &[0x34, 0x12]), (0x2000, &[9])]);
        let xex = Xex::parse(&raw).expect("it parses");
        assert_eq!(xex.base(), 0x2000);
        assert_eq!(xex.init_address(), Some(0x1234));
        assert_eq!(xex.run_address(), None);
    }

    /// Scattered segments assemble over the range they span, holes zero-filled,
    /// and a later segment wins where two overlap.
    #[test]
    fn scattered_segments_assemble_in_file_order() {
        let raw = build(&[(0x2000, &[1, 2]), (0x2006, &[3, 4]), (0x2001, &[9])]);
        let xex = Xex::parse(&raw).expect("it parses");
        assert_eq!(xex.base(), 0x2000);
        assert_eq!(xex.image(), &[1, 9, 0, 0, 0, 0, 3, 4]);
        assert_eq!(xex.entry_name(), "$2000-$2007");
    }

    /// A repeated `FF FF` between segments is DOS's own concatenation
    /// convention, and is skipped rather than read as a segment header.
    #[test]
    fn a_repeated_magic_between_segments_is_ignored() {
        let mut raw = build(&[(0x3000, &[1, 2, 3])]);
        raw.extend_from_slice(&MAGIC);
        raw.extend_from_slice(&build(&[(0x3010, &[4, 5])])[2..]);
        let xex = Xex::parse(&raw).expect("it parses");
        assert_eq!(xex.segments().len(), 2);
        assert_eq!(xex.base(), 0x3000);
        assert_eq!(xex.image().len(), 0x12);
    }

    /// **The magic alone is not enough**, and this is what makes a two-byte
    /// signature safe to run over a whole library.
    #[test]
    fn segments_must_account_for_every_byte_of_the_file() {
        let good = sample_binary(&[7; 8]);
        assert!(looks_like_xex(&good));

        let mut short = good.clone();
        short.pop();
        assert!(!looks_like_xex(&short), "the last segment overruns");

        let mut long = good.clone();
        long.push(0);
        assert!(!looks_like_xex(&long), "a trailing byte is not a segment header");

        // start > end is malformed.
        let mut backwards = good.clone();
        backwards[2..4].copy_from_slice(&0x5000u16.to_le_bytes());
        assert!(!looks_like_xex(&backwards));

        assert!(!looks_like_xex(&[]));
        assert!(!looks_like_xex(&MAGIC));
        assert!(!looks_like_xex(&[0x03, 0x00, 0x00, 0x00, 0x00, 0x00]), "a story is not one");
        // A file that is nothing but vectors loads no program.
        assert!(!looks_like_xex(&build(&[(RUNAD, &[0, 0])])));
        assert_eq!(Xex::parse(&[]).err(), Some(XexError::NotAXex));
    }

    /// The one specimen, measured. `stories/` is gitignored, so this skips
    /// vacuously on CI.
    #[test]
    fn the_hulk_loads_one_segment_at_four_thousand_and_sets_runad() {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/scott-dialects/atari/The Hulk.xex");
        let Ok(raw) = std::fs::read(&p) else {
            eprintln!("SKIP: gitignored fixture missing at {}", p.display());
            return;
        };
        assert_eq!(raw.len(), 21_821, "the file the specimen README records");
        assert!(looks_like_xex(&raw));
        let xex = Xex::parse(&raw).expect("it parses");

        // Two records: the program, and a RUNAD poke.
        assert_eq!(xex.segments().len(), 2);
        assert_eq!(xex.segments()[0].start, 0x4000);
        assert_eq!(xex.segments()[0].end, 0x9530);
        assert_eq!(xex.segments()[0].len(), 21_809);
        assert_eq!(xex.segments()[1].start, RUNAD);
        assert_eq!(xex.run_address(), Some(0x9500));
        assert_eq!(xex.init_address(), None);

        // The image is the program alone, based where it loads.
        assert_eq!(xex.base(), 0x4000);
        assert_eq!(xex.image().len(), 21_809);
        assert_eq!(xex.entry_name(), "$4000-$9530");
        assert_eq!(xex.contents().len(), 1);
        assert_eq!(xex.contents()[0].0, "$4000-$9530");

        // `scott-dialects-spec.md` §4.1's plain four-letter dictionary
        // signature, at the file offset the README records — and therefore at
        // the image offset and the address this reader's base implies.
        const SIGNATURE: &[u8] = b"AUTO\0GO\0";
        assert_eq!(raw.windows(8).position(|w| w == SIGNATURE), Some(0x2cc4));
        let at = xex.image().windows(8).position(|w| w == SIGNATURE).expect("in the image too");
        // Six bytes of magic and segment header in front of the payload, so the
        // image offset is the file offset less six…
        assert_eq!(at, 0x2cc4 - 6);
        // …and the ADDRESS is what a loader actually wants, which is why the
        // base is read off the file rather than assumed.
        assert_eq!(usize::from(xex.base()) + at, 0x6cbe, "address $6CBE");

        // It is not Z-code, and says so rather than being offered as a story.
        assert_eq!(xex.story(), None);
        assert!(xex.pictures().is_none());
    }
}
