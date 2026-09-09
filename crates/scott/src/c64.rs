//! Reads the **Commodore 64 *Mysterious Adventures*** — Brian Howarth's
//! eleven-title series as sold on the two `MYSTADV` compilation disks — into
//! the same [`Database`] every other Scott Adams dialect decodes to, and
//! decodes their line-drawn artwork ([`decode_family_b_pictures`]).
//!
//! The input is a raw 6502 **memory image**: the body of a Commodore program
//! file with its two load-address bytes removed, plus the address those bytes
//! held ([`prg_image`] does that split). Nothing in these eleven is compressed
//! and no 6502 is executed to reach the tables — every one is an uncrunched
//! image whose driver plants the table addresses into zero page as literal
//! operands, which a reader simply reads back out.
//!
//! # Provenance
//!
//! Everything here is implemented from
//! [`docs/internals/scott-dialects-spec.md`](../../../docs/internals/scott-dialects-spec.md)
//! (§4 locating and encodings, §5.3 load-time repairs, §6 the Mysterious
//! releases, §8.2 the Family B picture format, §9.2 lamp behaviour, §11
//! refusals) and
//! [`docs/internals/scott-c64-layout-findings.md`](../../../docs/internals/scott-c64-layout-findings.md),
//! SQ-1455's measurement of these exact specimens. Both were written under the
//! clean-room protocol in `docs/internals/clean-room.md`. **No GPL
//! interpreter's source was read to write this module** — lanthorn is
//! BSD-3-Clause and every established Scott Adams interpreter is GPL, so those
//! two documents plus the specimens are the only channels by which a fact
//! about this format reached this file.
//!
//! The specification has since absorbed most of what the findings measured —
//! §4.5 and §6.2 on the `JMP`-marked header and the pointer block, §5.3 on the
//! two repairs, §6.4 on the first-person wording — so the doc comments below
//! cite the SPECIFICATION wherever both now say the same thing, and reach for
//! the findings only where they still differ. **Where the two disagree the
//! measurement wins for these eleven specimens**, and each such site says so.
//! Two still differ: §4.2's escape-driven dictionary reader (these
//! dictionaries are plain fixed-width cells — see `read_cells`) and §5.3's
//! Mysterious Commodore 64 dictionary repairs (the eleven already carry `ANY`
//! and the six direction words in noun cells 0-6 — see
//! [`system_direction_words`]).
//!
//! # The layout, in one picture
//!
//! ```text
//!   $4000  driver (shared by all eleven)    $48E6  zero-page pointer block
//!   $4406  system messages (44 strings)     $5DD6  JMP $4D19  ← header guard
//!   $5DD7  header  →  actions  →  dictionary
//!          →  room descriptions  →  room connections  →  messages
//!          →  item descriptions  →  item locations
//!          →  zero gap  →  Family B picture data  →  end of file
//! ```
//!
//! Only two facts per release are not in the bytes: which of the four header
//! field orders it uses, and how its dictionary splits into verb and noun
//! cells. [`RELEASES`] is that table, keyed by the image's 16-bit checksum.

use crate::database::CARRIED;
use crate::loader::{extract_auto_noun, Dialect, LoadError};
use crate::{Action, Condition, Database, Item, Room};

// ── Fixed addresses ───────────────────────────────────────────────────────────

/// `JMP $4D19`, at [`HEADER_GUARD_ADDR`] in all eleven releases: the
/// instruction immediately in front of the header, whose two-byte operand is
/// what §4.5 calls the early header's "unused word 0" (§4.5, "Word 0 of the
/// early shape is not a field"). §6.2 calls the three bytes "a serviceable
/// recognition signature for the whole family in its own right", and reading
/// them as a guard is what makes [`HEADER_ADDR`] safe to trust.
const HEADER_GUARD: [u8; 3] = [0x4C, 0x19, 0x4D];

/// Where [`HEADER_GUARD`] sits.
const HEADER_GUARD_ADDR: u16 = 0x5DD6;

/// The header's address — the `JMP`'s opcode plus one, so header word 0 is the
/// `JMP` operand and word 1 the first real count, which is the numbering
/// §4.5's field orders use.
const HEADER_ADDR: u16 = HEADER_GUARD_ADDR + 1;

/// How many little-endian words §4.3 says a memory-image header holds.
const HEADER_WORDS: usize = 15;

/// The driver's initialisation code, where twelve `LDA #imm` / `STA <zp>`
/// pairs plant six 16-bit table addresses into zero page as literal operands
/// (§6.2, "The table addresses are in the file, at a fixed offset"). This is
/// the de facto pointer table §4.6 tells an implementer to look for **before**
/// tabulating anything: one constant per series instead of a dozen per
/// release.
const POINTER_BLOCK: u16 = 0x48E6;

/// How many `LDA #`/`STA` pairs the block holds: six addresses, low byte then
/// high byte.
const POINTER_PAIRS: usize = 12;

/// The **seventh** address §6.2 records, planted a few bytes later by two
/// `LDA #imm` / `STA <absolute>` pairs rather than to zero page: its low byte
/// is the immediate at `$4917` and its high byte the one at `$491C`, and it is
/// the **dictionary**. §6.2 calls it "a free and complete cross-check on §4.1",
/// and [`parse_c64_mysterious`] uses it as exactly that — in all eleven
/// specimens the plain `AUTO\0GO\0` signature occurs once and at this address.
const DICTIONARY_POINTER: (u16, u16) = (0x4917, 0x491C);

/// 6502 `LDA #immediate`, the first byte of every pair in the pointer block.
const OP_LDA_IMM: u8 = 0xA9;

/// 6502 `STA zeropage`, the third byte of every pair.
const OP_STA_ZP: u8 = 0x85;

/// The system-message block: a 790-byte run of 44 CR-or-NUL-terminated
/// strings, **byte-identical across all eleven releases**, which §6.4
/// tabulates string by string. §4.4's rule that Commodore 64 English releases
/// have no separate direction-word table and take those six from the head of
/// this block is what [`direction_words`] reads.
const SYSTEM_MESSAGES_ADDR: u16 = 0x4406;

/// How far back §4.3's system-message search may walk looking for `NORTH`
/// before giving up. It never walks at all on these eleven; the bound exists
/// so a damaged image cannot make the search run away.
const SYSTEM_MESSAGE_BACKOFF: u16 = 16;

/// §4.1's plain four-letter dictionary signature, with back-off 0: the match
/// lands **on** the dictionary's first byte.
const DICTIONARY_SIGNATURE: &[u8] = b"AUTO\0GO\0";

/// Bytes per action record: eight little-endian 16-bit words (§4.4).
const ACTION_RECORD: u16 = 16;

/// Exits per room connection record: north, south, east, west, up, down (§4.4).
const EXITS_PER_ROOM: u16 = 6;

/// What an item description reads as when the block holds no string for it.
///
/// §5.3's *The Time Machine* repair: its item block holds 62 NUL-terminated
/// strings where the header's item count of 62 implies 63, and its location
/// table is a full 63 bytes, so item 62 has a location and no stored
/// description. A loader "**must supply an empty description** rather than read
/// a 63rd string", which would run into the location table. Every other title
/// in the series is the control: on the §10.4 specimens the item block holds
/// exactly (item count + 1) strings, and only this one does not.
const MISSING_ITEM_TEXT: &str = "";

/// The reference format's own "carried" byte, normalised to
/// [`crate::database::CARRIED`] at load (§4.4).
const STORED_CARRIED: u8 = 255;

// ── Zero-page slots the pointer block writes ──────────────────────────────────

/// Room-description table address, low byte then high.
const ZP_ROOMS: (u8, u8) = (0x32, 0x33);
/// Room-connection table address.
const ZP_CONNECTIONS: (u8, u8) = (0x2C, 0x2D);
/// Message-pool address.
const ZP_MESSAGES: (u8, u8) = (0x34, 0x35);
/// Item-description table address.
const ZP_ITEMS: (u8, u8) = (0x36, 0x37);
/// Item-location table address.
const ZP_LOCATIONS: (u8, u8) = (0x2E, 0x2F);
/// The first byte after the item-location table — where the run of zero bytes
/// before the picture data begins.
const ZP_LOCATIONS_END: (u8, u8) = (0x30, 0x31);

// ── §4.3's validation ranges ──────────────────────────────────────────────────

/// §4.3 step 5's plausibility ranges for the four counts, as
/// `(low, high)` inclusive pairs: items, actions, words, rooms. A header that
/// falls outside them is not this dialect's, whatever the checksum said.
const COUNT_RANGES: [(&str, u16, u16); 4] =
    [("items", 10, 500), ("actions", 100, 500), ("words", 50, 190), ("rooms", 10, 100)];

// ── The per-release table ─────────────────────────────────────────────────────

/// Which of §4.5's field orders a release's header uses.
///
/// The four the series spends between them, and the only ones this module
/// implements — §4.5's other seven belong to Adventure International releases
/// this module does not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderShape {
    /// §4.5's **early** shape, which is the reference format's own field
    /// order: items 1, actions 2, words 3, rooms 4, max carried 5, start room
    /// 6, treasure count 7, word length 8, lamp turns 9, messages 10,
    /// treasure room 11.
    Early,
    /// §4.5's **Mysterious Commodore 64** shape: as [`HeaderShape::Early`]
    /// through word 4, then max carried is the LOW byte of word 5 and start
    /// room the HIGH byte, treasure count 6, word length 7, lamp turns 8,
    /// messages 9, and the treasure room is forced to 0 (§6.2).
    Mysterious,
    /// §4.5's **Arrow of Death part 2** shape: [`HeaderShape::Mysterious`],
    /// except that items come from word 3, actions from word 1 and the word
    /// count from word 2.
    Arrow2,
    /// §4.5's **Ten Little Indians** shape, **read at byte granularity**.
    ///
    /// §4.5 renders this as a word grid read half at a time and says "lamp
    /// turns = high byte of word 7"; on that grid word 7 is `$F400`, whose
    /// high byte is 244. §4.5 now spells the byte-level reading instead —
    /// from the header address plus two: four little-endian words (items,
    /// actions, words, rooms), then four single bytes (max carried, start
    /// room, treasure count, word length), then one filler byte, then two
    /// ordinary little-endian words — which gives lamp turns `F4 01` = **500**,
    /// exactly what that title's published conversion says. This module
    /// implements the byte-level reading.
    TenLittleIndians,
}

impl HeaderShape {
    /// How many bytes the header occupies from `$5DD7`, the `JMP` operand
    /// included.
    ///
    /// Measured from `$5DD7`, the `JMP $4D19` operand (header word 0)
    /// inclusive — §6.2's own table. The action table begins immediately after
    /// it, which is a free cross-check: the table's start is independently
    /// derivable as (dictionary − (action count + 1) × 16), and
    /// [`parse_c64_mysterious`] requires the two to agree.
    pub const fn header_len(self) -> u16 {
        match self {
            HeaderShape::Early => 26,
            HeaderShape::Mysterious | HeaderShape::Arrow2 => 24,
            HeaderShape::TenLittleIndians => 23,
        }
    }
}

/// One catalogued release: the two facts a loader cannot read out of the
/// bytes, plus the identity and the one load-time repair the series needs.
///
/// **How each column was derived**, so it can be re-derived from a specimen
/// without this table — the honest column §4.6 asks any such table to carry.
/// §4.6's own answer is that a release with a pointer block needs "two
/// numbers, and no more", and these are they:
///
/// * `checksum` — [`image_checksum`], measured over the thirteen program files
///   extracted from `MYSTADV1.D64` and `MYSTADV2.D64` by the public 1541
///   directory and block-chain walk.
/// * `shape` — found by reading the fifteen header words at `$5DD7`
///   under each of §4.5's four candidate orders and keeping the one whose
///   counts agree with the title's published reference-format conversion and
///   whose implied header length puts the action table exactly where
///   (dictionary − (actions + 1) × 16) puts it.
/// * `verb_cells` — found by reading the dictionary's cells (the whole span
///   from the signature hit to the room-description pointer) and searching for
///   the split at which the leading cells match the conversion's verb column
///   and the trailing cells its noun column. In all eleven the larger of the
///   two blocks is exactly (word count + 1) and the other is the remainder,
///   which is why one number suffices.
/// * `action_count` — `None` except for the one release whose stored count is
///   wrong; see the field's own doc.
///
/// None of it came from any interpreter's catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Release {
    /// [`image_checksum`] of the program file this record describes.
    pub checksum: u16,
    /// The name the compilation disk stores the program file under, which is
    /// also how §6.5 names it.
    pub file_name: &'static str,
    /// The game, spelled as the series spells it.
    pub title: &'static str,
    /// Which of §4.5's field orders the header uses.
    pub shape: HeaderShape,
    /// How many of the dictionary's cells are verbs; the rest are nouns.
    ///
    /// §4.6 lists this as the one genuinely undetermined quantity of the
    /// family, and getting it wrong costs vocabulary rather than layout,
    /// because the room descriptions have a pointer of their own.
    pub verb_cells: usize,
    /// A §5.3-style load-time repair: the action count to use in place of the
    /// one the header stores.
    ///
    /// `Some(190)` for *Escape from Pulsar 7* alone. Its header says 195, but
    /// only 191 records fit between the end of the header and the dictionary
    /// and 191 records validate perfectly, while 196 overrun the dictionary by
    /// 80 bytes — precisely the shape of §5.3's "the header's action count
    /// (243) is wrong and must be replaced by 236" for the German Commodore 64
    /// *Gremlins*. [`parse_c64_mysterious`] checks that the table this implies
    /// ends exactly on the dictionary's first byte, so a wrong number here
    /// cannot pass silently.
    pub action_count: Option<u16>,
}

/// The eleven Commodore 64 *Mysterious Adventures* releases, as they sit on
/// `MYSTADV1.D64` and `MYSTADV2.D64` (§6.5, §10.4).
///
/// See [`Release`] for how every column was derived from the specimens.
pub const RELEASES: [Release; 11] = [
    Release {
        checksum: 0x01FF,
        file_name: "BATON",
        title: "The Golden Baton",
        shape: HeaderShape::Mysterious,
        verb_cells: 79,
        action_count: None,
    },
    Release {
        checksum: 0xBBDD,
        file_name: "TIME MACHINE",
        title: "The Time Machine",
        shape: HeaderShape::Mysterious,
        verb_cells: 86,
        action_count: None,
    },
    Release {
        checksum: 0xAFE0,
        file_name: "ARROW I",
        title: "Arrow of Death part 1",
        shape: HeaderShape::Mysterious,
        verb_cells: 91,
        action_count: None,
    },
    Release {
        checksum: 0x1A8C,
        file_name: "ARROW II",
        title: "Arrow of Death part 2",
        shape: HeaderShape::Arrow2,
        verb_cells: 80,
        action_count: None,
    },
    Release {
        checksum: 0x1058,
        file_name: "PULSAR 7",
        title: "Escape from Pulsar 7",
        shape: HeaderShape::Early,
        verb_cells: 146,
        action_count: Some(190),
    },
    Release {
        checksum: 0x1194,
        file_name: "CIRCUS",
        title: "Circus",
        shape: HeaderShape::Mysterious,
        verb_cells: 98,
        action_count: None,
    },
    Release {
        checksum: 0xDB00,
        file_name: "EXPERIMENT",
        title: "Feasibility Experiment",
        shape: HeaderShape::Early,
        verb_cells: 56,
        action_count: None,
    },
    Release {
        checksum: 0xA7FE,
        file_name: "WIZARD OF AKYRZ",
        title: "The Wizard of Akyrz",
        shape: HeaderShape::Mysterious,
        verb_cells: 67,
        action_count: None,
    },
    Release {
        checksum: 0xE116,
        file_name: "PERSEUS",
        title: "Perseus and Andromeda",
        shape: HeaderShape::Early,
        verb_cells: 131,
        action_count: None,
    },
    Release {
        checksum: 0x0AD7,
        file_name: "INDIANS",
        title: "Ten Little Indians",
        shape: HeaderShape::TenLittleIndians,
        verb_cells: 64,
        action_count: None,
    },
    Release {
        checksum: 0x25CC,
        file_name: "WAXWORKS",
        title: "Waxworks",
        shape: HeaderShape::Early,
        verb_cells: 91,
        action_count: None,
    },
];

// ── Identification ────────────────────────────────────────────────────────────

/// §7.2's identity for a Commodore 64 file: **the low sixteen bits of the sum
/// of every byte in the program file, with wraparound — not a CRC.**
///
/// Takes the memory image and the load address rather than the file, because
/// that is what this module's entry points take; the two load-address bytes
/// are added back in, so the answer is the program file's own checksum and
/// matches the per-title table §10.4 pins.
pub fn image_checksum(image: &[u8], load_address: u16) -> u16 {
    let mut sum = 0u16;
    for &b in load_address.to_le_bytes().iter().chain(image) {
        sum = sum.wrapping_add(u16::from(b));
    }
    sum
}

/// The catalogued release whose checksum this image matches, if any.
pub fn identify(image: &[u8], load_address: u16) -> Option<&'static Release> {
    let sum = image_checksum(image, load_address);
    RELEASES.iter().find(|r| r.checksum == sum)
}

/// Is this memory image one of the eleven?
///
/// Three independent checks: a catalogued checksum, the `JMP $4D19` guard in
/// front of the header, and §4.1's dictionary signature somewhere in the
/// image. A file that answers `true` is one [`parse_c64_mysterious`] reads; a
/// Commodore 64 image that answers `false` is still refused **by name** —
/// [`crate::Database::parse`] reports [`Dialect::C64OrZxSnapshot`] for it
/// rather than a token-level parse error.
pub fn looks_like_c64_mysterious(image: &[u8], load_address: u16) -> bool {
    identify(image, load_address).is_some()
        && read_at(image, load_address, HEADER_GUARD_ADDR, 3) == Some(&HEADER_GUARD[..])
        && find_signature(image, load_address).is_some()
}

/// Split a Commodore program file into its memory image and its load address:
/// bytes 0-1 are the address, little-endian, and byte 2 is the byte at that
/// address (§7.2, "From container to program file"). **There is no fixed
/// Commodore 64 base address — it is read from the file, every time**; that it
/// is `$4000` in all eleven of these is a fact about them, not about the
/// format.
///
/// `None` for a file too short to hold either.
pub fn prg_image(file: &[u8]) -> Option<(&[u8], u16)> {
    if file.len() < 3 {
        return None;
    }
    Some((&file[2..], u16::from_le_bytes([file[0], file[1]])))
}

/// Does this **program file** (load address included) hold one of the eleven?
/// [`looks_like_c64_mysterious`] over [`prg_image`].
pub fn looks_like_c64_mysterious_prg(file: &[u8]) -> bool {
    prg_image(file).is_some_and(|(image, at)| looks_like_c64_mysterious(image, at))
}

/// Read one of the eleven **program files**, load address included:
/// [`parse_c64_mysterious`] over [`prg_image`].
///
/// This is the entry point a host reaches for after pulling a named file off a
/// `.d64` — the program file is exactly what the container yields, and
/// stripping its first two bytes is this crate's business rather than the
/// container reader's.
pub fn parse_c64_mysterious_prg(file: &[u8]) -> Result<Database, LoadError> {
    let (image, at) = prg_image(file).ok_or_else(|| bad("shorter than a load address"))?;
    parse_c64_mysterious(image, at)
}

// ── Address arithmetic ────────────────────────────────────────────────────────

/// A refusal for a recognised release whose bytes did not check out — §11 asks
/// an implementer to name these rather than guess past them.
fn bad(what: &'static str) -> LoadError {
    LoadError::BadDialectData(Dialect::C64OrZxSnapshot, what)
}

/// File offset of memory address `addr`, or `None` when it is below the load
/// address or past the end of the image.
fn offset_of(image: &[u8], load_address: u16, addr: u16) -> Option<usize> {
    let off = usize::from(addr.checked_sub(load_address)?);
    (off <= image.len()).then_some(off)
}

/// `len` bytes at memory address `addr`, or `None` when they do not all lie
/// within the image.
fn read_at(image: &[u8], load_address: u16, addr: u16, len: usize) -> Option<&[u8]> {
    let off = offset_of(image, load_address, addr)?;
    image.get(off..off.checked_add(len)?)
}

/// One byte at memory address `addr`.
fn byte_at(image: &[u8], load_address: u16, addr: u16) -> Option<u8> {
    read_at(image, load_address, addr, 1).map(|s| s[0])
}

/// The address of §4.1's plain signature, with back-off 0. The first hit wins
/// and the search is unanchored (§4.1).
fn find_signature(image: &[u8], load_address: u16) -> Option<u16> {
    let at = image.windows(DICTIONARY_SIGNATURE.len()).position(|w| w == DICTIONARY_SIGNATURE)?;
    u16::try_from(usize::from(load_address) + at).ok()
}

// ── The header ────────────────────────────────────────────────────────────────

/// The eleven counts §4.5's field orders assign, whichever order assigned them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Header {
    items: u16,
    actions: u16,
    words: u16,
    rooms: u16,
    max_carry: u16,
    start_room: u16,
    treasures: u16,
    word_length: u16,
    lamp: u16,
    messages: u16,
    treasure_room: u16,
}

/// Read the header at [`HEADER_ADDR`] under `shape`'s field order.
fn read_header(image: &[u8], load_address: u16, shape: HeaderShape) -> Result<Header, LoadError> {
    let raw = read_at(image, load_address, HEADER_ADDR, HEADER_WORDS * 2)
        .ok_or_else(|| bad("the header at $5DD7 does not lie within the image"))?;
    let w = |i: usize| u16::from_le_bytes([raw[i * 2], raw[i * 2 + 1]]);
    let b = |i: usize| u16::from(raw[i]);
    Ok(match shape {
        HeaderShape::Early => Header {
            items: w(1),
            actions: w(2),
            words: w(3),
            rooms: w(4),
            max_carry: w(5),
            start_room: w(6),
            treasures: w(7),
            word_length: w(8),
            lamp: w(9),
            messages: w(10),
            treasure_room: w(11),
        },
        HeaderShape::Mysterious | HeaderShape::Arrow2 => Header {
            items: if shape == HeaderShape::Arrow2 { w(3) } else { w(1) },
            actions: if shape == HeaderShape::Arrow2 { w(1) } else { w(2) },
            words: if shape == HeaderShape::Arrow2 { w(2) } else { w(3) },
            rooms: w(4),
            max_carry: w(5) & 0xFF,
            start_room: w(5) >> 8,
            treasures: w(6),
            word_length: w(7),
            lamp: w(8),
            messages: w(9),
            // §6.2: this shape drops the field, and the treasure room is 0.
            treasure_room: 0,
        },
        // Byte-packed throughout, per §4.5 — NOT a word grid. Bytes 0 and 1
        // are the `JMP` operand, so the counts start at byte 2:
        // four count words (2..10), four single bytes (10..14), one filler
        // byte (14), then two ordinary little-endian words (15..19).
        HeaderShape::TenLittleIndians => Header {
            items: w(1),
            actions: w(2),
            words: w(3),
            rooms: w(4),
            max_carry: b(10),
            start_room: b(11),
            treasures: b(12),
            word_length: b(13),
            lamp: u16::from_le_bytes([raw[15], raw[16]]),
            messages: u16::from_le_bytes([raw[17], raw[18]]),
            treasure_room: 0,
        },
    })
}

// ── The pointer block ─────────────────────────────────────────────────────────

/// The six table addresses the driver plants into zero page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tables {
    rooms: u16,
    connections: u16,
    messages: u16,
    items: u16,
    locations: u16,
    locations_end: u16,
}

/// Read the twelve `LDA #imm` / `STA <zp>` pairs at [`POINTER_BLOCK`] and pair
/// their operands into six addresses, low byte then high.
///
/// `None` when the block is not there — an opcode does not check out, a slot
/// this reader wants was never written, or the image is too short — which is
/// when [`chained_tables`] takes over.
fn pointer_block(image: &[u8], load_address: u16) -> Option<Tables> {
    let raw = read_at(image, load_address, POINTER_BLOCK, POINTER_PAIRS * 4)?;
    let mut zp = [None; 256];
    for pair in raw.as_chunks::<4>().0 {
        if pair[0] != OP_LDA_IMM || pair[2] != OP_STA_ZP {
            return None;
        }
        zp[usize::from(pair[3])] = Some(pair[1]);
    }
    let word = |(lo, hi): (u8, u8)| -> Option<u16> {
        Some(u16::from_le_bytes([zp[usize::from(lo)]?, zp[usize::from(hi)]?]))
    };
    Some(Tables {
        rooms: word(ZP_ROOMS)?,
        connections: word(ZP_CONNECTIONS)?,
        messages: word(ZP_MESSAGES)?,
        items: word(ZP_ITEMS)?,
        locations: word(ZP_LOCATIONS)?,
        locations_end: word(ZP_LOCATIONS_END)?,
    })
}

/// §6.2's **seventh** address: the dictionary, planted by two `LDA #imm` /
/// `STA <absolute>` pairs a few bytes past the zero-page block, with its low
/// byte at `$4917` and its high byte at `$491C`.
///
/// A cross-check rather than a source, because §4.1's signature already gives
/// the dictionary and gives it for every memory-image dialect rather than only
/// this series. `None` when the driver does not carry it, which is not a
/// refusal on its own.
fn dictionary_pointer(image: &[u8], load_address: u16) -> Option<u16> {
    let (lo, hi) = DICTIONARY_POINTER;
    Some(u16::from_le_bytes([
        byte_at(image, load_address, lo)?,
        byte_at(image, load_address, hi)?,
    ]))
}

/// §4.3's fallback for a release with no pointer block: every table starts
/// "immediately after the previous table read", in the later family's order —
/// dictionary, room descriptions, room connections, messages, item
/// descriptions, item locations.
///
/// **Unexercised by the eleven**, all of which carry the block; it exists so
/// that a release which does not is refused on its data rather than on the
/// block's absence. The chain rule cannot supply the dictionary's cell count,
/// so this uses the regularity measured across all eleven — one of the two
/// columns is always exactly (word count + 1) — which `verb_cells` then
/// completes. The result goes through the same end-to-end checks the pointer
/// path gets, so a release where that regularity does not hold is refused
/// rather than mis-read.
fn chained_tables(
    image: &[u8],
    load_address: u16,
    header: &Header,
    release: &Release,
    dictionary: u16,
) -> Result<Tables, LoadError> {
    let full = usize::from(header.words) + 1;
    let cells = release.verb_cells + full;
    let end_of = |what: &'static str, addr: u16, count: usize| -> Result<u16, LoadError> {
        read_strings(image, load_address, addr, None, count).map(|(_, end)| end).ok_or_else(|| bad(what))
    };
    let width = usize::from(header.word_length) + 1;
    let rooms = read_cells(image, load_address, dictionary, None, width, cells)
        .map(|(_, end)| end)
        .ok_or_else(|| bad("the dictionary runs past the end of the image"))?;
    let connections = end_of("the room descriptions run past the end of the image", rooms, usize::from(header.rooms) + 1)?;
    let messages = (header.rooms + 1)
        .checked_mul(EXITS_PER_ROOM)
        .and_then(|n| connections.checked_add(n))
        .ok_or_else(|| bad("the room connections run past the end of the image"))?;
    let items = end_of("the messages run past the end of the image", messages, usize::from(header.messages) + 1)?;
    let locations = end_of("the item descriptions run past the end of the image", items, usize::from(header.items) + 1)?;
    let locations_end = locations
        .checked_add(header.items + 1)
        .ok_or_else(|| bad("the item locations run past the end of the image"))?;
    Ok(Tables { rooms, connections, messages, items, locations, locations_end })
}

// ── Table encodings (§4.4) ────────────────────────────────────────────────────

/// Read `count` dictionary cells from `addr`, stopping at `limit` when one is
/// given. Returns the cells and the address one past the last byte consumed.
///
/// **A cell is (word length + 1) bytes, NUL-padded**, with a leading `*`
/// marking a synonym of the nearest preceding canonical word, exactly as in
/// the reference format. One escape: **a NUL where a cell should begin is
/// alignment padding and is skipped**, which is §4.2's own first rule and is
/// what makes *The Golden Baton*'s final cell read `CAST` rather than one byte
/// short of it. (Measured: no cell in any of the eleven begins with a NUL for
/// any other reason, so the skip fires exactly once in the whole corpus.)
///
/// §4.2's other two escapes — `*` restarting the character count and so
/// costing an extra byte, and a space followed by a non-space rewinding it —
/// are **not** implemented, because they are wrong for this family: a `*` here
/// occupies one of the cell's own bytes, and *Waxworks* carries dictionary
/// cells that are nothing but spaces, which the space escape mis-aligns.
/// Measured against the six titles whose tables are byte-identical to their
/// published conversions, this reading reproduces every verb and noun cell
/// exactly and §4.2's does not.
fn read_cells(
    image: &[u8],
    load_address: u16,
    addr: u16,
    limit: Option<u16>,
    width: usize,
    count: usize,
) -> Option<(Vec<String>, u16)> {
    let mut out = Vec::with_capacity(count);
    let mut at = addr;
    while out.len() < count {
        if limit.is_some_and(|end| at >= end) {
            break;
        }
        // Alignment padding: a NUL where a cell should begin.
        if byte_at(image, load_address, at) == Some(0) {
            at = at.checked_add(1)?;
        }
        let cell = read_at(image, load_address, at, width)?;
        at = at.checked_add(u16::try_from(width).ok()?)?;
        let (synonym, body) = match cell.split_first() {
            Some((b'*', rest)) => (true, rest),
            _ => (false, cell),
        };
        let text: String = body
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| if b < 0x80 { b as char } else { '?' })
            .collect();
        out.push(if synonym { format!("*{text}") } else { text });
    }
    Some((out, at))
}

/// Read `count` NUL-terminated byte strings from `addr`, stopping at `limit`
/// when one is given (§4.4). Returns the strings and the address one past the
/// last byte consumed.
///
/// A byte above 127 ends the string's **text** — §4.4's "a byte above 127
/// aborts the read" — while the scan continues to the NUL so that the records
/// after it stay aligned. None of the eleven exercises it: every room
/// description, message and item name in the corpus is plain ASCII.
///
/// A string the block has no room for comes back as [`MISSING_ITEM_TEXT`]
/// rather than reading into the table that follows. That is *The Time
/// Machine*'s 63rd item, whose header count implies 63 descriptions where the
/// block holds 62 (§5.3): it has a location but no stored text, and a reader
/// must supply an empty one rather than run into the location table.
fn read_strings(
    image: &[u8],
    load_address: u16,
    addr: u16,
    limit: Option<u16>,
    count: usize,
) -> Option<(Vec<String>, u16)> {
    let mut out = Vec::with_capacity(count);
    let mut at = addr;
    for _ in 0..count {
        if limit.is_some_and(|end| at >= end) {
            out.push(MISSING_ITEM_TEXT.to_string());
            continue;
        }
        let mut text = String::new();
        let mut aborted = false;
        loop {
            if limit.is_some_and(|end| at >= end) {
                break;
            }
            let b = byte_at(image, load_address, at)?;
            at = at.checked_add(1)?;
            if b == 0 {
                break;
            }
            if b > 127 {
                aborted = true;
            } else if !aborted {
                text.push(b as char);
            }
        }
        out.push(text);
    }
    Some((out, at))
}

/// The six direction words, taken from the head of the system-message block.
///
/// §4.4: Commodore 64 English releases have no separate direction-word table
/// and take north, south, east, west, up and down from there. §4.3 makes the
/// block's address a hint rather than a constant — "read the first string
/// there; if it is not `NORTH`, back the start up by one byte and retry" — and
/// this reproduces that search rather than trusting [`SYSTEM_MESSAGES_ADDR`],
/// bounded by [`SYSTEM_MESSAGE_BACKOFF`]. Measured: it is satisfied at the
/// address itself, with no back-up needed, in all eleven.
///
/// Strings here are terminated by NUL **or** by a carriage return, which §4.4
/// says is kept as the string's last byte; it is trimmed off a direction word,
/// which is a vocabulary entry rather than a printed line.
fn direction_words(image: &[u8], load_address: u16) -> Option<[String; 6]> {
    for back in 0..=SYSTEM_MESSAGE_BACKOFF {
        let at = SYSTEM_MESSAGES_ADDR.checked_sub(back)?;
        let mut words: Vec<String> = Vec::with_capacity(6);
        let mut cursor = at;
        while words.len() < 6 {
            let mut text = String::new();
            loop {
                let b = byte_at(image, load_address, cursor)?;
                cursor = cursor.checked_add(1)?;
                if b == 0 || b == 0x0D {
                    break;
                }
                text.push(if b < 0x80 { b as char } else { '?' });
            }
            // §4.4: zero-length strings are skipped and do not consume an index.
            if !text.is_empty() {
                words.push(text);
            }
        }
        if words[0] == "NORTH" {
            return words.try_into().ok();
        }
    }
    None
}

// ── The loader ────────────────────────────────────────────────────────────────

/// Read a Commodore 64 *Mysterious Adventures* memory image into a
/// [`Database`].
///
/// `image` is the program file's body — its two load-address bytes already
/// removed — and `load_address` is what they held (`$4000` in all eleven).
/// [`parse_c64_mysterious_prg`] is the same thing over an unsplit program
/// file, and is what a host holding bytes off a `.d64` wants.
///
/// # Errors
///
/// * [`LoadError::UnsupportedDialect`] with [`Dialect::C64OrZxSnapshot`] when
///   the image is not one of the eleven — §11's "a dictionary signature that
///   matches but for which no catalogued release validates is the normal
///   outcome for an unknown release of a known game, and there is no
///   fallback". The refusal is by name, so a host can say what the file is.
/// * [`LoadError::BadDialectData`] when it IS one of the eleven and its own
///   structures did not check out: a table pointer resolving outside the
///   image, a record running past the end, an action table that does not land
///   exactly on the dictionary, a count outside §4.3's ranges, a start room
///   that indexes nothing.
pub fn parse_c64_mysterious(image: &[u8], load_address: u16) -> Result<Database, LoadError> {
    let Some(release) = identify(image, load_address) else {
        return Err(LoadError::UnsupportedDialect(Dialect::C64OrZxSnapshot));
    };
    if read_at(image, load_address, HEADER_GUARD_ADDR, 3) != Some(&HEADER_GUARD[..]) {
        return Err(bad("no JMP $4D19 in front of the header at $5DD6"));
    }
    let dictionary = find_signature(image, load_address)
        .ok_or_else(|| bad("no AUTO/GO dictionary signature anywhere in the image"))?;
    // §6.2's free cross-check: the driver's seventh planted address IS the
    // dictionary, so two independent routes to it must agree. They do in all
    // eleven specimens, and a disagreement means the image is not what the
    // checksum claimed.
    if dictionary_pointer(image, load_address).is_some_and(|p| p != dictionary) {
        return Err(bad("the driver's dictionary pointer disagrees with the AUTO/GO signature"));
    }

    let header = read_header(image, load_address, release.shape)?;
    for (name, lo, hi) in COUNT_RANGES {
        let v = match name {
            "items" => header.items,
            "actions" => header.actions,
            "words" => header.words,
            _ => header.rooms,
        };
        // The action count is checked against the REPAIRED value, since that
        // is the one this loader goes on to use.
        let v = if name == "actions" { release.action_count.unwrap_or(v) } else { v };
        if !(lo..=hi).contains(&v) {
            return Err(bad("a header count is outside the ranges §4.3 gives"));
        }
    }
    if !(3..=5).contains(&header.word_length) {
        return Err(bad("the header's word length is not 3, 4 or 5"));
    }

    // §4.4: the action table's start is the dictionary less (count + 1)
    // records, and §6.2's free cross-check is that this lands exactly on the
    // byte after the header. Requiring both is what keeps the repaired
    // Pulsar 7 count honest — and what would catch a wrong header shape.
    let actions = release.action_count.unwrap_or(header.actions);
    let span = (actions + 1)
        .checked_mul(ACTION_RECORD)
        .ok_or_else(|| bad("the action count is absurd"))?;
    let action_addr =
        dictionary.checked_sub(span).ok_or_else(|| bad("the action table starts before the image"))?;
    if action_addr != HEADER_ADDR + release.shape.header_len() {
        return Err(bad("the action table does not begin immediately after the header"));
    }

    let tables = match pointer_block(image, load_address) {
        Some(t) => t,
        None => chained_tables(image, load_address, &header, release, dictionary)?,
    };
    for addr in [tables.rooms, tables.connections, tables.messages, tables.items, tables.locations] {
        if offset_of(image, load_address, addr).is_none() {
            return Err(bad("a table pointer resolves outside the image"));
        }
    }

    // Actions.
    let raw = read_at(image, load_address, action_addr, usize::from(span))
        .ok_or_else(|| bad("the action table runs past the end of the image"))?;
    let mut action_table = Vec::with_capacity(usize::from(actions) + 1);
    for record in raw.chunks_exact(usize::from(ACTION_RECORD)) {
        let w = |i: usize| u16::from_le_bytes([record[i * 2], record[i * 2 + 1]]);
        let mut conditions = [Condition { code: 0, value: 0 }; 5];
        for (i, c) in conditions.iter_mut().enumerate() {
            let v = w(i + 1);
            c.code = (v % 20) as u8;
            c.value = v / 20;
        }
        action_table.push(Action {
            verb: w(0) / 150,
            noun: w(0) % 150,
            conditions,
            commands: [w(6) / 150, w(6) % 150, w(7) / 150, w(7) % 150],
        });
    }

    // Dictionary: every cell between the signature hit and the room
    // descriptions, split into the two contiguous blocks §4.2 describes —
    // "in a memory image it is two contiguous blocks: every verb cell, then
    // every noun cell", not the reference format's alternating pairs.
    let width = usize::from(header.word_length) + 1;
    let capacity = usize::from(header.rooms.max(header.words)) * 4 + 512;
    let (mut cells, _) =
        read_cells(image, load_address, dictionary, Some(tables.rooms), width, capacity)
            .ok_or_else(|| bad("the dictionary runs past the end of the image"))?;
    if cells.len() < release.verb_cells {
        return Err(bad("the dictionary holds fewer cells than its verb block needs"));
    }
    let nouns = cells.split_off(release.verb_cells);
    let verbs = cells;

    // Room descriptions and connections.
    let rooms_plus_one = usize::from(header.rooms) + 1;
    let (descs, _) =
        read_strings(image, load_address, tables.rooms, Some(tables.connections), rooms_plus_one)
            .ok_or_else(|| bad("the room descriptions run past the end of the image"))?;
    let exits = read_at(
        image,
        load_address,
        tables.connections,
        rooms_plus_one * usize::from(EXITS_PER_ROOM),
    )
    .ok_or_else(|| bad("the room connections run past the end of the image"))?;
    let mut room_table = Vec::with_capacity(rooms_plus_one);
    for (desc, exits) in descs.into_iter().zip(exits.chunks_exact(usize::from(EXITS_PER_ROOM))) {
        let mut six = [0usize; 6];
        for (slot, &e) in six.iter_mut().zip(exits) {
            if usize::from(e) >= rooms_plus_one {
                return Err(bad("a room connection points outside the room table"));
            }
            *slot = usize::from(e);
        }
        // §4.4: the leading `*` meaning "print literally" is a literal byte at
        // the start of the string, exactly as in the reference format.
        let literal = desc.starts_with('*');
        room_table.push(Room {
            desc: desc.strip_prefix('*').unwrap_or(&desc).to_string(),
            literal,
            exits: six,
        });
    }

    // Messages.
    let (messages, _) = read_strings(
        image,
        load_address,
        tables.messages,
        Some(tables.items),
        usize::from(header.messages) + 1,
    )
    .ok_or_else(|| bad("the messages run past the end of the image"))?;

    // Items: descriptions, then the separate one-byte location table.
    let items_plus_one = usize::from(header.items) + 1;
    let (texts, _) = read_strings(
        image,
        load_address,
        tables.items,
        Some(tables.locations),
        items_plus_one,
    )
    .ok_or_else(|| bad("the item descriptions run past the end of the image"))?;
    let locations = read_at(image, load_address, tables.locations, items_plus_one)
        .ok_or_else(|| bad("the item locations run past the end of the image"))?;
    let mut item_table = Vec::with_capacity(items_plus_one);
    for (mut text, &loc) in texts.into_iter().zip(locations) {
        let treasure = text.starts_with('*');
        let auto_noun = extract_auto_noun(&mut text);
        item_table.push(Item {
            text,
            treasure,
            auto_noun,
            start_loc: if loc == STORED_CARRIED { CARRIED } else { i32::from(loc) },
        });
    }

    let start_room = usize::from(header.start_room);
    if start_room >= room_table.len() {
        return Err(bad("the start room indexes no room"));
    }

    Ok(Database {
        max_carry: i32::from(header.max_carry),
        start_room,
        num_treasures: i32::from(header.treasures),
        word_length: usize::from(header.word_length),
        // Sign-extended: §4.5's "lamp turns to −1 ('never runs out')" is how
        // the reference format spells an inexhaustible lamp, and $FFFF is how
        // an unsigned word spells −1. *The Wizard of Akyrz* is the one release
        // here that uses it; *Arrow of Death part 1*'s $7FFF stays 32,767, as
        // its published conversion also says.
        light_time: i32::from(header.lamp as i16),
        treasure_room: usize::from(header.treasure_room),
        actions: action_table,
        verbs,
        nouns,
        rooms: room_table,
        messages,
        items: item_table,
        // §4.3: "action comments and the trailer do not exist" in a memory
        // image, so there is no adventure number to read.
        adventure_number: 0,
        ti99: None,
        // §9.2, §6.4: this is a Mysterious Adventures release, and the two
        // lamp options that section says "every Mysterious Adventures release
        // forces on" travel with the database rather than with the host.
        mysterious: true,
        saga_us: None,
    })
}

/// The six direction words this release spells, for a host that wants to show
/// them — the head of the system-message block (§4.4).
///
/// `None` when the block is not where §4.3's search can find it. This is a
/// query rather than part of [`parse_c64_mysterious`]'s work, because these
/// releases already carry `ANY` and the six directions in noun cells 0-6:
/// §5.3's repair "noun cell 0 set to `ANY`, and noun cells 1-6 copied from the
/// first six system messages" describes a state these eleven files are already
/// in, and applying it would only truncate *The Time Machine*'s stored `NORTH`
/// and `SOUTH` to four characters. Measured on all eleven; see the specimen
/// suite.
pub fn system_direction_words(image: &[u8], load_address: u16) -> Option<[String; 6]> {
    direction_words(image, load_address)
}

// ── Family B pictures (§8.2) ──────────────────────────────────────────────────

/// The Family B canvas width, in pixels (§8.2).
pub const PICTURE_WIDTH: usize = 255;

/// The Family B canvas height, in pixels (§8.2).
pub const PICTURE_HEIGHT: usize = 94;

/// §8.2's bottom-up coordinate flip: a stored vertical value *v* means canvas
/// row `190 - v`, so pictures occupy the top part of what was a 192-line
/// screen.
const VERTICAL_ORIGIN: i32 = 190;

/// §8.2's fill queue bound: 1024 points, with further enqueues silently
/// dropped, so a fill of a large region can terminate early and leave holes.
/// That is observable output — a decoder with an unbounded queue produces
/// different, more complete pictures — and this decoder reproduces the
/// original's behaviour deliberately.
const FILL_QUEUE_LIMIT: usize = 1024;

/// Opcode: move the current point. Operands are *v* then *h* — **vertical
/// first** (§8.2).
const OP_MOVE: u8 = 0xC0;
/// Opcode: flood fill. Operands are colour, *v*, *h*.
const OP_FILL: u8 = 0xC1;
/// Opcode: end of image, and simultaneously the introduction of the next.
const OP_END: u8 = 0xFF;

/// The Commodore 64 palette §8.2 says every Commodore 64 release uses, already
/// composed with §8.2's **remap table A** — the eight-colour mapping that
/// serves the Mysterious Adventures releases, because these files store the
/// *ZX Spectrum's* colour indices and the numbering was never converted.
///
/// So `PALETTE[i]` is the RGB a stored index *i* resolves to, both lookups
/// done. The remap it embeds is
/// `0 6 2 4 5 3 7 1 8 1 1 1 7 12 8 7`, and the Commodore 64 palette it indexes
/// is the one §8.2 tabulates. §11 restates that "the four remap tables were
/// derived by eye and may contain mistakes"; that caveat travels with this
/// constant.
pub const PALETTE: [(u8, u8, u8); 16] = {
    const C64: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (255, 255, 255),
        (191, 97, 72),
        (153, 230, 249),
        (177, 89, 185),
        (121, 213, 112),
        (95, 72, 233),
        (247, 255, 108),
        (186, 134, 32),
        (116, 105, 0),
        (231, 154, 132),
        (69, 69, 69),
        (167, 167, 167),
        (192, 255, 185),
        (162, 143, 255),
        (200, 200, 200),
    ];
    const REMAP_A: [usize; 16] = [0, 6, 2, 4, 5, 3, 7, 1, 8, 1, 1, 1, 7, 12, 8, 7];
    let mut out = [(0u8, 0u8, 0u8); 16];
    let mut i = 0;
    while i < 16 {
        out[i] = C64[REMAP_A[i]];
        i += 1;
    }
    out
};

/// The largest supersample [`PictureList::rasterise_at`] will draw, so a
/// caller cannot ask a 255 x 94 canvas for gigabytes by arithmetic accident.
/// Eight is already 2040 x 752, more device pixels than any terminal picture
/// band this artwork is shown in (SQ-1467).
pub const MAX_PICTURE_SCALE: u32 = 8;

/// One primitive of a Family B display list, in **native canvas coordinates**
/// — §8.2's opcode stream after its two state variables (the current point and
/// the image-wide line colour) have been resolved away, so each op stands
/// alone and can be drawn on a canvas of any size (SQ-1467).
///
/// A coordinate here is the one §8.2's flip already produced (`h`, `190 - v`)
/// and is deliberately **not** clipped: a line that runs off the canvas still
/// paints the part that is on it, and clipping the endpoints would move the
/// line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureOp {
    /// §8.2's line opcode: draw from `from` to `to` in the image's line
    /// colour ([`PictureList::line`]), both endpoints inclusive.
    Line {
        /// The current point when the opcode ran.
        from: (i32, i32),
        /// The endpoint the opcode named, which becomes the current point.
        to: (i32, i32),
    },
    /// §8.2's `0xC1`: flood fill from `seed` in `colour`.
    Fill {
        /// The point the opcode named.
        seed: (i32, i32),
        /// The fill colour, a raw 0-15 index like every other value here.
        colour: u8,
    },
}

/// One Family B drawing as the **display list** it is stored as: a background,
/// a derived line colour and §8.2's primitives in stream order (SQ-1467).
///
/// This is the form the file actually holds. [`Picture`] is what one *looks
/// like* once drawn, and the drawing is a separate step because nothing in the
/// data fixes a size: [`Self::rasterise`] draws §8.2's own 255 x 94 canvas and
/// [`Self::rasterise_at`] draws the same picture at an integer multiple of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureList {
    /// The image's background colour index — the canvas's starting value and
    /// the fill algorithm's boundary test (§8.2).
    pub background: u8,
    /// The line colour, derived rather than stored: **7 when the background
    /// index is 0, and 0 otherwise** (§8.2).
    pub line: u8,
    /// §8.2's primitives in stream order, which is load-bearing: each fill's
    /// boundary test sees everything drawn before it.
    pub ops: Vec<PictureOp>,
}

/// The one Bresenham in this module (§8.2's, exactly), walked at `scale`
/// device pixels per native pixel and handing every device pixel of the pen to
/// `pen` (SQ-1467).
///
/// §8.2: take the absolute deltas and step signs; if the horizontal delta is
/// the larger, double both deltas, set the error accumulator to (vertical
/// delta − horizontal delta), and step horizontally, adding a vertical step
/// and subtracting the horizontal delta whenever the accumulator is
/// non-negative, then adding the vertical delta; otherwise do the same with
/// the roles exchanged. Plot before each step and once more at the end.
///
/// **The pen is a `scale`-long run across the minor axis**, which is what
/// makes a scaled line the same line rather than a thicker one: an x-major
/// line puts exactly one pixel in every native column at 1x, so it puts
/// exactly `scale` device pixels in every device column at `scale`. A square
/// brush would instead widen every diagonal by √2.
///
/// **Both endpoints are capped with the whole `scale` x `scale` block**,
/// because at 1x an endpoint is a whole native pixel and a minor-axis run
/// covers only the near edge of one: the last stamp of a y-major line lands on
/// a single device row of the endpoint's block, leaving five of a 3 x 3 corner
/// unpainted and a rectangle's corner missing a bite. The cap also makes two
/// lines that share an endpoint share its whole block, which is what keeps a
/// joint sealed.
///
/// At `scale` 1 the run and the cap are both the single pixel §8.2 plots, so
/// this is the original loop, step for step.
fn bresenham(from: (i32, i32), to: (i32, i32), scale: i32, mut pen: impl FnMut(i32, i32)) {
    let (mut x, mut y) = (from.0 * scale, from.1 * scale);
    let (tx, ty) = (to.0 * scale, to.1 * scale);
    for dy in 0..scale {
        for dx in 0..scale {
            pen(x + dx, y + dy);
            pen(tx + dx, ty + dy);
        }
    }
    let dx = (tx - x).abs();
    let dy = (ty - y).abs();
    let sx = if tx >= x { 1 } else { -1 };
    let sy = if ty >= y { 1 } else { -1 };
    if dx >= dy {
        let (dx2, dy2) = (dx * 2, dy * 2);
        let mut err = dy2 - dx;
        for _ in 0..dx {
            for k in 0..scale {
                pen(x, y + k);
            }
            if err >= 0 {
                y += sy;
                err -= dx2;
            }
            x += sx;
            err += dy2;
        }
        for k in 0..scale {
            pen(tx, ty + k);
        }
    } else {
        let (dx2, dy2) = (dx * 2, dy * 2);
        let mut err = dx2 - dy;
        for _ in 0..dy {
            for k in 0..scale {
                pen(x + k, y);
            }
            if err >= 0 {
                x += sx;
                err -= dy2;
            }
            y += sy;
            err += dx2;
        }
        for k in 0..scale {
            pen(tx + k, ty);
        }
    }
}

/// A device pixel [`PictureList::supersample`] has not decided yet. Palette
/// indices are 0-15, so any value above them is free for the purpose.
const UNSET: u8 = 0xFF;

impl PictureList {
    /// Draw this list at §8.2's own 255 x 94 canvas — the original's output,
    /// bit for bit, quirks and fill-queue bound included.
    pub fn rasterise(&self) -> Picture {
        let mut picture = Picture::blank(self.background, 1);
        for op in &self.ops {
            match *op {
                PictureOp::Line { from, to } => picture.stroke(from, to, self.line),
                PictureOp::Fill { seed, colour } => picture.flood(seed, colour),
            }
        }
        picture
    }

    /// Draw this picture `scale` times larger on each axis, clamped to
    /// 1..=[`MAX_PICTURE_SCALE`] (SQ-1467).
    ///
    /// # The rule: the native raster owns the REGIONS, the supersample redraws
    /// the LINES
    ///
    /// [`Self::rasterise`] runs first and is the authority on which region
    /// every native pixel belongs to, because it is the picture the Commodore
    /// 64 actually drew. The large canvas then re-walks only the line ops at
    /// device resolution — so a staircase is resolved `scale` times as finely,
    /// which is the whole point — and every device pixel takes its colour from
    /// the native pixel it lies in:
    ///
    /// - a device pixel the scaled line covers is the line colour;
    /// - otherwise, if its native pixel was **not** line at 1x, it takes that
    ///   native pixel's colour, fill or background;
    /// - otherwise its native pixel *was* line at 1x and the finer line has
    ///   moved off part of it, so it takes the colour of whichever neighbour
    ///   it reaches first breadth-first without crossing the scaled line — and
    ///   the line colour if it is enclosed and reaches none.
    ///
    /// # Why the fills are not simply re-run on the big canvas
    ///
    /// Because they leak, and not rarely. A fill sealed at 1x by two lines a
    /// single pixel apart, or by two whose 1x rounding put them in the same
    /// row, finds a half-pixel seam once the lines are drawn where the
    /// geometry actually puts them, floods through it and repaints the region
    /// on the other side. Measured over the eleven Commodore 64 releases of
    /// §10.4 with the fills re-run at scale, **25 images of 516 leaked at one
    /// scale or another**, the worst of them (Ten Little Indians image 2)
    /// flooding 6,118 of the 23,970 native pixels — a quarter of the canvas
    /// the wrong colour. Nor is it fixable by thickening the pen: the 1x pixel
    /// a shallow line lands in is up to a whole pixel away from where the line
    /// truly runs, so nothing thinner than a **two**-pixel-wide stroke can
    /// cover it, and a two-pixel stroke is no longer this artwork's line.
    ///
    /// Sub-pixel staircases and 1x region topology are simply not both
    /// available, and between them the region topology is the one the player
    /// saw. Taking it from the native raster makes a leak unrepresentable
    /// rather than merely unlikely — `every_picture_keeps_its_regions_at_every_supersample`
    /// (`crates/scott/tests/c64_specimens.rs`) is the corpus check, and it
    /// passes at scales 2, 3 and 4 with no disagreement the ink does not
    /// explain.
    ///
    /// It also means §8.2's flood — its FIFO order, its background-only
    /// boundary test, its 1024-point queue bound — runs exactly once
    /// per picture, on the canvas it was measured on, and needs no opinion
    /// about what any of it should become at scale.
    ///
    /// # The one deviation
    ///
    /// §8.2's "bounds quirk" — column 255 is admitted even though rows are 255
    /// pixels wide, so a pixel plotted there lands at column 0 of the next row
    /// — is a property of byte-wide plotting into a flat buffer and exists
    /// only at 1x, where it is reproduced. A scaled canvas has no column that
    /// means "one past the row", so the scaled line drops such a point, which
    /// is §8.2's own sanctioned alternative ("otherwise clamp to 0-254 and
    /// note the deviation"). The native raster still carries the quirk's
    /// pixel, and the supersample takes its colour from there like any other.
    pub fn rasterise_at(&self, scale: u32) -> Picture {
        let scale = scale.clamp(1, MAX_PICTURE_SCALE);
        if scale == 1 {
            return self.rasterise();
        }
        self.supersample(scale)
    }

    /// Which pixels of a `scale`-times-larger canvas the line ops touch —
    /// [`bresenham`] over every [`PictureOp::Line`], and nothing else. At
    /// `scale` 1 this is the native raster's own ink, which is what tells
    /// [`Self::supersample`] whether a native pixel's colour is a line or a
    /// fill that happened to use the line's colour index.
    fn ink_mask(&self, scale: u32) -> Vec<bool> {
        let (w, h) = (PICTURE_WIDTH * scale as usize, PICTURE_HEIGHT * scale as usize);
        let mut mask = vec![false; w * h];
        for op in &self.ops {
            let PictureOp::Line { from, to } = *op else { continue };
            bresenham(from, to, scale as i32, |x, y| {
                // The 1x pass reproduces §8.2's column-255 wrap, exactly as
                // `Picture::plot` does, so the two masks agree pixel for
                // pixel with the raster they describe; a scaled canvas has no
                // such column and drops the point.
                let limit = if scale == 1 { w as i32 } else { w as i32 - 1 };
                if !(0..=limit).contains(&x) || !(0..h as i32).contains(&y) {
                    return;
                }
                if let Some(m) = mask.get_mut(y as usize * w + x as usize) {
                    *m = true;
                }
            });
        }
        mask
    }

    /// [`Self::rasterise_at`]'s body for a scale above 1 — see its doc for the
    /// rule and for why it is that rule.
    fn supersample(&self, scale: u32) -> Picture {
        let native = self.rasterise();
        let native_ink = self.ink_mask(1);
        let big_ink = self.ink_mask(scale);
        let mut big = Picture::blank(self.background, scale);
        let s = scale as usize;
        let (w, h) = (big.width, big.height);
        let mut out = vec![UNSET; w * h];

        // The scaled line, and every device pixel whose native pixel the 1x
        // line did not touch.
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if big_ink[i] {
                    out[i] = self.line;
                    continue;
                }
                let n = (y / s) * native.width + x / s;
                if !native_ink[n] {
                    out[i] = native.pixels[n];
                }
            }
        }

        // What is left is the part of a 1x line's pixel that the finer line
        // has moved off: give it the region it is 4-connected to without
        // crossing the scaled line, breadth-first so the nearest wins, seeded
        // from every region pixel above (never from the line itself, which
        // would paint the vacated part the line colour and put the thickness
        // straight back).
        let mut queue: std::collections::VecDeque<usize> = (0..out.len())
            .filter(|&i| {
                if out[i] == UNSET || big_ink[i] {
                    return false;
                }
                let (x, y) = ((i % w) as i32, (i / w) as i32);
                [(x, y + 1), (x, y - 1), (x + 1, y), (x - 1, y)].into_iter().any(|(nx, ny)| {
                    (0..w as i32).contains(&nx)
                        && (0..h as i32).contains(&ny)
                        && out[ny as usize * w + nx as usize] == UNSET
                })
            })
            .collect();
        while let Some(i) = queue.pop_front() {
            let colour = out[i];
            let (x, y) = ((i % w) as i32, (i / w) as i32);
            for (nx, ny) in [(x, y + 1), (x, y - 1), (x + 1, y), (x - 1, y)] {
                if !(0..w as i32).contains(&nx) || !(0..h as i32).contains(&ny) {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if out[j] == UNSET {
                    out[j] = colour;
                    queue.push_back(j);
                }
            }
        }

        // Enclosed by ink on every side: it IS the line.
        big.pixels = out.into_iter().map(|v| if v == UNSET { self.line } else { v }).collect();
        big
    }
}

/// One decoded Family B drawing: an indexed bitmap, one palette index per
/// pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// [`PICTURE_WIDTH`] x [`Self::scale`]; carried so a consumer need not do
    /// the arithmetic.
    pub width: usize,
    /// [`PICTURE_HEIGHT`] x [`Self::scale`].
    pub height: usize,
    /// The supersample this raster was drawn at — 1 for §8.2's own canvas, and
    /// whatever [`PictureList::rasterise_at`] was asked for otherwise
    /// (SQ-1467). A consumer that needs the picture's true aspect wants
    /// `width / height`, which the supersample leaves alone; this is here for
    /// diagnostics and for a caller that wants to map a device pixel back to
    /// the native one it came from.
    pub scale: u32,
    /// The image's background colour index, which the canvas starts filled
    /// with and which is also the fill algorithm's boundary test (§8.2).
    pub background: u8,
    /// The line colour, derived rather than stored: **7 when the background
    /// index is 0, and 0 otherwise** (§8.2).
    pub line: u8,
    /// `width * height` palette indices, row-major from the top-left. Each is
    /// a **stored** index in the 0-15 space, so [`PALETTE`] — which already
    /// carries §8.2's remap — is the lookup that turns one into a colour.
    pub pixels: Vec<u8>,
    /// [`PALETTE`], carried so a `Picture` is self-contained.
    pub palette: &'static [(u8, u8, u8); 16],
}

impl Picture {
    /// A canvas filled with `background`, before any opcode has run, at
    /// `scale` device pixels per native pixel on each axis.
    fn blank(background: u8, scale: u32) -> Picture {
        let (width, height) = (PICTURE_WIDTH * scale as usize, PICTURE_HEIGHT * scale as usize);
        Picture {
            width,
            height,
            scale,
            background,
            // §8.2: "if the background colour index is 0 the line colour is 7;
            // otherwise it is 0."
            line: if background == 0 { 7 } else { 0 },
            pixels: vec![background; width * height],
            palette: &PALETTE,
        }
    }

    /// The RGB of the pixel at `(x, y)`, or `None` off the canvas.
    ///
    /// Bounded on both axes, unlike the decoder's own plotting: the 255-column wrap
    /// §8.2 describes is a property of how the original PLOTTED, not an
    /// invitation to read row *r* + 1 by asking for column 255 of row *r*.
    pub fn rgb(&self, x: usize, y: usize) -> Option<(u8, u8, u8)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = *self.pixels.get(y * self.width + x)?;
        Some(self.palette[usize::from(i) & 15])
    }

    /// Paint one pixel.
    ///
    /// §8.2's one bounds quirk is reproduced rather than clamped, because it
    /// costs nothing to reproduce: the horizontal test admits column 255 as
    /// well as 0-254 even though rows are 255 pixels wide, so a pixel plotted
    /// at column 255 of row *r* lands at column 0 of row *r* + 1 — which flat
    /// row-major indexing does by itself. On the last row it falls off the end
    /// and is dropped.
    fn plot(&mut self, x: i32, y: i32, colour: u8) {
        if !(0..=PICTURE_WIDTH as i32).contains(&x) || !(0..PICTURE_HEIGHT as i32).contains(&y) {
            return;
        }
        let i = y as usize * self.width + x as usize;
        if let Some(p) = self.pixels.get_mut(i) {
            *p = colour;
        }
    }

    /// The pixel at `(x, y)`, or `None` off the canvas — the fill's boundary
    /// test, which stays inside 0..254 horizontally (§8.2 resolves its own
    /// unsigned-byte wrap that way: "signed arithmetic gives the same
    /// result").
    fn at(&self, x: i32, y: i32) -> Option<u8> {
        if !(0..PICTURE_WIDTH as i32).contains(&x) || !(0..PICTURE_HEIGHT as i32).contains(&y) {
            return None;
        }
        self.pixels.get(y as usize * self.width + x as usize).copied()
    }

    /// §8.2's line, drawn on §8.2's canvas: [`bresenham`] at scale 1 with
    /// [`Self::plot`] as the pen.
    fn stroke(&mut self, from: (i32, i32), to: (i32, i32), colour: u8) {
        bresenham(from, to, 1, |x, y| self.plot(x, y, colour));
    }

    /// §8.2's flood fill, exactly: 4-connected, seeded at one point,
    /// breadth-first over a FIFO queue rather than recursion or scanlines,
    /// with **the test and the paint both at dequeue time** (so the queue
    /// routinely holds duplicates), a pixel filled only if its current value
    /// equals the image's **background** index — not the colour at the seed —
    /// and the queue bounded at [`FILL_QUEUE_LIMIT`] with further enqueues
    /// silently dropped.
    ///
    /// Neighbours are enqueued in the order below, above, right, left.
    ///
    /// Only ever run on §8.2's own 255 x 94 canvas: a supersample takes its
    /// regions from that one raster rather than re-flooding a larger one, so
    /// none of this algorithm's measured behaviour has to be re-argued at a
    /// size the original never had (see [`PictureList::rasterise_at`]).
    fn flood(&mut self, seed: (i32, i32), colour: u8) {
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(seed);
        while let Some((x, y)) = queue.pop_front() {
            if self.at(x, y) != Some(self.background) {
                continue;
            }
            self.plot(x, y, colour);
            for next in [(x, y + 1), (x, y - 1), (x + 1, y), (x - 1, y)] {
                if queue.len() < FILL_QUEUE_LIMIT {
                    queue.push_back(next);
                }
            }
        }
    }
}

/// Decode a Family B opcode block into one [`PictureList`] per room (§8.2) —
/// the stored primitives, before any canvas exists (SQ-1467).
///
/// `block` is the whole picture region, **starting at its leading `0xFF`**.
/// It is a plain concatenation with no index table: each image is a
/// background-colour byte followed by an opcode stream terminated by `0xFF`,
/// and that terminator simultaneously introduces the next. Decoding stops when
/// the block is exhausted; §11's rule that data truncating mid-record "leaves
/// the remaining rooms with no recoverable picture" is honoured by returning
/// only the images that decoded whole, so a caller can compare the count
/// against the room count and tell the player how many were lost.
///
/// **Room *n* shows image *n* − 1** (§8.6, "Family B. Pure identity"), so a
/// game with *N* rooms carries *N* images and room 0 — the copyright slot —
/// has none.
///
/// A block whose first byte is not `0xFF` yields nothing (§11).
pub fn decode_family_b_lists(block: &[u8]) -> Vec<PictureList> {
    let mut out = Vec::new();
    let Some((&first, mut rest)) = block.split_first() else {
        return out;
    };
    if first != OP_END {
        return out;
    }
    while let Some((&background, stream)) = rest.split_first() {
        // §8.2: "if the background colour index is 0 the line colour is 7;
        // otherwise it is 0."
        let mut list =
            PictureList { background, line: if background == 0 { 7 } else { 0 }, ops: Vec::new() };
        // §8.2: "a current point, initially (0, 0), and a line colour fixed
        // for the whole image."
        let mut point = (0i32, 0i32);
        let mut i = 0usize;
        let ended = loop {
            let Some(&op) = stream.get(i) else { break false };
            i += 1;
            match op {
                OP_END => break true,
                OP_MOVE => {
                    let (Some(&v), Some(&h)) = (stream.get(i), stream.get(i + 1)) else {
                        break false;
                    };
                    i += 2;
                    point = (i32::from(h), VERTICAL_ORIGIN - i32::from(v));
                }
                OP_FILL => {
                    let (Some(&c), Some(&v), Some(&h)) =
                        (stream.get(i), stream.get(i + 1), stream.get(i + 2))
                    else {
                        break false;
                    };
                    i += 3;
                    let seed = (i32::from(h), VERTICAL_ORIGIN - i32::from(v));
                    list.ops.push(PictureOp::Fill { seed, colour: c });
                }
                // 0x00-0xBF: the opcode byte is itself the vertical operand.
                _ => {
                    let Some(&h) = stream.get(i) else { break false };
                    i += 1;
                    let end = (i32::from(h), VERTICAL_ORIGIN - i32::from(op));
                    list.ops.push(PictureOp::Line { from: point, to: end });
                    point = end;
                }
            }
        };
        if !ended {
            break;
        }
        out.push(list);
        rest = &stream[i..];
        // The block ends when the last image's terminator was the last byte.
        if rest.is_empty() {
            break;
        }
    }
    out
}

/// Decode a Family B opcode block and draw every image at §8.2's own 255 x 94
/// canvas — [`decode_family_b_lists`] plus [`PictureList::rasterise`], which is
/// the shape this function has always had and the output it has always
/// produced.
pub fn decode_family_b_block(block: &[u8]) -> Vec<Picture> {
    decode_family_b_lists(block).iter().map(PictureList::rasterise).collect()
}

/// Locate a release's artwork and decode it to display lists: the picture
/// region of a memory image through [`decode_family_b_lists`] (SQ-1467).
///
/// The region's start is derivable and needs no catalogue: it is the first
/// `0xFF` at or after the address the driver plants in zero page as the byte
/// following the item-location table, past the run of 61 to 113 zero bytes
/// that separates them — 61, 76 or 101 bytes of it (§6.2's layout diagram).
///
/// # Errors
///
/// The same refusals [`parse_c64_mysterious`] gives, for the same reasons.
pub fn decode_family_b_picture_lists(
    image: &[u8],
    load_address: u16,
) -> Result<Vec<PictureList>, LoadError> {
    if identify(image, load_address).is_none() {
        return Err(LoadError::UnsupportedDialect(Dialect::C64OrZxSnapshot));
    }
    let after_locations = pointer_block(image, load_address)
        .ok_or_else(|| bad("no driver pointer block, so the picture region cannot be located"))?
        .locations_end;
    let from = offset_of(image, load_address, after_locations)
        .ok_or_else(|| bad("the end-of-locations pointer resolves outside the image"))?;
    let start = image[from..]
        .iter()
        .position(|&b| b == OP_END)
        .ok_or_else(|| bad("no picture block after the item-location table"))?;
    Ok(decode_family_b_lists(&image[from + start..]))
}

/// Locate and decode a release's artwork at §8.2's own 255 x 94 canvas —
/// [`decode_family_b_picture_lists`] plus [`PictureList::rasterise`], which is
/// the shape this function has always had and the output it has always
/// produced.
///
/// # Errors
///
/// The same refusals [`parse_c64_mysterious`] gives, for the same reasons.
pub fn decode_family_b_pictures(
    image: &[u8],
    load_address: u16,
) -> Result<Vec<Picture>, LoadError> {
    Ok(decode_family_b_picture_lists(image, load_address)?
        .iter()
        .map(PictureList::rasterise)
        .collect())
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, Presentation, Vm};

    // ── A hand-built image, constructed from the format above ─────────────────

    /// Which catalogued release the hand-built image impersonates.
    /// *Feasibility Experiment* is the plain [`HeaderShape::Early`] shape with
    /// the smallest verb block and no load-time repair — the least machinery
    /// between the bytes and the assertions.
    const FIXTURE: &Release = &RELEASES[6];

    const LOAD: u16 = 0x4000;
    /// Where the builder puts each table. Chosen, not derived: the point of the
    /// fixture is that the loader finds them through the pointer block rather
    /// than by assuming any of these.
    const ROOMS_AT: u16 = fixture_rooms();
    const CONNS_AT: u16 = 0x7100;
    const MSGS_AT: u16 = 0x7200;
    const ITEMS_AT: u16 = 0x7300;
    const LOCS_AT: u16 = 0x7400;
    const PICS_AT: u16 = 0x7500;
    /// A run of bytes the builder is free to tune so the whole image sums to
    /// `FIXTURE.checksum` — see [`balance`]. The checksum is a sum of BYTES,
    /// so absorbing an arbitrary 16-bit difference takes 258 of them and not
    /// two.
    const BALLAST_AT: u16 = 0x4100;
    const BALLAST_LEN: usize = 258;

    /// The world the fixture describes: small enough to assert by hand, and
    /// still inside §4.3's plausibility ranges (items 10-500, actions 100-500,
    /// words 50-190, rooms 10-100).
    const ROOMS: u16 = 10;
    const ITEMS: u16 = 10;
    const ACTIONS: u16 = 100;
    const WORDS: u16 = 50;
    const WORD_LENGTH: u16 = 4;
    const MESSAGES: u16 = 3;

    struct Fixture {
        mem: Vec<u8>,
    }

    impl Fixture {
        fn new() -> Fixture {
            Fixture { mem: vec![0u8; 0x8000 - usize::from(LOAD)] }
        }

        fn put(&mut self, addr: u16, bytes: &[u8]) {
            let at = usize::from(addr - LOAD);
            self.mem[at..at + bytes.len()].copy_from_slice(bytes);
        }

        fn put_word(&mut self, addr: u16, v: u16) {
            self.put(addr, &v.to_le_bytes());
        }

        /// One `LDA #imm` / `STA <zp>` pair of the driver's pointer block.
        fn pointer(&mut self, index: usize, zp: u8, imm: u8) {
            let at = POINTER_BLOCK + u16::try_from(index).unwrap() * 4;
            self.put(at, &[OP_LDA_IMM, imm, OP_STA_ZP, zp]);
        }

        fn address(&mut self, index: usize, slots: (u8, u8), addr: u16) {
            let [lo, hi] = addr.to_le_bytes();
            self.pointer(index * 2, slots.0, lo);
            self.pointer(index * 2 + 1, slots.1, hi);
        }

        /// Tune [`BALLAST_AT`] until the image checksums to the release it
        /// impersonates.
        fn balance(&mut self) {
            balance(&mut self.mem);
        }
    }

    /// Rewrite the ballast so `mem` checksums to the release the fixture
    /// impersonates — used both when building the image and after a test
    /// deliberately corrupts one, since a checksum that no longer matches
    /// would hide the corruption behind an "unknown release" refusal.
    fn balance(mem: &mut [u8]) {
        let ballast = usize::from(BALLAST_AT - LOAD);
        mem[ballast..ballast + BALLAST_LEN].fill(0);
        let mut short = FIXTURE.checksum.wrapping_sub(image_checksum(mem, LOAD));
        for slot in &mut mem[ballast..ballast + BALLAST_LEN] {
            let step = short.min(255);
            *slot = step as u8;
            short -= step;
        }
        assert_eq!(short, 0, "258 bytes absorb any 16-bit difference");
        assert_eq!(image_checksum(mem, LOAD), FIXTURE.checksum);
    }

    /// Is `at` one of the bytes [`balance`] is free to rewrite?
    fn in_ballast(at: usize) -> bool {
        (usize::from(BALLAST_AT - LOAD)..usize::from(BALLAST_AT - LOAD) + BALLAST_LEN).contains(&at)
    }

    /// The address the fixture's dictionary lands at: §6.2's rule that the
    /// action table begins immediately after the header, and §4.4's that it is
    /// (action count + 1) sixteen-byte records long.
    const fn fixture_dictionary() -> u16 {
        HEADER_ADDR + HeaderShape::Early.header_len() + (ACTIONS + 1) * ACTION_RECORD
    }

    /// …and where its room descriptions begin, which is where the dictionary
    /// ends: (verb cells + noun cells) cells of (word length + 1) bytes. The
    /// tables chain, so this cannot be a chosen constant — a gap here would
    /// be read as hundreds of blank noun cells.
    const fn fixture_rooms() -> u16 {
        let cells = FIXTURE.verb_cells + (WORDS as usize + 1);
        fixture_dictionary() + (cells as u16) * (WORD_LENGTH + 1)
    }

    /// A whole valid image of the shape this module reads, built from the
    /// format rather than captured from a specimen.
    fn hand_built() -> Vec<u8> {
        let mut f = Fixture::new();

        // §4.4: Commodore 64 English releases take their direction words from
        // the head of the system-message block, which begins with NORTH.
        f.put(SYSTEM_MESSAGES_ADDR, b"NORTH\0SOUTH\0EAST\0WEST\0UP\0DOWN\0EXITS: \0");

        // The header, in the reference format's own field order, behind the
        // `JMP $4D19` whose operand is word 0.
        f.put(HEADER_GUARD_ADDR, &HEADER_GUARD);
        let h = HEADER_ADDR;
        f.put_word(h + 2, ITEMS);
        f.put_word(h + 4, ACTIONS);
        f.put_word(h + 6, WORDS);
        f.put_word(h + 8, ROOMS);
        f.put_word(h + 10, 6); // max carried
        f.put_word(h + 12, 2); // start room
        f.put_word(h + 14, 1); // treasure count
        f.put_word(h + 16, WORD_LENGTH);
        f.put_word(h + 18, 125); // lamp turns
        f.put_word(h + 20, MESSAGES);
        f.put_word(h + 22, 4); // treasure room

        // Actions begin immediately after the header, and the dictionary
        // immediately after them.
        let action_at = HEADER_ADDR + HeaderShape::Early.header_len();
        let dictionary = fixture_dictionary();
        // Action 0: verb 1 ("GO"), noun 2, one condition (code 4, value 3),
        // and commands 52 and 63 in the two command words.
        f.put_word(action_at, 150 + 2);
        f.put_word(action_at + 2, 4 + 20 * 3);
        f.put_word(action_at + 12, 52 * 150);
        f.put_word(action_at + 14, 63 * 150);
        // Action 1: a verb-0 occurrence line that always fires.
        f.put_word(action_at + 16, 100);

        // The dictionary: `verb_cells` verbs then (WORDS + 1) nouns, in
        // (word length + 1)-byte NUL-padded cells.
        let width = u16::try_from(usize::from(WORD_LENGTH) + 1).unwrap();
        let mut at = dictionary;
        let cell = |f: &mut Fixture, at: &mut u16, text: &[u8]| {
            f.put(*at, text);
            *at += width;
        };
        cell(&mut f, &mut at, b"AUTO\0");
        cell(&mut f, &mut at, b"GO\0\0\0");
        cell(&mut f, &mut at, b"*WALK");
        cell(&mut f, &mut at, b"TAKE\0");
        for _ in 4..FIXTURE.verb_cells {
            cell(&mut f, &mut at, b"    \0");
        }
        cell(&mut f, &mut at, b"ANY\0\0");
        cell(&mut f, &mut at, b"NORT\0");
        cell(&mut f, &mut at, b"LAMP\0");
        for _ in 3..=usize::from(WORDS) {
            cell(&mut f, &mut at, b"    \0");
        }

        // Rooms: NUL-terminated descriptions, then a separate six-byte
        // connection record each. Rooms 3 upward have a lone NUL for a
        // description, which is an empty one.
        let mut at = ROOMS_AT;
        for text in ["(C) 1984", "*Outside a hut", "a dusty study"] {
            f.put(at, text.as_bytes());
            at += u16::try_from(text.len()).unwrap() + 1;
        }
        // Room 1 leads north to room 2; room 2 leads south to room 1.
        f.put(CONNS_AT + 6, &[2, 0, 0, 0, 0, 0]);
        f.put(CONNS_AT + 12, &[0, 1, 0, 0, 0, 0]);

        let mut at = MSGS_AT;
        for text in ["", "A voice booms out.", "Nothing happens.", "The end."] {
            f.put(at, text.as_bytes());
            at += u16::try_from(text.len()).unwrap() + 1;
        }

        let mut at = ITEMS_AT;
        for text in ["*A gold crown", "A brass lamp/LAMP/", "Some scenery"] {
            f.put(at, text.as_bytes());
            at += u16::try_from(text.len()).unwrap() + 1;
        }
        // Items 3..=10 have no stored description at all — the block simply
        // ends where the next one begins. That is §5.3's Time Machine repair,
        // generalised: the location table is still a full (items + 1) bytes.
        f.put(LOCS_AT, &[0, 2, STORED_CARRIED, 1, 0, 0, 0, 0, 0, 0, 0]);

        // The pointer block, in the order and the zero-page slots the driver
        // writes them, plus §6.2's seventh address (the dictionary).
        f.address(0, ZP_ROOMS, ROOMS_AT);
        f.address(1, ZP_CONNECTIONS, CONNS_AT);
        f.address(2, ZP_MESSAGES, MSGS_AT);
        f.address(3, ZP_ITEMS, ITEMS_AT);
        f.address(4, ZP_LOCATIONS, LOCS_AT);
        f.address(5, ZP_LOCATIONS_END, PICS_AT);
        let [lo, hi] = dictionary.to_le_bytes();
        f.put(DICTIONARY_POINTER.0, &[lo]);
        f.put(DICTIONARY_POINTER.1, &[hi]);

        // One Family B image: background 1, a move, a line, a fill, and the
        // terminator that both ends it and would introduce the next.
        f.put(PICS_AT, &[OP_END, 1, OP_MOVE, 190, 0, 100, 8, OP_FILL, 5, 150, 4, OP_END]);

        f.balance();
        f.mem
    }

    fn parsed() -> Database {
        parse_c64_mysterious(&hand_built(), LOAD).expect("the hand-built image parses")
    }

    #[test]
    fn hand_built_image_decodes_to_the_expected_database() {
        let db = parsed();

        // The header, field for field.
        assert_eq!(db.max_carry, 6);
        assert_eq!(db.start_room, 2);
        assert_eq!(db.num_treasures, 1);
        assert_eq!(db.word_length, 4);
        assert_eq!(db.light_time, 125);
        assert_eq!(db.treasure_room, 4);
        assert!(db.mysterious, "a Mysterious release identifies itself as one");
        assert!(db.ti99.is_none());
        // §4.3: a memory image has no trailer, so there is no adventure number.
        assert_eq!(db.adventure_number, 0);

        // Every table is (count + 1) long, as §4.4 requires.
        assert_eq!(db.actions.len(), usize::from(ACTIONS) + 1);
        assert_eq!(db.rooms.len(), usize::from(ROOMS) + 1);
        assert_eq!(db.items.len(), usize::from(ITEMS) + 1);
        assert_eq!(db.messages.len(), usize::from(MESSAGES) + 1);
        assert_eq!(db.verbs.len(), FIXTURE.verb_cells);
        assert_eq!(db.nouns.len(), usize::from(WORDS) + 1);

        // The action encoding: verb x 150 + noun, code + 20 x value, and each
        // command word holding two commands as quotient and remainder by 150.
        assert_eq!(db.actions[0].verb, 1);
        assert_eq!(db.actions[0].noun, 2);
        assert_eq!(db.actions[0].conditions[0], Condition { code: 4, value: 3 });
        assert_eq!(db.actions[0].conditions[1], Condition { code: 0, value: 0 });
        assert_eq!(db.actions[0].commands, [52, 0, 63, 0]);
        assert_eq!((db.actions[1].verb, db.actions[1].noun), (0, 100));

        // Rooms: the leading `*` is stripped and remembered as `literal`, and
        // the exits are the six unsigned bytes of the separate table.
        assert_eq!(db.rooms[0].desc, "(C) 1984");
        assert!(!db.rooms[0].literal);
        assert_eq!(db.rooms[1].desc, "Outside a hut");
        assert!(db.rooms[1].literal, "the leading * means print literally");
        assert_eq!(db.rooms[1].exits, [2, 0, 0, 0, 0, 0]);
        assert_eq!(db.rooms[2].desc, "a dusty study");
        assert_eq!(db.rooms[2].exits, [0, 1, 0, 0, 0, 0]);
        assert_eq!(db.rooms[10].desc, "");

        // Items: the treasure `*`, the `/WORD/` auto-noun, 255 as CARRIED, and
        // §5.3's empty description for an item the block has no room for.
        assert_eq!(db.items[0].text, "*A gold crown");
        assert!(db.items[0].treasure);
        assert_eq!(db.items[0].start_loc, 0);
        assert_eq!(db.items[1].text, "A brass lamp");
        assert_eq!(db.items[1].auto_noun.as_deref(), Some("LAMP"));
        assert_eq!(db.items[1].start_loc, 2);
        assert_eq!(db.items[2].start_loc, CARRIED, "255 means carried");
        assert_eq!(db.items[3].text, MISSING_ITEM_TEXT);
        assert_eq!(db.items[3].start_loc, 1);

        // The dictionary is two contiguous blocks, not alternating pairs, and
        // the `*` synonym convention survives into the reference shape.
        assert_eq!(&db.verbs[..4], ["AUTO", "GO", "*WALK", "TAKE"]);
        assert_eq!(&db.nouns[..3], ["ANY", "NORT", "LAMP"]);
        assert_eq!(db.match_verb("walk"), Some(1), "a synonym resolves to GO");
        assert_eq!(db.match_noun("lamp"), Some(2));

        assert_eq!(db.messages, ["", "A voice booms out.", "Nothing happens.", "The end."]);
    }

    #[test]
    fn the_release_is_identified_by_the_program_files_own_byte_sum() {
        let image = hand_built();
        assert_eq!(image_checksum(&image, LOAD), FIXTURE.checksum);
        assert_eq!(identify(&image, LOAD).map(|r| r.title), Some(FIXTURE.title));
        assert!(looks_like_c64_mysterious(&image, LOAD));

        // A program file is the image with its load address in front, and that
        // is what a host hands over after pulling it off a `.d64`.
        let mut prg = LOAD.to_le_bytes().to_vec();
        prg.extend_from_slice(&image);
        assert!(looks_like_c64_mysterious_prg(&prg));
        assert!(crate::looks_like_scott_bytes(&prg), "the engine sniff answers for it too");
        assert_eq!(prg_image(&prg), Some((&image[..], LOAD)));
        assert_eq!(parse_c64_mysterious_prg(&prg), Ok(parsed()));
        // …and `Database::parse` reaches it through the same one entry point
        // the text format and the TI-99/4A releases go through.
        assert_eq!(Database::parse(&prg), Ok(parsed()));
    }

    #[test]
    fn every_catalogued_release_is_listed_once_and_completely() {
        for r in RELEASES {
            assert_eq!(RELEASES.iter().filter(|o| o.checksum == r.checksum).count(), 1);
            assert!(!r.file_name.is_empty() && !r.title.is_empty());
            assert!(r.verb_cells > 0);
        }
        // §5.3's one action-count repair, and only that one.
        let repaired: Vec<_> =
            RELEASES.iter().filter(|r| r.action_count.is_some()).map(|r| r.title).collect();
        assert_eq!(repaired, ["Escape from Pulsar 7"]);
        assert_eq!(RELEASES[4].action_count, Some(190));
        // §6.5's two compilation disks, six games and five.
        assert_eq!(RELEASES.len(), 11);
    }

    #[test]
    fn the_four_header_shapes_read_the_fields_their_release_puts_where() {
        // §4.5's worked example, byte for byte: Ten Little Indians packs at
        // BYTE granularity, and reading it on a word grid gives lamp 244
        // (the low byte of the right answer) instead of 500.
        let mut f = Fixture::new();
        f.put(HEADER_GUARD_ADDR, &HEADER_GUARD);
        let h = HEADER_ADDR;
        f.put(
            h + 2,
            &[0x49, 0, 0xA1, 0, 0x52, 0, 0x3F, 0, 0x05, 0x3D, 0, 0x04, 0, 0xF4, 0x01, 0x43, 0x00],
        );

        let indians = read_header(&f.mem, LOAD, HeaderShape::TenLittleIndians).unwrap();
        assert_eq!(
            (indians.items, indians.actions, indians.words, indians.rooms),
            (73, 161, 82, 63)
        );
        assert_eq!(indians.max_carry, 5);
        assert_eq!(indians.start_room, 61);
        assert_eq!(indians.treasures, 0);
        assert_eq!(indians.word_length, 4);
        assert_eq!(indians.lamp, 500, "F4 01 read as a word, not as one byte of 244");
        assert_eq!(indians.messages, 67);

        // The Mysterious shape splits word 5 into (max carried, start room)…
        let mut f = Fixture::new();
        f.put(HEADER_GUARD_ADDR, &HEADER_GUARD);
        f.put(h + 2, &[48, 0, 166, 0, 78, 0, 31, 0, 0x06, 0x01, 0, 0, 4, 0, 200, 0, 99, 0]);
        let myst = read_header(&f.mem, LOAD, HeaderShape::Mysterious).unwrap();
        assert_eq!((myst.items, myst.actions, myst.words, myst.rooms), (48, 166, 78, 31));
        assert_eq!((myst.max_carry, myst.start_room), (6, 1));
        assert_eq!((myst.lamp, myst.messages), (200, 99));
        assert_eq!(myst.treasure_room, 0, "§6.2: this shape drops the field");

        // …and Arrow of Death part 2 is that shape with three fields permuted.
        let arrow = read_header(&f.mem, LOAD, HeaderShape::Arrow2).unwrap();
        assert_eq!((arrow.items, arrow.actions, arrow.words), (78, 48, 166));
        assert_eq!((arrow.max_carry, arrow.start_room), (6, 1), "the rest is unchanged");

        // The early shape reads the same bytes as an unpacked word grid, which
        // is why the shape has to be tabulated and cannot be guessed.
        let early = read_header(&f.mem, LOAD, HeaderShape::Early).unwrap();
        assert_eq!((early.max_carry, early.start_room), (0x0106, 0));
    }

    #[test]
    fn header_lengths_put_the_action_table_where_the_dictionary_says_it_is() {
        // §6.2's free cross-check, and §4.6's identification procedure: the
        // action table begins immediately after the header AND at the
        // dictionary minus (action count + 1) x 16, and a wrong field order
        // makes the two arithmetics disagree.
        assert_eq!(HeaderShape::Early.header_len(), 26);
        assert_eq!(HeaderShape::Mysterious.header_len(), 24);
        assert_eq!(HeaderShape::Arrow2.header_len(), 24);
        assert_eq!(HeaderShape::TenLittleIndians.header_len(), 23);
        // §6.2 tabulates where each shape puts the action table on a real
        // release, which is HEADER_ADDR + header_len.
        assert_eq!(HEADER_ADDR + HeaderShape::Early.header_len(), 0x5DF1);
        assert_eq!(HEADER_ADDR + HeaderShape::Mysterious.header_len(), 0x5DEF);
        assert_eq!(HEADER_ADDR + HeaderShape::TenLittleIndians.header_len(), 0x5DEE);

        assert_eq!(find_signature(&hand_built(), LOAD), Some(fixture_dictionary()));
    }

    #[test]
    fn a_leading_nul_where_a_cell_should_begin_is_alignment_padding() {
        // The Golden Baton's final cell, in miniature: an extra NUL in front of
        // a four-letter word. Read as fixed cells with no skip this gives
        // "CAS"; §4.2's first escape gives "CAST", which is what that title's
        // published conversion says.
        let mut f = Fixture::new();
        f.put(0x5000, b"HELM\0OFF\0\0\0CAST\0");
        let (cells, _) = read_cells(&f.mem, LOAD, 0x5000, Some(0x5010), 5, 3).unwrap();
        assert_eq!(cells, ["HELM", "OFF", "CAST"]);
    }

    #[test]
    fn a_cell_of_spaces_stays_a_cell_of_spaces() {
        // Waxworks carries these, and §4.2's space escape mis-aligns the whole
        // table on them — which is why this module does not implement it.
        let mut f = Fixture::new();
        f.put(0x5000, b"AUTO\0     GO\0\0\0*TAKE");
        let (cells, end) = read_cells(&f.mem, LOAD, 0x5000, Some(0x5014), 5, 4).unwrap();
        assert_eq!(cells, ["AUTO", "     ", "GO", "*TAKE"]);
        assert_eq!(end, 0x5014, "four cells of five bytes, no escapes taken");
    }

    #[test]
    fn a_string_the_block_has_no_room_for_is_a_placeholder_not_an_overrun() {
        // §5.3's Time Machine repair. Reading a 63rd string here would run
        // into the item-location table.
        let mut f = Fixture::new();
        f.put(0x5000, b"Generator\0Archway\0");
        let (texts, _) = read_strings(&f.mem, LOAD, 0x5000, Some(0x5012), 3).unwrap();
        assert_eq!(texts, ["Generator", "Archway", MISSING_ITEM_TEXT]);
    }

    #[test]
    fn the_direction_words_come_from_the_head_of_the_system_message_block() {
        let image = hand_built();
        assert_eq!(
            system_direction_words(&image, LOAD).unwrap(),
            ["NORTH", "SOUTH", "EAST", "WEST", "UP", "DOWN"].map(String::from)
        );
    }

    // ── Refusals (§11) ────────────────────────────────────────────────────────

    #[test]
    fn an_uncatalogued_image_is_refused_by_name_rather_than_guessed_at() {
        let mut image = hand_built();
        image[0x50] ^= 0xFF; // changes the checksum, nothing else
        assert_eq!(
            parse_c64_mysterious(&image, LOAD),
            Err(LoadError::UnsupportedDialect(Dialect::C64OrZxSnapshot))
        );
        assert!(!looks_like_c64_mysterious(&image, LOAD));
    }

    #[test]
    fn a_truncated_image_is_a_named_error_and_never_a_panic() {
        let full = hand_built();
        // Cuts that remove structure the loader needs.
        for cut in [0, 1, 2, 3, 0x100, 0x1DD8, 0x2000, 0x3000] {
            let short = &full[..cut];
            assert!(parse_c64_mysterious(short, LOAD).is_err(), "cut at {cut}");
            assert!(decode_family_b_pictures(short, LOAD).is_err(), "cut at {cut}");
            assert!(!looks_like_c64_mysterious(short, LOAD), "cut at {cut}");
        }
        // …and every cut at all, for the property that actually matters: no
        // panic. Trimming trailing zero bytes changes neither the checksum nor
        // any table, so the shortest of these still loads, correctly.
        for cut in 0..full.len() {
            let _ = parse_c64_mysterious(&full[..cut], LOAD);
            let _ = decode_family_b_pictures(&full[..cut], LOAD);
            let _ = looks_like_c64_mysterious(&full[..cut], LOAD);
        }
    }

    #[test]
    fn an_image_with_no_jmp_guard_is_refused() {
        let mut f = Fixture::new();
        f.put(HEADER_GUARD_ADDR, &[0x4C, 0x00, 0x00]);
        f.put(0x5000, DICTIONARY_SIGNATURE);
        f.balance();
        assert_eq!(
            parse_c64_mysterious(&f.mem, LOAD),
            Err(bad("no JMP $4D19 in front of the header at $5DD6"))
        );
    }

    #[test]
    fn a_catalogued_image_with_a_wrong_action_table_is_refused_not_mis_read() {
        // Move the dictionary one record along, so (dictionary − (actions + 1)
        // × 16) no longer lands on the byte after the header.
        let mut image = hand_built();
        let dictionary = usize::from(fixture_dictionary() - LOAD);
        image[dictionary..dictionary + 8].fill(0);
        image[dictionary + 16..dictionary + 24].copy_from_slice(DICTIONARY_SIGNATURE);
        let moved = u16::try_from(dictionary + 16).unwrap() + LOAD;
        let [lo, hi] = moved.to_le_bytes();
        image[usize::from(DICTIONARY_POINTER.0 - LOAD)] = lo;
        image[usize::from(DICTIONARY_POINTER.1 - LOAD)] = hi;
        balance(&mut image);
        assert_eq!(
            parse_c64_mysterious(&image, LOAD),
            Err(bad("the action table does not begin immediately after the header"))
        );
    }

    #[test]
    fn a_dictionary_pointer_that_contradicts_the_signature_is_refused() {
        // §6.2's cross-check, exercised: two independent routes to the
        // dictionary must agree.
        let mut image = hand_built();
        image[usize::from(DICTIONARY_POINTER.0 - LOAD)] ^= 0x10;
        balance(&mut image);
        assert_eq!(
            parse_c64_mysterious(&image, LOAD),
            Err(bad("the driver's dictionary pointer disagrees with the AUTO/GO signature"))
        );
    }

    #[test]
    fn two_hundred_byte_flips_never_panic_and_never_produce_a_wrong_database() {
        // Every flip is re-balanced, so the corruption is not simply hidden by
        // the checksum: this is a catalogued file whose contents are wrong.
        let full = hand_built();
        let mut seed = 0x1234_5678u32;
        let mut flips = 0;
        while flips < 200 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let at = (seed >> 8) as usize % full.len();
            if in_ballast(at) {
                continue; // `balance` would simply overwrite it
            }
            flips += 1;
            let mut image = full.clone();
            image[at] ^= 1u8 << (seed % 8);
            balance(&mut image);
            // No panic, and any database it does produce is structurally sound.
            if let Ok(db) = parse_c64_mysterious(&image, LOAD) {
                assert!(db.start_room < db.rooms.len());
                for room in &db.rooms {
                    assert!(room.exits.iter().all(|&e| e < db.rooms.len()));
                }
                assert_eq!(db.items.len(), usize::from(ITEMS) + 1);
                assert_eq!(db.rooms.len(), usize::from(ROOMS) + 1);
            }
            let _ = decode_family_b_pictures(&image, LOAD);
        }
    }

    // ── Family B pictures (§8.2) ──────────────────────────────────────────────

    #[test]
    fn the_palette_composes_remap_a_with_the_commodore_64_colours() {
        // §8.2's table A is an eight-colour mapping, "which is why its upper
        // half collapses": sources 9, 10, 11 and 15 all land on entry 1, white.
        assert_eq!(PALETTE[0], (0, 0, 0), "0 -> 0, black");
        assert_eq!(PALETTE[1], (95, 72, 233), "1 -> 6, blue");
        assert_eq!(PALETTE[7], (255, 255, 255), "7 -> 1, white");
        assert_eq!(PALETTE[9], PALETTE[7]);
        assert_eq!(PALETTE[10], PALETTE[7]);
        assert_eq!(PALETTE[11], PALETTE[7], "9, 10 and 11 all collapse onto white");
        assert_eq!(PALETTE[13], (167, 167, 167), "13 -> 12, grey");
        assert_eq!(PALETTE[15], (247, 255, 108), "15 -> 7, yellow");
    }

    #[test]
    fn one_image_decodes_to_a_line_and_a_fill_on_a_background() {
        // Background 3, so the line colour is 0 (§8.2: 7 only when the
        // background index is 0). Move to (0, row 190-190 = 0), draw to
        // (40, row 190-130 = 60), then fill from (60, row 1).
        let block = [OP_END, 3, OP_MOVE, 190, 0, 130, 40, OP_FILL, 5, 189, 60, OP_END];
        let pics = decode_family_b_block(&block);
        assert_eq!(pics.len(), 1);
        let p = &pics[0];
        assert_eq!((p.width, p.height), (PICTURE_WIDTH, PICTURE_HEIGHT));
        assert_eq!(p.background, 3);
        assert_eq!(p.line, 0, "any background but 0 draws in colour 0");
        assert_eq!(p.pixels.len(), PICTURE_WIDTH * PICTURE_HEIGHT);
        // Both endpoints inclusive.
        assert_eq!(p.pixels[0], 0);
        assert_eq!(p.pixels[60 * PICTURE_WIDTH + 40], 0);
        // The fill was seeded at (60, 1) and ran over the background.
        assert_eq!(p.pixels[PICTURE_WIDTH + 60], 5);
        assert_eq!(p.rgb(60, 1), Some(PALETTE[5]));
        assert_eq!(p.rgb(PICTURE_WIDTH, 0), None, "off the canvas");
    }

    #[test]
    fn a_fill_stops_at_the_background_test_and_never_repaints() {
        // §8.2: a pixel is filled only if its current value equals the image's
        // BACKGROUND index — so a second fill of the same seed finds nothing.
        let block = [
            OP_END, 0, //
            OP_FILL, 4, 189, 10, // fill outward from (10, 1) with 4
            OP_FILL, 5, 189, 10, // …and this one finds no background left
            OP_END,
        ];
        let pics = decode_family_b_block(&block);
        assert_eq!(pics[0].line, 7, "background 0 draws in colour 7");
        assert!(pics[0].pixels.contains(&4));
        assert!(!pics[0].pixels.contains(&5), "the second fill found no background");
    }

    #[test]
    fn the_bounded_fill_queue_does_not_change_the_output_on_this_canvas() {
        // §8.2 calls the 1024-point queue bound "a real behavioural fork" and
        // asks an implementer to choose deliberately: a fill of a large region
        // "can therefore terminate early and leave holes", and a decoder with
        // an unbounded queue would produce different, more complete pictures.
        // This decoder reproduces the bound. **Measured here, it makes no
        // difference on a Family B canvas**: an unobstructed fill of all
        // 255 x 94 pixels still paints every one of them, because a dropped
        // neighbour is reached again from another direction. The fork is real
        // in the format and unobservable at this canvas size — which is worth
        // pinning, since it is what lets this decoder be called faithful
        // without anyone having to compare two variants of it.
        assert_eq!(FILL_QUEUE_LIMIT, 1024);
        let block = [OP_END, 0, OP_FILL, 4, 189, 10, OP_END];
        let painted = decode_family_b_block(&block)[0].pixels.iter().filter(|&&p| p == 4).count();
        assert_eq!(painted, PICTURE_WIDTH * PICTURE_HEIGHT, "an open canvas fills completely");
    }


    /// A rectangle with a fill inside it, drawn by hand at scale 1 and scale 3
    /// (SQ-1467). Every edge is axis-aligned, so there is no staircase for the
    /// supersample to resolve differently and the two rasters must agree
    /// **exactly**: each native pixel's 3 x 3 block is that pixel, nine times.
    fn rectangle() -> PictureList {
        PictureList {
            background: 3,
            line: 0,
            ops: vec![
                PictureOp::Line { from: (10, 10), to: (20, 10) },
                PictureOp::Line { from: (20, 10), to: (20, 20) },
                PictureOp::Line { from: (20, 20), to: (10, 20) },
                PictureOp::Line { from: (10, 20), to: (10, 10) },
                PictureOp::Fill { seed: (15, 15), colour: 5 },
            ],
        }
    }

    #[test]
    fn a_hand_drawn_rectangle_is_the_pixels_it_should_be() {
        let p = rectangle().rasterise();
        assert_eq!((p.width, p.height, p.scale), (PICTURE_WIDTH, PICTURE_HEIGHT, 1));
        let at = |x: usize, y: usize| p.pixels[y * PICTURE_WIDTH + x];
        assert_eq!(at(10, 10), 0, "top-left corner is the line");
        assert_eq!(at(15, 10), 0, "top edge");
        assert_eq!(at(20, 15), 0, "right edge");
        assert_eq!(at(15, 20), 0, "bottom edge");
        assert_eq!(at(15, 15), 5, "the fill reached the middle");
        assert_eq!(at(11, 11), 5, "…and the corner just inside the border");
        assert_eq!(at(9, 10), 3, "one pixel outside is still the background");
        assert_eq!(at(15, 21), 3, "the fill did not escape below");
        assert_eq!(p.pixels.iter().filter(|&&v| v == 5).count(), 9 * 9, "a 9 x 9 interior");
    }

    #[test]
    fn an_axis_aligned_drawing_supersamples_to_exactly_itself() {
        let list = rectangle();
        let one = list.rasterise();
        let three = list.rasterise_at(3);
        assert_eq!(
            (three.width, three.height, three.scale),
            (PICTURE_WIDTH * 3, PICTURE_HEIGHT * 3, 3)
        );
        // Spot the three regions first, by hand: the top edge runs along
        // device rows 30-32, the interior starts at device (33, 33), and the
        // canvas outside the rectangle is untouched background.
        assert_eq!(three.pixels[31 * three.width + 45], 0, "device (45, 31) is the top edge");
        assert_eq!(three.pixels[45 * three.width + 45], 5, "device (45, 45) is inside the fill");
        assert_eq!(three.pixels[0], 3, "device (0, 0) is outside");
        // Then the whole canvas: with no diagonal anywhere, every native pixel
        // is its own 3 x 3 block, nine identical device pixels.
        for y in 0..PICTURE_HEIGHT {
            for x in 0..PICTURE_WIDTH {
                let want = one.pixels[y * PICTURE_WIDTH + x];
                for dy in 0..3 {
                    for dx in 0..3 {
                        assert_eq!(
                            three.pixels[(y * 3 + dy) * three.width + x * 3 + dx],
                            want,
                            "native ({x}, {y}) block pixel ({dx}, {dy})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_corner_touching_pair_of_lines_stays_sealed_at_every_supersample() {
        // The room's left wall stops one pixel short of, and one pixel left
        // of, the end of its ceiling: (9, 11) and (10, 10) touch only at a
        // CORNER. A 4-connected flood cannot cross a diagonal, so at 1x the
        // fill seeded outside fills the whole canvas around the room and not
        // one pixel of the room itself — and no supersample may open that
        // corner, which is the failure a finer line invites.
        let list = PictureList {
            background: 3,
            line: 0,
            ops: vec![
                PictureOp::Line { from: (10, 10), to: (30, 10) }, // ceiling
                PictureOp::Line { from: (30, 10), to: (30, 30) }, // right wall
                PictureOp::Line { from: (30, 30), to: (9, 30) },  // floor
                PictureOp::Line { from: (9, 30), to: (9, 11) },   // left wall, one short
                PictureOp::Fill { seed: (0, 0), colour: 5 },
            ],
        };
        for scale in 1..=4u32 {
            let p = list.rasterise_at(scale);
            let s = scale as usize;
            let at = |x: usize, y: usize| p.pixels[(y * s) * p.width + x * s];
            assert_eq!(at(0, 0), 5, "{scale}x: the fill covers the outside");
            assert_eq!(at(9, 10), 5, "{scale}x: …including the pocket at the open corner");
            assert_eq!(at(20, 20), 3, "{scale}x: the room's middle is untouched background");
            assert_eq!(at(10, 11), 3, "{scale}x: …and so is the pixel just inside the corner");
            // Not one device pixel of the room's interior, either.
            let leaked = (11..30)
                .flat_map(|y| (10..30).map(move |x| (x, y)))
                .flat_map(|(x, y)| {
                    (0..s).flat_map(move |dy| (0..s).map(move |dx| (x * s + dx, y * s + dy)))
                })
                .filter(|&(dx, dy)| p.pixels[dy * p.width + dx] == 5)
                .count();
            assert_eq!(leaked, 0, "{scale}x: {leaked} device pixels of the room took the fill");
        }
    }

    #[test]
    fn the_display_list_and_the_native_raster_are_the_same_decode() {
        // `decode_family_b_block` is `decode_family_b_lists` plus a rasterise,
        // and nothing about the byte walk may differ between them (SQ-1467).
        let block = [OP_END, 3, OP_MOVE, 190, 0, 130, 40, OP_FILL, 5, 189, 60, OP_END];
        let lists = decode_family_b_lists(&block);
        assert_eq!(lists.len(), 1);
        assert_eq!(
            lists[0].ops,
            vec![
                PictureOp::Line { from: (0, 0), to: (40, 60) },
                PictureOp::Fill { seed: (60, 1), colour: 5 },
            ],
            "the move resolved into the line's start rather than surviving as an op"
        );
        assert_eq!(decode_family_b_block(&block), vec![lists[0].rasterise()]);
    }

    #[test]
    fn a_supersample_outside_the_supported_range_is_clamped_rather_than_refused() {
        let list = rectangle();
        assert_eq!(list.rasterise_at(0), list.rasterise(), "0 is the native canvas");
        assert_eq!(list.rasterise_at(999).scale, MAX_PICTURE_SCALE);
    }

    #[test]
    fn a_point_off_the_canvas_is_discarded_rather_than_wrapped_into_the_picture() {
        // §8.2: "any point resolving to a row of 94 or more, or a negative row,
        // is discarded". A stored vertical of 8 means row 182.
        let block = [OP_END, 3, OP_MOVE, 8, 10, 8, 200, OP_END];
        let p = &decode_family_b_block(&block)[0];
        assert!(p.pixels.iter().all(|&v| v == 3), "nothing landed on the canvas");
    }

    #[test]
    fn a_block_that_does_not_open_with_ff_yields_nothing() {
        assert!(decode_family_b_block(&[]).is_empty());
        assert!(decode_family_b_block(&[0, 1, OP_END]).is_empty());
        // …and one that truncates mid-record yields only the images that
        // decoded whole (§11: "a partial load, not a fatal error").
        let block = [OP_END, 0, OP_END, 1, OP_MOVE, 190];
        assert_eq!(decode_family_b_block(&block).len(), 1);
    }

    #[test]
    fn the_hand_built_images_artwork_decodes_through_the_pointer_block() {
        let image = hand_built();
        let pics = decode_family_b_pictures(&image, LOAD).unwrap();
        assert_eq!(pics.len(), 1);
        assert_eq!(pics[0].background, 1);
        assert_eq!(pics[0].palette, &PALETTE);
    }

    // ── The options the database forces (§9.2, §9.3) ──────────────────────────

    #[test]
    fn a_mysterious_database_forces_the_two_lamp_options_on() {
        // §9.2: "every Mysterious Adventures release and every TI-99/4A
        // release forces both on", and Appendix A says those runtime
        // differences travel with the database rather than with the host.
        let vm = Vm::new_full(parsed(), false, 1, Options::default());
        assert!(vm.options().scott_light);
        assert!(vm.options().prehistoric_lamp);
        // …but NOT second-person wording. §6.4 and §9.3 make that a property of
        // the PLATFORM, not the series: these eleven are first-person, so
        // `you_are` stays the host's, default off.
        assert!(!vm.options().you_are);
        assert_eq!(vm.options().presentation, Presentation::default());
    }

    #[test]
    fn an_ordinary_database_is_left_alone() {
        let mut db = parsed();
        db.mysterious = false;
        let vm = Vm::new_full(db, false, 1, Options::default());
        assert!(!vm.options().scott_light);
        assert!(!vm.options().prehistoric_lamp);
    }
}
