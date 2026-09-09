//! Reads the **TI-99/4A tokenised releases** of the Adventure International
//! games into the same [`Database`] every other Scott Adams dialect decodes
//! to.
//!
//! The file is a raw memory image based at address `0x0380`, with a
//! fixed-size character-pattern block whose contents give a stable detection
//! signature, a 34-byte header at a constant distance from it, and eleven
//! table pointers in that header. Nothing in the dialect is per-release and
//! nothing is bit-packed, which makes it the only dialect in
//! `docs/internals/scott-dialects-spec.md` that a loader can read with no
//! game-specific knowledge at all.
//!
//! # Provenance
//!
//! Everything here is implemented from
//! [`docs/internals/scott-dialects-spec.md`](../../../docs/internals/scott-dialects-spec.md)
//! §3, an independent functional description of the format written under the
//! clean-room protocol in `docs/internals/clean-room.md`, plus measurement on
//! the specimens that document's §10.2 names. **No GPL interpreter's source
//! was read to write this module** — lanthorn is BSD-3-Clause and every
//! established Scott Adams interpreter is GPL, so the specification is the
//! only channel by which a fact about this format reached this file. Each
//! rule below cites the specification section it implements.
//!
//! # What a TI-99/4A database differs in
//!
//! Every table decodes to the reference format's shape — rooms with six
//! exits, items with a start location and a take/drop noun, a flat verb and
//! noun vocabulary, a message pool — with **one exception, which is
//! structural rather than cosmetic**: the action script. A reference-format
//! action is a fixed record of five conditions and four commands. A
//! tokenised record is a variable-length opcode stream in which conditions
//! and commands interleave freely, operands are inline, and a failure
//! handler gives the format an if/else the reference format has no
//! equivalent for (spec §3.7). Measured over the twelve specimens, a single
//! record reaches **25 conditions and 37 commands** (Savage Island part I)
//! and 89 records across the corpus push a failure handler — so a tokenised
//! record cannot be re-expressed as an [`Action`](crate::Action), and this
//! module produces a [`Ti99Script`] instead, which [`crate::Vm`] executes in
//! place of [`Database::actions`] (left empty for a database from this
//! dialect).
//!
//! [`Database`]: crate::Database

use crate::database::CARRIED;
use crate::loader::{Dialect, LoadError};
use crate::{Database, Item, Room};

/// The ten-byte detection signature, searched for anywhere in the file with
/// the first occurrence winning (spec §3.1). It is not part of any game
/// table — it falls inside the fixed-size character-pattern block every
/// release of this dialect carries between its title screen and its data
/// header, and its value is a run of glyph bitmaps. What makes it usable is
/// that the distance from it to the data header is constant.
pub const SIGNATURE: [u8; 10] = [0x30, 0x30, 0x30, 0x30, 0x00, 0x30, 0x30, 0x00, 0x28, 0x28];

/// Distance from the first byte of [`SIGNATURE`] back to the **baseline**
/// *B*, the value that absorbs whatever container prefix the file carries
/// (spec §3.1). `B = signature_offset - 0x589`.
const SIGNATURE_TO_BASELINE: usize = 0x589;

/// Distance from the baseline to the 34-byte data header (spec §3.1).
const HEADER_FROM_BASELINE: usize = 0x8A0;

/// The data header's length in bytes (spec §3.3).
const HEADER_LEN: usize = 34;

/// The address the memory image is based at: a stored address *A* resolves
/// to file offset `A - 0x380 + B` (spec §3.1).
const IMAGE_BASE: i64 = 0x380;

/// The number of table pointers the header carries (spec §3.3).
const POINTER_COUNT: usize = 11;

/// A chunk length of 0, or greater than this, marks a string as malformed
/// (spec §3.5).
const MAX_CHUNK_LEN: u8 = 100;

/// A dictionary word of this many characters or more is rejected, leaving
/// the slot empty (spec §3.6).
const MAX_WORD_LEN: usize = 20;

/// The conventional filler a reader pads the shorter of the two
/// dictionaries with, so both can be indexed over the same range
/// (spec §3.6). A single period can never match a typed word.
const DICTIONARY_FILLER: &str = ".";

/// What a string that decodes to nothing is replaced by, because the
/// interpreter's description and message paths treat an empty description
/// as an error and a leading period as "nothing to show" (spec §3.5).
const EMPTY_STRING_PLACEHOLDER: &str = ".";

/// One tokenised action record: the byte that decides whether it applies,
/// and the opcode stream that runs when it does (spec §3.7).
///
/// Records are variable length and terminated in band. Byte 1 of the stored
/// record is a **link** to the next record, not an extent — 0 means only
/// that no record follows. **A record whose link is zero is the last of its
/// chain and is a real record in every other respect**, eligible to match
/// and to run, and its opcode stream is not empty: it begins at byte 2 like
/// every other record's and ends at its own end-of-record opcode 255. This
/// field always holds that recovered stream, whichever way the record's
/// extent was determined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ti99Record {
    /// For an explicit (verb-triggered) record, the noun index this record
    /// matches; 0 means "any noun". For an implicit (automatic) record, the
    /// percentage chance, 0 to 100, that the record runs on a given turn
    /// (spec §3.7).
    pub key: u8,
    /// The opcode stream, exactly as stored. Opcodes and operands are whole
    /// bytes told apart by numeric range; nothing here is bit-packed
    /// (spec §3.7). Kept raw rather than pre-decoded so that an unassigned
    /// opcode — whose operand count is unknown, making the rest of the
    /// record undecodable (spec §11) — is a record the interpreter
    /// abandons at run time rather than a file the loader refuses.
    pub ops: Vec<u8>,
}

/// The action script of a TI-99/4A release: one chain of [`Ti99Record`]s per
/// verb, plus the single chain of automatic records (spec §3.7).
///
/// This is the one table that does **not** decode to the reference format's
/// shape; see the module docs for why it cannot. [`crate::Vm`] executes it
/// in place of [`crate::Database::actions`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ti99Script {
    /// Indexed by verb number: the records of that verb's chain, in the
    /// order the interpreter must walk them. An empty vector means the verb
    /// has no records (a zero dispatch entry), which is one of the three
    /// outcomes spec §9.1 requires a caller to tell apart.
    ///
    /// Sized to the vocabulary the [`crate::Database`] ended up with, not to
    /// the header's highest verb index: a verb index beyond that index is
    /// reachable, because the effective word count is the larger of the two
    /// dictionary counts (spec §3.7), and the surplus slots are empty
    /// chains rather than a bounds check every caller has to remember.
    pub verb_chains: Vec<Vec<Ti99Record>>,
    /// The implicit (automatic) chain, every record of which is visited once
    /// per turn with a percentage roll against its `key` deciding whether
    /// its stream runs. Empty when the game has no automatic actions at all
    /// (spec §3.7: a leading zero byte in the implicit block).
    pub automatic: Vec<Ti99Record>,
}

/// Where in a file the TI-99/4A tables were found: the baseline *B* and the
/// offset the signature matched at (spec §3.1). Returned by
/// [`locate`] so a caller can report what it recognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Anchor {
    /// The baseline *B*: `signature_offset - 0x589`.
    baseline: usize,
    /// File offset of the 34-byte data header.
    header: usize,
}

/// Finds the detection signature and derives the baseline (spec §3.1).
///
/// Returns `None` when the signature is absent, when the baseline would be
/// negative, or when the header does not lie wholly within the file — all
/// of which mean "not this dialect" rather than "a corrupt file", because
/// another dialect's detector may yet have to run.
fn locate(bytes: &[u8]) -> Option<Anchor> {
    let sig = bytes
        .windows(SIGNATURE.len())
        .position(|w| w == SIGNATURE)?;
    let baseline = sig.checked_sub(SIGNATURE_TO_BASELINE)?;
    let header = baseline.checked_add(HEADER_FROM_BASELINE)?;
    if header.checked_add(HEADER_LEN)? > bytes.len() {
        return None;
    }
    Some(Anchor { baseline, header })
}

/// The header's fields, read as spec §3.3 lays them out. Every "number of"
/// value is the **highest valid index**, not a cardinality, exactly as in
/// the reference text format (spec §2.2).
#[derive(Debug, Clone, Copy)]
struct Header {
    /// Highest item index (header offset 0).
    max_item: usize,
    /// Highest verb index (offset 1).
    max_verb: usize,
    /// Highest noun index (offset 2).
    max_noun: usize,
    /// The "red room" — the room a dead player is moved to, which is also
    /// the highest room index (offset 3).
    max_room: usize,
    /// Maximum items carried (offset 4).
    max_carry: i32,
    /// Starting room (offset 5).
    start_room: usize,
    /// Significant word length (offset 7).
    word_length: usize,
    /// Light-source duration in turns, big-endian (offset 8). `0xFFFF` is
    /// read as −1, "never runs out" (spec §9.2).
    light_time: i32,
    /// Treasure room (offset 10).
    treasure_room: usize,
    /// The eleven table pointers (offsets 12 through 33), in header order.
    pointers: [u16; POINTER_COUNT],
}

/// Index of the initial-item-locations pointer within [`Header::pointers`].
/// Pointer 0 is the object table, which is **declared but of unknown
/// layout** and must be left alone entirely (spec §3.3, §11) — it is
/// validated as an in-range address and never read.
const P_ITEM_LOCATIONS: usize = 1;
/// Index of the noun-to-item link table pointer.
const P_NOUN_LINKS: usize = 2;
/// Index of the item description pointer table's pointer.
const P_ITEM_DESCRIPTIONS: usize = 3;
/// Index of the message pointer table's pointer.
const P_MESSAGES: usize = 4;
/// Index of the room exit table's pointer.
const P_ROOM_EXITS: usize = 5;
/// Index of the room description pointer table's pointer.
const P_ROOM_DESCRIPTIONS: usize = 6;
/// Index of the noun pointer table's pointer.
const P_NOUNS: usize = 7;
/// Index of the verb pointer table's pointer.
const P_VERBS: usize = 8;
/// Index of the explicit (verb-triggered) action dispatch table's pointer.
const P_EXPLICIT_ACTIONS: usize = 9;
/// Index of the implicit (automatic) action block's pointer.
const P_IMPLICIT_ACTIONS: usize = 10;

/// A cursor over the memory image that resolves **stored addresses** to file
/// offsets and bounds-checks every read.
///
/// Every multi-byte quantity in this dialect is big-endian (spec §3.2), and
/// a stored address *A* resolves to file offset `A - 0x380 + B` (spec §3.1);
/// both facts live here so no caller re-derives either.
struct Image<'a> {
    bytes: &'a [u8],
    baseline: usize,
}

impl<'a> Image<'a> {
    /// File offset of stored address `addr`, or `None` when it falls
    /// outside the file. A pointer that does not resolve means "not this
    /// dialect" during detection and a malformed file afterwards
    /// (spec §3.1).
    fn resolve(&self, addr: u16) -> Option<usize> {
        let off = i64::from(addr) - IMAGE_BASE + self.baseline as i64;
        let off = usize::try_from(off).ok()?;
        (off < self.bytes.len()).then_some(off)
    }

    /// The byte at file offset `off`.
    fn byte(&self, off: usize) -> Option<u8> {
        self.bytes.get(off).copied()
    }

    /// The big-endian 16-bit word at file offset `off`, most significant
    /// byte first (spec §3.2).
    fn word(&self, off: usize) -> Option<u16> {
        let hi = self.byte(off)?;
        let lo = self.byte(off + 1)?;
        Some(u16::from(hi) << 8 | u16::from(lo))
    }

    /// `count` consecutive bytes starting at stored address `addr`.
    fn byte_array(&self, addr: u16, count: usize) -> Option<&'a [u8]> {
        let off = self.resolve(addr)?;
        self.bytes.get(off..off.checked_add(count)?)
    }

    /// Entry `index` of the pointer table based at stored address `addr` —
    /// an array of big-endian 16-bit stored addresses (spec §3.4).
    fn table_entry(&self, addr: u16, index: usize) -> Option<u16> {
        let off = self.resolve(addr)?.checked_add(index.checked_mul(2)?)?;
        self.word(off)
    }

    /// Decodes the string whose extent runs from stored address `start` up
    /// to (but not including) stored address `end` (spec §3.5).
    ///
    /// Within the extent the bytes form a sequence of length-prefixed
    /// chunks: one byte giving a character count *L*, then exactly *L* bytes
    /// of characters, then the next chunk. Chunks are consumed until the
    /// extent is exhausted, and the decoded string is the chunks'
    /// characters concatenated **with a single space between consecutive
    /// chunks** — the separator is implied, never stored, and no space
    /// follows the last chunk. There is no terminator byte and no
    /// compression.
    ///
    /// A chunk length of 0, or greater than 100, marks the string as
    /// malformed and yields `None`.
    fn string(&self, start: u16, end: u16) -> Option<String> {
        let mut pos = self.resolve(start)?;
        let stop = self.resolve(end)?;
        if stop < pos {
            return None;
        }
        let mut out = String::new();
        let mut first = true;
        while pos < stop {
            let len = self.byte(pos)?;
            if len == 0 || len > MAX_CHUNK_LEN {
                return None;
            }
            let from = pos + 1;
            let to = from.checked_add(usize::from(len))?;
            if to > stop {
                return None;
            }
            if !first {
                out.push(' ');
            }
            first = false;
            for &b in self.bytes.get(from..to)? {
                push_char(&mut out, b);
            }
            pos = to;
        }
        Some(out)
    }
}

/// Appends the display form of one stored character byte (spec §3.5).
///
/// Characters are 7-bit ASCII with four display substitutions specific to
/// this dialect: byte `0x40` renders as a copyright sign **followed by a
/// space** (two output characters from one stored byte); `0x7B` renders as
/// `ä`; `0x7D` renders as `ü`; `0x0C` renders as `ö`. A byte outside the
/// printable ASCII range that is none of those four has no defined
/// rendering; it is passed through as its Latin-1 character rather than
/// failing the load, the same tolerance the reference-format lexer applies
/// to a `.dat` with bytes above `0x7F` (spec §2.6's non-ASCII rule).
///
/// None of the four substitution bytes occurs in any game table of the
/// twelve specimens (measured over every room, item, message and dictionary
/// entry of `adv01.fiad`-`adv12.fiad`); the hand-built fixture in this
/// module's tests is what exercises them.
fn push_char(out: &mut String, b: u8) {
    match b {
        0x40 => out.push_str("\u{00A9} "),
        0x7B => out.push('\u{00E4}'),
        0x7D => out.push('\u{00FC}'),
        0x0C => out.push('\u{00F6}'),
        _ => out.push(b as char),
    }
}

/// Reads the 34-byte header at `anchor.header` (spec §3.3) and validates it
/// (spec §3.1).
fn read_header(image: &Image, anchor: Anchor) -> Result<Header, LoadError> {
    let at = |i: usize| -> Result<u8, LoadError> {
        image.byte(anchor.header + i).ok_or(bad("header truncated"))
    };
    let mut pointers = [0u16; POINTER_COUNT];
    for (i, slot) in pointers.iter_mut().enumerate() {
        *slot = image
            .word(anchor.header + 12 + i * 2)
            .ok_or(bad("header truncated"))?;
    }
    // Spec §3.1's validation before acceptance: all eleven pointers must
    // resolve to offsets within the file. Pointer 0 (the object table) is
    // validated here and never read — its layout is unknown (spec §11).
    for p in pointers {
        if image.resolve(p).is_none() {
            return Err(bad("a header table pointer resolves outside the file"));
        }
    }
    let light = image
        .word(anchor.header + 8)
        .ok_or(bad("header truncated"))?;
    Ok(Header {
        max_item: usize::from(at(0)?),
        max_verb: usize::from(at(1)?),
        max_noun: usize::from(at(2)?),
        max_room: usize::from(at(3)?),
        max_carry: i32::from(at(4)?),
        start_room: usize::from(at(5)?),
        // Header offset 6 is a treasure count that is present but NOT
        // authoritative: it is ignored in favour of counting items whose
        // description begins with `*`, since the two are not guaranteed to
        // agree (spec §3.3). Header offset 11 is unassigned; no meaning is
        // known (spec §3.3, §11), and it is not read.
        word_length: usize::from(at(7)?),
        light_time: if light == u16::MAX {
            -1
        } else {
            i32::from(light)
        },
        treasure_room: usize::from(at(10)?),
        pointers,
    })
}

/// A malformed-file refusal that names what did not check out.
fn bad(what: &'static str) -> LoadError {
    LoadError::BadDialectData(Dialect::Ti994aBytecode, what)
}

/// Reads a run of strings addressed by a pointer table (spec §3.4, §3.5).
///
/// Entry *i* gives the start of string *i* and **entry *i* + 1 gives its
/// end**: there is no length field and no terminator, so every extent is the
/// difference between two consecutive pointers and the table carries one
/// entry beyond its highest index as an end sentinel.
///
/// `max_index` is the header's highest index, which is authoritative where
/// it disagrees with what the pointer table can supply (spec §3.5). A slot
/// the table cannot address — the sentinel is missing, or the extent is
/// malformed — becomes the single-period placeholder rather than failing the
/// load, because a failed decode past the end must never propagate as a hard
/// error (spec §3.5).
fn read_strings(image: &Image, table: u16, max_index: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(max_index + 1);
    for i in 0..=max_index {
        let decoded = image
            .table_entry(table, i)
            .zip(image.table_entry(table, i + 1))
            .and_then(|(start, end)| image.string(start, end))
            .filter(|s| !s.is_empty());
        out.push(decoded.unwrap_or_else(|| EMPTY_STRING_PLACEHOLDER.to_string()));
    }
    out
}

/// Reads a dictionary from a pointer table (spec §3.6).
///
/// Words are stored as bare characters, with no terminator and no padding;
/// each word's length is the difference between consecutive pointers, which
/// is why the table must contain at least `max_index + 2` entries. A word of
/// 20 characters or more is rejected, leaving the slot empty; a zero-length
/// word (two equal consecutive pointers) is likewise an empty entry rather
/// than something that stalls a reader.
fn read_words(image: &Image, table: u16, max_index: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(max_index + 1);
    for i in 0..=max_index {
        let word = (|| {
            let start = image.resolve(image.table_entry(table, i)?)?;
            let end = image.resolve(image.table_entry(table, i + 1)?)?;
            let span = image.bytes.get(start..end)?;
            if span.is_empty() || span.len() >= MAX_WORD_LEN {
                return None;
            }
            let mut w = String::with_capacity(span.len());
            for &b in span {
                push_char(&mut w, b);
            }
            Some(w)
        })();
        out.push(word.unwrap_or_default());
    }
    out
}

/// Derives the number of words in a pointer table from its own first entry
/// (spec §3.5).
///
/// Resolve the table's address, read its first entry, resolve that, and take
/// the byte difference divided by two. Because the first entry points at the
/// first byte after the table itself (spec §3.4), that difference is the
/// table's own length; and because the last word is an end sentinel, the
/// real entries are numbered 0 through *N* − 2.
fn derived_max_index(image: &Image, table: u16) -> Option<usize> {
    let base = image.resolve(table)?;
    let first = image.resolve(image.word(base)?)?;
    let words = first.checked_sub(base)? / 2;
    words.checked_sub(2)
}

/// Walks one chain of action records from stored address `addr`
/// (spec §3.7).
///
/// Each record is a key byte, a link byte *L* to the next record (0 meaning
/// none follows), and then an opcode stream starting at byte 2. For a
/// non-zero link the stream's extent is implied by the link — it runs up to
/// the next record's start, at the current record's start plus 1 + *L* — and
/// the next record begins there. **A record whose link is zero is the last
/// of the chain** and is a real record in every other respect: its stream is
/// recovered by walking the opcode arities to its own end-of-record opcode
/// 255 ([`walk_ti99_ops`]), since there is no next record's start to imply
/// it.
///
/// `implicit` selects the automatic block's one extra rule: if the very
/// first byte there is zero the game has no automatic actions at all and the
/// block must not be walked, which makes a genuine leading 0%-chance record
/// unrepresentable — the empty reading is the correct one.
fn read_chain(image: &Image, addr: u16, implicit: bool) -> Result<Vec<Ti99Record>, LoadError> {
    let mut pos = image
        .resolve(addr)
        .ok_or(bad("an action chain starts outside the file"))?;
    let mut out = Vec::new();
    if implicit && image.byte(pos) == Some(0) {
        return Ok(out);
    }
    loop {
        let key = image
            .byte(pos)
            .ok_or(bad("an action record runs past the end of the file"))?;
        let link = image
            .byte(pos + 1)
            .ok_or(bad("an action record runs past the end of the file"))?;
        if link == 0 {
            let ops = walk_ti99_ops(image, pos + 2)?;
            out.push(Ti99Record { key, ops });
            return Ok(out);
        }
        let from = pos + 2;
        let to = pos + 1 + usize::from(link);
        let ops = image
            .bytes
            .get(from..to)
            .ok_or(bad("an action record runs past the end of the file"))?;
        out.push(Ti99Record {
            key,
            ops: ops.to_vec(),
        });
        // Every step is strictly forward (`link` is non-zero here), so the
        // walk is bounded by the file length and cannot cycle.
        pos = to;
    }
}

/// Recovers a link-0 record's opcode stream by walking its opcode arities
/// from file offset `from` (the record's byte 2) to its own end-of-record
/// opcode 255, since there is no next record's start to imply the extent
/// (spec §3.7).
///
/// Message opcodes (0-182) and the two zero-operand conditions (195, 196)
/// occupy one byte; every other condition (183-201) two; a command's operand
/// count is [`crate::vm::ti99_command_operands`], the same table the
/// interpreter runs the stream through, so the two can never disagree about
/// where a record ends. An unassigned opcode (202-211, 213) makes the rest of
/// the record undecodable (spec §11) and stops the walk there, exactly as the
/// interpreter abandons the record when it meets one; the byte itself is
/// still included, since the interpreter reads it before bailing.
fn walk_ti99_ops(image: &Image, from: usize) -> Result<Vec<u8>, LoadError> {
    let mut pos = from;
    loop {
        let op = image
            .byte(pos)
            .ok_or(bad("an action record runs past the end of the file"))?;
        pos += 1;
        let operands = match op {
            255 | 202..=211 | 213 => break,
            0..=182 => 0,
            195 | 196 => 0,
            183..=201 => 1,
            _ => crate::vm::ti99_command_operands(op),
        };
        if image.bytes.get(pos..pos + operands).is_none() {
            return Err(bad("an action record runs past the end of the file"));
        }
        pos += operands;
    }
    image
        .bytes
        .get(from..pos)
        .map(<[u8]>::to_vec)
        .ok_or(bad("an action record runs past the end of the file"))
}

/// Whether `bytes` are a TI-99/4A tokenised release, by spec §3.1's
/// "validation before acceptance": the signature is present, the baseline it
/// implies is not negative, the header lies wholly within the file, and all
/// eleven header pointers resolve to offsets within the file.
///
/// Cheap — it reads 34 bytes and eleven addresses, not the game — and
/// deliberately the same predicate [`parse_ti994a`] applies a moment later,
/// so a host that sniffs with this and then loads cannot be told "yes" and
/// then handed a refusal. A `false` here means "not this dialect", which is
/// what spec §3.1 asks a detector to conclude rather than "a corrupt file".
pub fn looks_like_ti994a(bytes: &[u8]) -> bool {
    let Some(anchor) = locate(bytes) else {
        return false;
    };
    let image = Image {
        bytes,
        baseline: anchor.baseline,
    };
    read_header(&image, anchor).is_ok()
}

/// Parses a TI-99/4A tokenised release into a [`Database`].
///
/// Implements `docs/internals/scott-dialects-spec.md` §3 in full: the
/// signature scan and baseline rule (§3.1), the 34-byte header (§3.3), the
/// flat and pointer table shapes (§3.4), the chunked string encoding and the
/// derived message count (§3.5), the two separate dictionaries (§3.6), and
/// the tokenised action encoding (§3.7). Multi-byte quantities are
/// big-endian throughout (§3.2).
///
/// The resulting [`Database`] carries every table in the reference format's
/// own shape except the action script, which becomes a
/// [`Ti99Script`] on
/// [`Database::ti99`] with [`Database::actions`] left empty — see the module
/// docs for why the two cannot be the same table.
///
/// # Errors
///
/// [`LoadError::BadDialectData`] naming what did not check out, when the
/// signature is absent, the baseline would be negative, the header does not
/// lie wholly within the file, a header pointer resolves outside the file, or
/// a table runs past the end. Per spec §3.1 and §11 a *detector* chaining
/// several dialects should read those first four as "not this dialect"
/// rather than "a corrupt file"; this crate reaches here only after
/// [`crate::detect_dialect`] has already answered
/// [`Dialect::Ti994aBytecode`], so it reports them as the malformed file
/// they then are.
pub fn parse_ti994a(bytes: &[u8]) -> Result<Database, LoadError> {
    let anchor = locate(bytes).ok_or(bad(
        "no TI-99/4A signature with a usable baseline and an in-file header",
    ))?;
    let image = Image {
        bytes,
        baseline: anchor.baseline,
    };
    let h = read_header(&image, anchor)?;

    // --- Dictionaries (spec §3.6). Verbs and nouns live in two entirely
    // separate pointer tables located through two header pointers; they are
    // not interleaved as the reference format's are.
    let mut verbs = read_words(&image, h.pointers[P_VERBS], h.max_verb);
    let mut nouns = read_words(&image, h.pointers[P_NOUNS], h.max_noun);
    // "A reader producing a reference-format database must pad the shorter
    // dictionary up to the longer one's length with entries that can never
    // match" (spec §3.6).
    let vocabulary = verbs.len().max(nouns.len());
    verbs.resize(vocabulary, DICTIONARY_FILLER.to_string());
    nouns.resize(vocabulary, DICTIONARY_FILLER.to_string());

    // --- Rooms (spec §3.4 exits, §3.5 descriptions). Six bytes per room for
    // rooms 0 through the highest room index: north, south, east, west, up,
    // down, in that order; 0 means no exit.
    let exit_bytes = image
        .byte_array(h.pointers[P_ROOM_EXITS], (h.max_room + 1) * 6)
        .ok_or(bad("the room exit table runs past the end of the file"))?;
    let room_text = read_strings(&image, h.pointers[P_ROOM_DESCRIPTIONS], h.max_room);
    let mut rooms = Vec::with_capacity(h.max_room + 1);
    for (i, chunk) in exit_bytes.as_chunks::<6>().0.iter().enumerate() {
        let mut exits = [0usize; 6];
        for (slot, &b) in exits.iter_mut().zip(chunk) {
            // An exit naming a room outside the table would index past the
            // room list at run time; the reference-format loader rejects the
            // file over it, and this dialect's own tables never do it (no
            // exit byte in any of the twelve specimens exceeds its room
            // count), so treating it as "no exit" keeps a corrupt image
            // playable rather than unloadable.
            *slot = if usize::from(b) <= h.max_room {
                usize::from(b)
            } else {
                0
            };
        }
        // The text format's leading-asterisk convention survives: a room
        // whose decoded description begins with `*` is printed literally,
        // without the interpreter's "I'm in a" prefix, and the asterisk
        // itself is not printed (spec §3.5, §2.3).
        let raw = &room_text[i];
        let literal = raw.starts_with('*');
        let desc = if literal {
            raw[1..].to_string()
        } else {
            raw.clone()
        };
        rooms.push(Room {
            exits,
            desc,
            literal,
        });
    }

    // --- Items (spec §3.4 locations and noun links, §3.5 descriptions).
    let locations = image
        .byte_array(h.pointers[P_ITEM_LOCATIONS], h.max_item + 1)
        .ok_or(bad("the item location table runs past the end of the file"))?;
    let links = image
        .byte_array(h.pointers[P_NOUN_LINKS], h.max_item + 1)
        .ok_or(bad("the noun-to-item link table runs past the end of the file"))?;
    let item_text = read_strings(&image, h.pointers[P_ITEM_DESCRIPTIONS], h.max_item);
    let mut items = Vec::with_capacity(h.max_item + 1);
    for i in 0..=h.max_item {
        let text = item_text[i].clone();
        // The text format's other leading-asterisk convention also survives:
        // an item whose decoded description begins with `*` is a treasure
        // and the asterisk IS printed (spec §3.5, §2.3), so unlike a room's
        // it is not stripped.
        let treasure = text.starts_with('*');
        // The link table is this dialect's spelling of the text format's
        // trailing `/WORD/` marker: the word is a dictionary index held out
        // of line, and the item's description carries no marker at all
        // (spec §3.4). A non-zero byte names the noun; zero means no name.
        // The stored word may itself be a `*`-marked synonym, so the marker
        // is stripped and `Database::match_noun` folds it to the canonical
        // index the same way it folds a typed word (spec §3.4, §3.6).
        let auto_noun = match usize::from(links[i]) {
            0 => None,
            n => nouns
                .get(n)
                .map(|w| w.trim_start_matches('*').to_string())
                .filter(|w| !w.is_empty()),
        };
        // Spec §3.4 states that this dialect has no in-band value meaning
        // "carried". MEASUREMENT CONTRADICTS IT: byte 255 occurs in five of
        // the twelve specimens, on exactly the items whose `.dat` twins give
        // location −1 or 255, which is the reference format's own "carried"
        // marker — adv03 item 48 (the implanted bomb detector), adv05 item
        // 16 (Tent STAKE), adv07 items 3/25/56 (Shoes, Watch, gum), adv08
        // items 4/8 (canteen, flashlite), adv10 item 17 (Watch). Reading 255
        // as a room number would start those games with the player's own
        // possessions in a room that does not exist. It is normalised to
        // `CARRIED` exactly as the `.dat` loader normalises 255, so that
        // conditions 17/18 ("item still/not in its initial room") still
        // compare equal after a programmatic take.
        let start_loc = if locations[i] == 255 {
            CARRIED
        } else {
            i32::from(locations[i])
        };
        items.push(Item {
            text,
            treasure,
            auto_noun,
            start_loc,
        });
    }

    // --- Messages (spec §3.5). The count is derived, not stored.
    let max_message = derived_max_index(&image, h.pointers[P_MESSAGES])
        .ok_or(bad("the message pointer table does not describe its own length"))?;
    let messages = read_strings(&image, h.pointers[P_MESSAGES], max_message);

    // --- Actions (spec §3.7).
    let mut verb_chains = vec![Vec::new(); vocabulary];
    let dispatch = h.pointers[P_EXPLICIT_ACTIONS];
    for (verb, chain) in verb_chains.iter_mut().enumerate().take(h.max_verb + 1) {
        // The dispatch table has one entry per verb index and no sentinel;
        // each word stands alone, and a word of zero means the verb has no
        // records (spec §3.7). An entry that does not lie within the file is
        // read the same way: `adv07.fiad` ends two bytes before its last
        // verb's entry, the only one of the twelve specimens whose image is
        // short of what its own header describes, and refusing the whole
        // game over a verb with no records would be the wrong trade.
        let head = match image.table_entry(dispatch, verb) {
            Some(0) | None => continue,
            Some(a) => a,
        };
        *chain = read_chain(&image, head, false)?;
    }
    let automatic = read_chain(&image, h.pointers[P_IMPLICIT_ACTIONS], true)?;

    // The header's treasure count is present but not authoritative
    // (spec §3.3): count items whose description begins with `*` instead.
    let num_treasures = items.iter().filter(|i| i.treasure).count() as i32;

    if h.start_room >= rooms.len() {
        return Err(bad("the starting room does not index a real room"));
    }

    Ok(Database {
        max_carry: h.max_carry,
        start_room: h.start_room,
        num_treasures,
        word_length: h.word_length,
        light_time: h.light_time,
        treasure_room: h.treasure_room,
        // No reference-format action table exists in this dialect; the
        // script below is what `Vm` runs instead (see the module docs).
        actions: Vec::new(),
        verbs,
        nouns,
        rooms,
        messages,
        items,
        // The trailer the reference text format carries an adventure number
        // in has no counterpart here.
        adventure_number: 0,
        mysterious: false,
        ti99: Some(Ti99Script {
            verb_chains,
            automatic,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, StepResult, Vm};

    /// Builds a TI-99/4A image byte for byte from spec §3's format
    /// description — NOT by re-encoding anything this loader produced, and
    /// not by copying bytes out of a specimen. Two rooms, three items (one
    /// of them a treasure with a noun link, one carried from the start), a
    /// three-word verb dictionary and a four-word noun dictionary of
    /// different lengths, two messages, one explicit action chain and one
    /// automatic record.
    ///
    /// The image is laid out at the stored addresses named below, with the
    /// baseline `B` = 0 so a stored address `A` sits at file offset
    /// `A - 0x380`.
    struct Fixture {
        bytes: Vec<u8>,
    }

    /// Stored address of the first table. Everything the fixture writes
    /// lives above the 34-byte header at `B + 0x8A0`, i.e. above stored
    /// address `0x8A0 + 0x380 = 0xC20`.
    const F_BASE: u16 = 0x0D00;

    impl Fixture {
        fn new() -> Fixture {
            // The image is addressed from 0x380, and the header sits at file
            // offset 0x8A0. Reserve enough room for the signature block, the
            // header and the tables.
            let mut bytes = vec![0u8; 0x1400];
            // Detection: the signature at file offset 0x589 gives B = 0
            // (spec §3.1), which is where every distributed TI-99/4A file
            // carrying the standard 128-byte file-descriptor header puts it.
            bytes[0x589..0x589 + SIGNATURE.len()].copy_from_slice(&SIGNATURE);
            Fixture { bytes }
        }

        fn put(&mut self, addr: u16, data: &[u8]) {
            let off = usize::from(addr) - IMAGE_BASE as usize;
            self.bytes[off..off + data.len()].copy_from_slice(data);
        }

        fn header(&mut self, fields: [u8; 12], pointers: [u16; POINTER_COUNT]) {
            let h = HEADER_FROM_BASELINE;
            self.bytes[h..h + 12].copy_from_slice(&fields);
            for (i, p) in pointers.iter().enumerate() {
                self.bytes[h + 12 + i * 2] = (p >> 8) as u8;
                self.bytes[h + 12 + i * 2 + 1] = *p as u8;
            }
        }
    }

    /// One length-prefixed chunk (spec §3.5).
    fn chunk(s: &str) -> Vec<u8> {
        let mut v = vec![s.len() as u8];
        v.extend_from_slice(s.as_bytes());
        v
    }

    /// A pointer table plus the data it addresses, laid out so that the
    /// first entry points at the first byte after the table itself — which
    /// is what makes the derived count of spec §3.5 work. `items` are the
    /// already-encoded byte runs, one per entry; the table gets one extra
    /// end-sentinel entry (spec §3.4).
    fn pointer_table(base: u16, runs: &[Vec<u8>]) -> Vec<u8> {
        let table_len = (runs.len() + 1) * 2;
        let mut out = Vec::new();
        let mut addr = base + table_len as u16;
        for run in runs {
            out.push((addr >> 8) as u8);
            out.push(addr as u8);
            addr += run.len() as u16;
        }
        out.push((addr >> 8) as u8);
        out.push(addr as u8);
        for run in runs {
            out.extend_from_slice(run);
        }
        out
    }

    /// The fixture's stored addresses, chosen so no table overlaps another.
    const A_ITEM_LOC: u16 = F_BASE;
    const A_LINKS: u16 = F_BASE + 0x10;
    const A_EXITS: u16 = F_BASE + 0x20;
    const A_ROOMS: u16 = F_BASE + 0x40;
    const A_ITEMS: u16 = F_BASE + 0x100;
    const A_MESSAGES: u16 = F_BASE + 0x200;
    const A_VERBS: u16 = F_BASE + 0x300;
    const A_NOUNS: u16 = F_BASE + 0x380;
    const A_EXPLICIT: u16 = F_BASE + 0x400;
    const A_CHAIN: u16 = F_BASE + 0x420;
    const A_RUB_CHAIN: u16 = F_BASE + 0x430;
    const A_IMPLICIT: u16 = F_BASE + 0x440;
    const A_OBJECTS: u16 = F_BASE + 0x460;

    fn build() -> Vec<u8> {
        let mut f = Fixture::new();

        // Header (spec §3.3): highest item 2, highest verb 2, highest noun
        // 3, highest room 1, carry 4, start room 1, treasure count 9 (a
        // deliberate lie — spec §3.3 says it is not authoritative and the
        // loader must count `*` texts instead, which gives 1), word length
        // 3, light 0x0064 = 100 turns, treasure room 1, unassigned 0.
        f.header(
            [2, 2, 3, 1, 4, 1, 9, 3, 0x00, 0x64, 1, 0],
            [
                A_OBJECTS, A_ITEM_LOC, A_LINKS, A_ITEMS, A_MESSAGES, A_EXITS, A_ROOMS, A_NOUNS,
                A_VERBS, A_EXPLICIT, A_IMPLICIT,
            ],
        );

        // Initial item locations, one byte per item; 255 means carried.
        f.put(A_ITEM_LOC, &[0, 1, 255]);
        // Noun-to-item links, one byte per item; a noun index, 0 for none.
        // Item 1 is named by noun 2 (`*LMP`, a synonym), item 2 by noun 3.
        f.put(A_LINKS, &[0, 2, 3]);
        // Six exits per room, N S E W U D. Room 1 goes north to nothing and
        // south to room 0; room 0 is the conventional unused placeholder.
        f.put(A_EXITS, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

        // Rooms. Room 1's description is two chunks, so the implied single
        // space between them is exercised; room 0 decodes to nothing and
        // must become the placeholder.
        let rooms = pointer_table(
            A_ROOMS,
            &[
                Vec::new(),
                [chunk("dusty"), chunk("study.")].concat(),
            ],
        );
        f.put(A_ROOMS, &rooms);

        // Items. Item 0 carries the copyright substitution (0x40 renders as
        // a copyright sign and a space) and item 1 begins with `*`, marking
        // it a treasure whose asterisk IS printed.
        let mut brass = chunk("Brass");
        brass.extend_from_slice(&[1, 0x40]);
        let items = pointer_table(
            A_ITEMS,
            &[
                brass,
                chunk("*Gold*"),
                chunk("Lamp"),
            ],
        );
        f.put(A_ITEMS, &items);

        // Messages. The derived count is the table's own length in words:
        // three entries plus a sentinel is four words, so the highest real
        // message index is 4 - 2 = 2.
        let messages = pointer_table(
            A_MESSAGES,
            &[chunk("Nothing happens."), chunk("Taken."), chunk("Dropped.")],
        );
        f.put(A_MESSAGES, &messages);

        // Verbs: bare characters, no terminator and no padding.
        let verbs = pointer_table(
            A_VERBS,
            &[b"AUT".to_vec(), b"GET".to_vec(), b"RUB".to_vec()],
        );
        f.put(A_VERBS, &verbs);
        // Nouns: one more than the verbs, so the loader must pad the verb
        // list up to the noun list's length (spec §3.6).
        let nouns = pointer_table(
            A_NOUNS,
            &[
                b"ANY".to_vec(),
                b"LAM".to_vec(),
                b"*LMP".to_vec(),
                b"GOL".to_vec(),
            ],
        );
        f.put(A_NOUNS, &nouns);

        // The explicit dispatch table: one entry per verb index, no
        // sentinel. Verb 0 has no records; verb 1 chains; verb 2 ("RUB")
        // chains to a single link-0 record (see `A_RUB_CHAIN` below).
        let mut dispatch = Vec::new();
        for a in [0u16, A_CHAIN, A_RUB_CHAIN] {
            dispatch.push((a >> 8) as u8);
            dispatch.push(a as u8);
        }
        f.put(A_EXPLICIT, &dispatch);

        // Verb 1's chain. The first record matches noun 1 only and reads:
        //   B8 02   condition 184: item 2 must be in the current room
        //   ED 02   command 237: take item 2, ignoring the carry limit
        //   01      message 1
        //   FF      end of record; the record succeeds
        // The second record is the chain terminator: link 0, keyed to
        // noun 3 — still a real record (spec §3.7), and its own opcode
        // stream is NOT empty. It must be recovered by walking arities from
        // byte 2 to the closing 255, exactly like any other record's:
        //   F5 FF   command 245: set the current counter to *p*, p = 255 —
        //           deliberately the same byte value as the end-of-record
        //           opcode, so a reader that stopped at the byte right after
        //           245 (as if it took no operand) would misread this
        //           operand as the terminator and truncate the record to
        //           `F5 FF`, which can never reach a real 255 and always
        //           fails.
        //   01      message 1
        //   FF      end of record; the record succeeds
        f.put(
            A_CHAIN,
            &[
                1, 7, 0xB8, 0x02, 0xED, 0x02, 0x01, 0xFF, // record 1
                3, 0, 0xF5, 0xFF, 0x01, 0xFF, // record 2: link 0
            ],
        );

        // Verb 2's whole chain is a single link-0 record (spec §3.7, §3.8's
        // worked example — this is Adventureland's `INVENTORY` shape).
        // Matches any noun (key 0), so nothing in the verb's dispatch is
        // built-in (only verb indices 1, 10 and 18 are, spec §9.1), and its
        // opcode stream — recovered the same arity-walked way as verb 1's
        // second record above — is F5 FF 01 FF: set the counter, print
        // message 1, end successfully.
        f.put(A_RUB_CHAIN, &[0, 0, 0xF5, 0xFF, 0x01, 0xFF]);

        // One automatic record at 100%, printing message 2, then the
        // terminator. Link 3 counts the link byte itself plus the two
        // opcode bytes (spec §3.7). The second record is the chain
        // terminator: 0% (never fires) and link 0, with a one-byte opcode
        // stream that is just the end-of-record opcode.
        f.put(A_IMPLICIT, &[100, 3, 0x02, 0xFF, 0, 0, 0xFF]);

        // The object table is declared but never read (spec §3.3, §11); it
        // only has to resolve to an in-file offset.
        f.put(A_OBJECTS, &[0]);

        f.bytes
    }

    /// The `Database` the fixture describes, written out by hand from spec
    /// §3 rather than by running the loader over anything.
    fn expected() -> Database {
        Database {
            max_carry: 4,
            start_room: 1,
            // Counted from the `*` texts, NOT the header's lying 9.
            num_treasures: 1,
            word_length: 3,
            light_time: 100,
            treasure_room: 1,
            actions: Vec::new(),
            // Padded up to the noun list's four entries.
            verbs: vec![
                "AUT".into(),
                "GET".into(),
                "RUB".into(),
                DICTIONARY_FILLER.into(),
            ],
            nouns: vec!["ANY".into(), "LAM".into(), "*LMP".into(), "GOL".into()],
            rooms: vec![
                Room {
                    exits: [0; 6],
                    desc: EMPTY_STRING_PLACEHOLDER.into(),
                    literal: false,
                },
                Room {
                    exits: [0; 6],
                    desc: "dusty study.".into(),
                    literal: false,
                },
            ],
            messages: vec![
                "Nothing happens.".into(),
                "Taken.".into(),
                "Dropped.".into(),
            ],
            items: vec![
                Item {
                    text: "Brass \u{00A9} ".into(),
                    treasure: false,
                    auto_noun: None,
                    start_loc: 0,
                },
                Item {
                    text: "*Gold*".into(),
                    treasure: true,
                    // Noun 2 is `*LMP`; the synonym marker is stripped.
                    auto_noun: Some("LMP".into()),
                    start_loc: 1,
                },
                Item {
                    text: "Lamp".into(),
                    treasure: false,
                    auto_noun: Some("GOL".into()),
                    // 255 is "carried" (see the note in `parse_ti994a`).
                    start_loc: CARRIED,
                },
            ],
            adventure_number: 0,
            mysterious: false,
            ti99: Some(Ti99Script {
                verb_chains: vec![
                    Vec::new(),
                    vec![
                        Ti99Record {
                            key: 1,
                            ops: vec![0xB8, 0x02, 0xED, 0x02, 0x01, 0xFF],
                        },
                        Ti99Record {
                            key: 3,
                            // Link 0, so recovered by walking arities: 245
                            // (1 operand) then message 1 then 255.
                            ops: vec![0xF5, 0xFF, 0x01, 0xFF],
                        },
                    ],
                    vec![Ti99Record {
                        key: 0,
                        // A single link-0 record; ditto the walk above.
                        ops: vec![0xF5, 0xFF, 0x01, 0xFF],
                    }],
                    // Padded to the vocabulary length.
                    Vec::new(),
                ],
                automatic: vec![
                    Ti99Record {
                        key: 100,
                        ops: vec![0x02, 0xFF],
                    },
                    Ti99Record {
                        key: 0,
                        // Link 0, so recovered by walking arities: just the
                        // end-of-record opcode.
                        ops: vec![0xFF],
                    },
                ],
            }),
        }
    }

    #[test]
    fn hand_built_image_decodes_to_the_hand_written_database() {
        let db = parse_ti994a(&build()).expect("fixture must load");
        assert_eq!(db, expected());
    }

    /// Spec §3.7 / §3.8: a link-0 record's opcode stream is not empty, and
    /// this is not just a claim about the decoded bytes — the VM must
    /// actually run it. Verb 2 ("RUB")'s whole chain is one link-0 record —
    /// the shape §3.8 works through for Adventureland's `INVENTORY` — that
    /// sets the counter, prints message 1 and ends successfully; typing
    /// `RUB` (verb index 2 has no built-in handling of any kind, spec §9.1)
    /// must reach it and produce that message rather than "I can't do that
    /// yet.".
    #[test]
    fn a_link_zero_records_real_opcode_stream_runs_through_the_vm() {
        let db = parse_ti994a(&build()).expect("fixture must load");
        let mut vm = Vm::new_full(db, false, 1, Options::new());
        assert_eq!(vm.step(), StepResult::NeedLine);
        let _ = vm.take_output();
        vm.supply_line("rub");
        vm.step();
        let out = vm.take_output();
        assert!(
            out.contains("Taken."),
            "the link-0 record's message opcode must run: {out:?}"
        );
        assert!(
            !out.contains("can't do that yet"),
            "the record must reach its own 255 and succeed: {out:?}"
        );
    }

    #[test]
    fn database_parse_routes_a_ti99_image_to_this_loader() {
        // The text parse fails and `detect_dialect` answers TI-99/4A, so the
        // byte path must produce a real database rather than a refusal.
        let db = Database::parse(&build()).expect("Database::parse must route this");
        assert_eq!(db, expected());
    }

    #[test]
    fn chunk_separator_is_a_single_implied_space() {
        let db = parse_ti994a(&build()).unwrap();
        assert_eq!(db.rooms[1].desc, "dusty study.");
    }

    #[test]
    fn header_treasure_count_is_ignored_in_favour_of_counting_asterisks() {
        let db = parse_ti994a(&build()).unwrap();
        // The fixture's header says 9; only one item's text begins with `*`.
        assert_eq!(db.num_treasures, 1);
    }

    #[test]
    fn light_duration_of_all_ones_is_infinite() {
        let mut bytes = build();
        let h = HEADER_FROM_BASELINE;
        bytes[h + 8] = 0xFF;
        bytes[h + 9] = 0xFF;
        assert_eq!(parse_ti994a(&bytes).unwrap().light_time, -1);
    }

    #[test]
    fn literal_room_marker_is_stripped_and_recorded() {
        // Rewrite room 1's first chunk with a leading asterisk.
        let mut f = Fixture { bytes: build() };
        let rooms = pointer_table(
            A_ROOMS,
            &[Vec::new(), [chunk("*Outside"), chunk("a gate.")].concat()],
        );
        f.put(A_ROOMS, &rooms);
        let db = parse_ti994a(&f.bytes).unwrap();
        assert!(db.rooms[1].literal);
        assert_eq!(db.rooms[1].desc, "Outside a gate.");
    }

    #[test]
    fn a_word_of_twenty_characters_or_more_is_rejected() {
        let mut f = Fixture { bytes: build() };
        let verbs = pointer_table(
            A_VERBS,
            &[
                b"AUT".to_vec(),
                b"THIS-WORD-IS-FAR-TOO-LONG".to_vec(),
                b"RUB".to_vec(),
            ],
        );
        f.put(A_VERBS, &verbs);
        let db = parse_ti994a(&f.bytes).unwrap();
        assert_eq!(db.verbs[1], "");
    }

    #[test]
    fn a_malformed_chunk_length_becomes_the_placeholder_not_an_error() {
        let mut f = Fixture { bytes: build() };
        // A chunk claiming 200 characters is malformed (spec §3.5).
        let messages = pointer_table(
            A_MESSAGES,
            &[chunk("fine"), vec![200, b'x'], chunk("also fine")],
        );
        f.put(A_MESSAGES, &messages);
        let db = parse_ti994a(&f.bytes).unwrap();
        assert_eq!(db.messages[1], EMPTY_STRING_PLACEHOLDER);
        assert_eq!(db.messages[0], "fine");
    }

    #[test]
    fn every_display_substitution_is_applied() {
        let mut f = Fixture { bytes: build() };
        let messages = pointer_table(
            A_MESSAGES,
            &[
                vec![4, 0x40, 0x7B, 0x7D, 0x0C],
                chunk("b"),
                chunk("c"),
            ],
        );
        f.put(A_MESSAGES, &messages);
        let db = parse_ti994a(&f.bytes).unwrap();
        assert_eq!(db.messages[0], "\u{00A9} \u{00E4}\u{00FC}\u{00F6}");
    }

    #[test]
    fn no_signature_is_refused_by_name() {
        let err = parse_ti994a(b"not a TI-99/4A image at all").unwrap_err();
        assert!(matches!(
            err,
            LoadError::BadDialectData(Dialect::Ti994aBytecode, _)
        ));
    }

    #[test]
    fn a_pointer_resolving_outside_the_file_is_refused_by_name() {
        let mut bytes = build();
        // Point the room exit table at the very top of the address space.
        let h = HEADER_FROM_BASELINE;
        bytes[h + 12 + P_ROOM_EXITS * 2] = 0xFF;
        bytes[h + 12 + P_ROOM_EXITS * 2 + 1] = 0xF0;
        assert_eq!(
            parse_ti994a(&bytes).unwrap_err(),
            LoadError::BadDialectData(
                Dialect::Ti994aBytecode,
                "a header table pointer resolves outside the file"
            )
        );
    }

    #[test]
    fn a_negative_baseline_is_refused_by_name() {
        // The signature present, but too near the start of the file for the
        // baseline subtraction to stay non-negative (spec §3.1).
        let mut bytes = vec![0u8; 0x100];
        bytes[0x10..0x10 + SIGNATURE.len()].copy_from_slice(&SIGNATURE);
        assert!(matches!(
            parse_ti994a(&bytes).unwrap_err(),
            LoadError::BadDialectData(Dialect::Ti994aBytecode, _)
        ));
    }

    #[test]
    fn a_truncated_action_chain_is_refused_rather_than_panicking() {
        let mut bytes = build();
        // Claim a record far longer than the file can supply.
        let off = usize::from(A_CHAIN) - IMAGE_BASE as usize;
        bytes[off + 1] = 0xFF;
        bytes.truncate(off + 4);
        assert!(matches!(
            parse_ti994a(&bytes).unwrap_err(),
            LoadError::BadDialectData(Dialect::Ti994aBytecode, _)
        ));
    }

    #[test]
    fn byte_flipping_the_fixture_never_panics() {
        // 200 deterministic single-byte mutations over the whole image: the
        // loader must answer `Ok` or a named `LoadError` for every one, and
        // must never panic, hang, or exhaust memory (SQ-1414, brief item 3c).
        let base = build();
        let mut state: u32 = 0x1BAD_C0DE;
        for _ in 0..200 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let pos = (state as usize) % base.len();
            let bit = 1u8 << ((state >> 16) % 8);
            let mut bytes = base.clone();
            bytes[pos] ^= bit;
            // The result is uninteresting; not panicking is the assertion.
            let _ = parse_ti994a(&bytes);
        }
    }

    #[test]
    fn a_zero_dispatch_entry_leaves_the_verb_with_no_records() {
        let db = parse_ti994a(&build()).unwrap();
        let script = db.ti99.as_ref().unwrap();
        assert!(script.verb_chains[0].is_empty());
        assert_eq!(script.verb_chains[1].len(), 2);
        // Verb 2 ("RUB") has a dispatch entry, and its one record is what
        // `a_link_zero_records_real_opcode_stream_runs_through_the_vm` below
        // drives through the VM.
        assert_eq!(script.verb_chains[2].len(), 1);
    }

    #[test]
    fn a_leading_zero_in_the_implicit_block_means_no_automatic_actions() {
        let mut f = Fixture { bytes: build() };
        f.put(A_IMPLICIT, &[0, 3, 0x02, 0xFF, 0, 0]);
        let db = parse_ti994a(&f.bytes).unwrap();
        assert!(db.ti99.as_ref().unwrap().automatic.is_empty());
    }
}
