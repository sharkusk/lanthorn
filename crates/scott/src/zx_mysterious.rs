//! Reads the **ZX Spectrum *Mysterious Adventures*** — Brian Howarth's
//! eleven titles from *The Golden Baton* to *Waxworks*, as sold for the 48K
//! Spectrum — into the same [`Database`] every other Scott Adams dialect
//! decodes to, and decodes their line-drawn artwork
//! ([`decode_pictures`]).
//!
//! The input is the plain **48K memory image**: 49,152 bytes whose first byte
//! is address `$4000` ([`crate::decompress_z80`] produces exactly that from a
//! `.z80` snapshot, and [`parse_zx_mysterious_z80`] is the two steps spelled
//! once). Nothing inside the image is compressed and no Z80 is executed to
//! reach the tables.
//!
//! # There is no per-release catalogue here, and none is needed
//!
//! [`crate::c64`], the Commodore 64 half of the same eleven titles, keys two
//! facts off a checksum table: which of `docs/internals/scott-dialects-spec.md`
//! §4.5's header field orders the release uses, and how its dictionary splits
//! into verb and noun cells. **Neither is a table on the Spectrum**, because
//! this driver plants nine table addresses in the nine words that follow the
//! header and gives the verb and the noun block a pointer each:
//!
//! * the **field order** is found by trying §4.5's four candidate orders at
//!   every address in the image and keeping the one whose counts make the
//!   pointer block's spans come out exactly right (`locate`) — measured over
//!   all eleven §10.3 specimens, exactly one (order, address) pair survives
//!   per title, and it is §4.5's **early** order in every one;
//! * the **verb/noun split** is read: the verb block runs from the signature
//!   hit to the noun pointer and each block is exactly (word count + 1) cells
//!   (§4.6 calls the split "genuinely not derivable"; that is true of the
//!   Commodore 64 releases and false of these).
//!
//! So this module identifies **nothing** in order to load: hand it any 48K
//! image and it either finds a self-consistent Mysterious database in it or
//! refuses by name. [`RELEASES`] exists only to put a title on a row in a
//! story list, and a release missing from it still loads and plays.
//!
//! # Provenance
//!
//! Implemented from
//! [`docs/internals/scott-dialects-spec.md`](../../../docs/internals/scott-dialects-spec.md)
//! — §4.1 the dictionary signature, §4.2 dictionary cells (and its ZX
//! prohibition on the alignment escape), §4.3-§4.4 locating and the
//! uncompressed table encodings, §4.5 the header shapes, §4.6 the
//! plausibility scan and the driver pointer-block rule, §5.3 the load-time
//! repairs (none of which fires here — see [`parse_zx_mysterious`]), §6 the
//! Mysterious releases, §7.1 the `.z80` container, §8.2 the Family B picture
//! format, §9.2-§9.3 the runtime facts the database forces, §10.3 the
//! specimens, §11 the refusals — plus the specimens themselves. Both were
//! written under the clean-room protocol in `docs/internals/clean-room.md`:
//! **no GPL interpreter's source was read to write this module.**
//!
//! Where the specification and the specimens disagree the measurement wins,
//! and each such site says so; `docs/internals/scott-dialects-spec.md`
//! Appendix A carries the list.
//!
//! # The layout, in one picture
//!
//! ```text
//!   $4000  driver, screen, system messages
//!   $6349 or $6351  header (12 count words) + nine table pointers
//!   $7B56 or $7B81  actions  →  room connections  →  item locations
//!          →  item locations, second copy  →  verb cells  →  noun cells
//!          →  room descriptions  →  messages  →  item descriptions
//!          →  one zero byte  →  Family B picture data  →  unused RAM
//! ```
//!
//! Every one of those addresses is read out of the pointer block; the two
//! spelled above are what the eleven specimens happen to hold, and nothing
//! here assumes them.

use crate::c64::{decode_family_b_lists_at_most, HeaderShape, Picture, PictureList};
use crate::database::CARRIED;
use crate::loader::{extract_auto_noun, Dialect, LoadError};
use crate::{Action, Condition, Database, Item, Room};

// ── The image ─────────────────────────────────────────────────────────────────

/// The address the first byte of a 48K memory image holds (§7.1: "a byte array
/// of exactly 49,152 bytes whose first byte is address 0x4000").
pub const IMAGE_BASE: u16 = 0x4000;

/// §4.1's plain four-letter dictionary signature, with back-off 0: the match
/// lands **on** the dictionary's first byte. Ten of the eleven ZX releases
/// answer it and the eleventh (the Italian *Perseus*) is not read here (§6.1).
const DICTIONARY_SIGNATURE: &[u8] = b"AUTO\0GO\0";

/// How many little-endian words this module reads at the header address: §4.3's
/// fifteen would stop one word short of the driver's ninth pointer, so the
/// window is the twelve count words plus the nine pointers that follow them.
const HEADER_WINDOW: usize = 21;

/// Bytes per action record: eight little-endian 16-bit words (§4.4).
const ACTION_RECORD: u16 = 16;

/// Exits per room-connection record: north, south, east, west, up, down (§4.4).
const EXITS_PER_ROOM: u16 = 6;

/// The reference format's own "carried" byte, normalised to
/// [`crate::database::CARRIED`] at load (§4.4).
const STORED_CARRIED: u8 = 255;

/// §8.2's end-of-image opcode, which also introduces the next image and so is
/// the byte the whole picture block begins with.
const PICTURE_BLOCK_MARK: u8 = 0xFF;

/// How far past the item descriptions the picture block's leading `$FF` may sit
/// (§8.2's "immediately after" with the run of filler the Commodore 64
/// releases also carry). Measured on all eleven §10.3 specimens: **exactly one
/// zero byte**, every time; the bound exists so a damaged image cannot make the
/// search run away.
const PICTURE_BLOCK_SEARCH: usize = 64;

// ── The pointer block ─────────────────────────────────────────────────────────
//
// §4.6: "look for a driver pointer block before you tabulate anything … where
// one interpreter binary was shipped with several different data payloads, its
// initialisation code plants the table addresses into fixed locations from
// literal operands". These eleven are such a series, and their block is not in
// the initialisation code at all — it is nine words in the header itself,
// immediately after the twelve counts, which is why §4.3's "fifteen
// consecutive little-endian words" reads three of them as header fields.
//
// Found the way §4.6 says to find one: two releases believed to share a driver
// compared byte for byte (`m1goldba` and `m5pulsar` are identical up to
// `$5120`), and then the bytes around the header — which the §4.6 plausibility
// scan had already located — read as addresses and checked against the tables
// two other routes already give (the dictionary from its §4.1 signature, and
// the end of the last table from the start of the picture data). Every slot's
// span then matches a header count exactly, in all eleven.

/// Slot 0 — the byte after the item descriptions, and so where the picture
/// block's leading `$FF` is searched for.
///
/// §4.5 calls word 0 of the early shape "not a field … unused"; on the
/// Commodore 64 it is a `JMP` operand (§6.2). **On the Spectrum it is the tenth
/// table pointer**, which is the one place this module's reading of the early
/// header differs from §4.5's.
const SLOT_PICTURES: usize = 0;
/// Slot 12 — the room descriptions. Equal to [`SLOT_ROOMS_AGAIN`] in all
/// eleven, which is a free self-consistency check.
const SLOT_ROOMS: usize = 12;
/// Slot 13 — the room-connection table, (room count + 1) six-byte records.
const SLOT_CONNECTIONS: usize = 13;
/// Slot 14 — the item-location table, (item count + 1) single bytes.
const SLOT_LOCATIONS: usize = 14;
/// Slot 15 — a **second** item-location table of the same size, which the
/// driver plays out of while slot 14 keeps the starting positions.
///
/// Byte-identical to slot 14's in all eleven specimens, as it must be in a
/// snapshot taken before the first move; this module reads slot 14, so a
/// snapshot taken mid-game would still start the player where the file's
/// author put them.
const SLOT_LOCATIONS_COPY: usize = 15;
/// Slot 16 — the verb cells, and the address §4.1's signature lands on.
const SLOT_VERBS: usize = 16;
/// Slot 17 — the noun cells. §4.2 says a memory image holds "every verb cell,
/// then every noun cell"; this driver points at both, so the split needs no
/// catalogue.
const SLOT_NOUNS: usize = 17;
/// Slot 18 — the room descriptions again (see [`SLOT_ROOMS`]).
const SLOT_ROOMS_AGAIN: usize = 18;
/// Slot 19 — the message pool, (message count + 1) NUL-terminated strings.
const SLOT_MESSAGES: usize = 19;
/// Slot 20 — the item descriptions, (item count + 1) NUL-terminated strings.
const SLOT_ITEMS: usize = 20;

// ── §4.3's validation ranges ──────────────────────────────────────────────────

/// §4.6's plausibility window for the early shape, as the scan applies it:
/// items 10-500, actions 100-500, words 50-190, rooms 10-100, max carried
/// 1-20, start room between 1 and the room count, word length 3-5 and message
/// count below 200.
///
/// §4.6 measured this over two of these specimens and found "exactly one
/// candidate" in each; measured here over all eleven, and over the nine other
/// snapshots in the same archive, the count is one and zero respectively — see
/// `crates/scott/tests/zx_specimens.rs`.
fn plausible(h: &ZxHeader) -> bool {
    (10..=500).contains(&h.items)
        && (100..=500).contains(&h.actions)
        && (50..=190).contains(&h.words)
        && (10..=100).contains(&h.rooms)
        && (1..=20).contains(&h.max_carry)
        && (1..=h.rooms).contains(&h.start_room)
        && (3..=5).contains(&h.word_length)
        && h.messages < 200
}

// ── The per-title table, for TITLES only ──────────────────────────────────────

/// One of the eleven titles, keyed by the seven header counts §6.1 calls "the
/// only route available for a reference-format database".
///
/// **Nothing in this table is needed to load a game**, which is the whole
/// difference between this module and [`crate::c64`]: the loader reads the
/// counts, the field order and the verb/noun split out of the bytes, and a
/// twelfth release nobody catalogued loads and plays exactly the same. The
/// table exists so a story list can say "The Golden Baton" instead of
/// `m1goldba`, and [`identify`] is the only thing that reads it.
///
/// **How the column was derived**, per §4.6's honesty requirement: the counts
/// are what `locate` reads out of each §10.3 snapshot, cross-checked against
/// the eleven published reference-format conversions (`mysterious-dat`, §10.3),
/// which agree exactly on five titles and differ on the rest — those are
/// different releases of the same games, and the *snapshot's* numbers are the
/// ones here. The titles are the series' own spelling, as [`crate::c64`]
/// spells them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZxRelease {
    /// The game, spelled as the series spells it.
    pub title: &'static str,
    /// The IF Archive filename this release is distributed under (§10.3),
    /// which is what a player sees before this table renames the row.
    pub file_name: &'static str,
    /// Item count (header word 1).
    pub items: u16,
    /// Action count (word 2).
    pub actions: u16,
    /// Word count (word 3).
    pub words: u16,
    /// Room count (word 4).
    pub rooms: u16,
    /// Maximum carried (word 5).
    pub max_carry: u16,
    /// Word length (word 8).
    pub word_length: u16,
    /// Message count (word 10).
    pub messages: u16,
}

/// The eleven ZX Spectrum *Mysterious Adventures* releases of §10.3.
///
/// See [`ZxRelease`] for what this is for — a display title, and nothing else.
/// The seven counts are unique across the eleven (and so are the first four);
/// `zx_specimens.rs` pins that.
pub const RELEASES: [ZxRelease; 11] = [
    ZxRelease {
        title: "The Golden Baton",
        file_name: "m1goldba.z80",
        items: 48,
        actions: 171,
        words: 76,
        rooms: 31,
        max_carry: 5,
        word_length: 4,
        messages: 99,
    },
    ZxRelease {
        title: "The Time Machine",
        file_name: "m2tmachi.z80",
        items: 62,
        actions: 164,
        words: 87,
        rooms: 44,
        max_carry: 6,
        word_length: 4,
        messages: 73,
    },
    ZxRelease {
        title: "Arrow of Death part 1",
        file_name: "m3arrow1.z80",
        items: 64,
        actions: 150,
        words: 90,
        rooms: 52,
        max_carry: 5,
        word_length: 4,
        messages: 82,
    },
    ZxRelease {
        title: "Arrow of Death part 2",
        file_name: "m4arrow2.z80",
        items: 91,
        actions: 190,
        words: 83,
        rooms: 65,
        max_carry: 9,
        word_length: 4,
        messages: 87,
    },
    ZxRelease {
        title: "Escape from Pulsar 7",
        file_name: "m5pulsar.z80",
        items: 90,
        actions: 220,
        words: 145,
        rooms: 45,
        max_carry: 6,
        word_length: 4,
        messages: 75,
    },
    ZxRelease {
        title: "Circus",
        file_name: "m6circus.z80",
        items: 65,
        actions: 165,
        words: 97,
        rooms: 36,
        max_carry: 6,
        word_length: 4,
        messages: 72,
    },
    ZxRelease {
        title: "Feasibility Experiment",
        file_name: "m7feasib.z80",
        items: 65,
        actions: 164,
        words: 82,
        rooms: 59,
        max_carry: 5,
        word_length: 4,
        messages: 65,
    },
    ZxRelease {
        title: "The Wizard of Akyrz",
        file_name: "m8akyrtz.z80",
        items: 49,
        actions: 201,
        words: 85,
        rooms: 40,
        max_carry: 6,
        word_length: 4,
        messages: 99,
    },
    ZxRelease {
        title: "Perseus and Andromeda",
        file_name: "m9perseu.z80",
        items: 60,
        actions: 178,
        words: 130,
        rooms: 40,
        max_carry: 6,
        word_length: 4,
        messages: 96,
    },
    ZxRelease {
        title: "Ten Little Indians",
        file_name: "m10india.z80",
        items: 73,
        actions: 161,
        words: 85,
        rooms: 63,
        max_carry: 5,
        word_length: 4,
        messages: 67,
    },
    ZxRelease {
        title: "Waxworks",
        file_name: "m11waxwo.z80",
        items: 57,
        actions: 189,
        words: 106,
        rooms: 41,
        max_carry: 6,
        word_length: 4,
        messages: 91,
    },
];

// ── What `locate` answers ─────────────────────────────────────────────────────

/// The header §4.5's field orders assign, whichever order assigned it, plus
/// where it was found and which order that was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZxHeader {
    /// The address the twelve count words begin at — `$6349` in two of the
    /// eleven and `$6351` in the other nine, found by the §4.6 scan and never
    /// assumed.
    pub addr: u16,
    /// Which of §4.5's field orders read these counts. [`HeaderShape::Early`]
    /// in all eleven specimens; the other three are tried and rejected.
    pub shape: HeaderShape,
    /// Highest item index, so the item tables hold one more record than this.
    pub items: u16,
    /// Highest action index.
    pub actions: u16,
    /// Highest vocabulary index; each of the two dictionary blocks holds one
    /// more cell than this.
    pub words: u16,
    /// Highest room index.
    pub rooms: u16,
    /// How many items the player may carry.
    pub max_carry: u16,
    /// Where the player starts.
    pub start_room: u16,
    /// How many treasures the game counts.
    pub treasures: u16,
    /// Characters of a dictionary word; a cell is one byte wider.
    pub word_length: u16,
    /// Lamp fuel in turns, as stored (`$FFFF` means "never runs out" and is
    /// sign-extended at the end of [`parse_zx_mysterious`]).
    pub lamp: u16,
    /// Highest message index.
    pub messages: u16,
    /// Where treasures must be deposited to score.
    pub treasure_room: u16,
}

/// The ten table addresses the driver's pointer block holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZxTables {
    /// The action table — **not** a pointer-block slot: it is
    /// (`connections` − (action count + 1) × 16), which is the §4.6 arithmetic
    /// that makes the stored action count checkable. See
    /// [`parse_zx_mysterious`] for the second check that pins it.
    pub actions: u16,
    /// Room connections (slot 13).
    pub connections: u16,
    /// Item locations (slot 14).
    pub locations: u16,
    /// The driver's working copy of the item locations (slot 15).
    pub locations_copy: u16,
    /// Verb cells (slot 16), which is also the §4.1 signature hit.
    pub verbs: u16,
    /// Noun cells (slot 17).
    pub nouns: u16,
    /// Room descriptions (slots 12 and 18, which agree).
    pub rooms: u16,
    /// Messages (slot 19).
    pub messages: u16,
    /// Item descriptions (slot 20).
    pub items: u16,
    /// One past the item descriptions (slot 0) — where the Family B picture
    /// block's leading `$FF` is searched for.
    pub pictures: u16,
}

/// Everything [`locate`] recovers from an image, which is everything
/// [`parse_zx_mysterious`] needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZxLayout {
    /// The counts, and where and how they were read.
    pub header: ZxHeader,
    /// Where every table is.
    pub tables: ZxTables,
}

// ── Address arithmetic ────────────────────────────────────────────────────────

/// A refusal for a recognised release whose bytes did not check out — §11 asks
/// an implementer to name these rather than guess past them.
fn bad(what: &'static str) -> LoadError {
    LoadError::BadDialectData(Dialect::C64OrZxSnapshot, what)
}

/// File offset of memory address `addr`, or `None` when it is below
/// [`IMAGE_BASE`] or past the end of the image.
fn offset_of(image: &[u8], addr: u16) -> Option<usize> {
    let off = usize::from(addr.checked_sub(IMAGE_BASE)?);
    (off <= image.len()).then_some(off)
}

/// `len` bytes at memory address `addr`, or `None` when they do not all lie
/// within the image.
fn read_at(image: &[u8], addr: u16, len: usize) -> Option<&[u8]> {
    let off = offset_of(image, addr)?;
    image.get(off..off.checked_add(len)?)
}

/// One byte at memory address `addr`.
fn byte_at(image: &[u8], addr: u16) -> Option<u8> {
    read_at(image, addr, 1).map(|s| s[0])
}

/// The little-endian word at memory address `addr`.
fn word_at(image: &[u8], addr: u16) -> Option<u16> {
    read_at(image, addr, 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

/// The address §4.1's signature lands on, which must be **unique** in the
/// image.
///
/// §4.1's search is unanchored and takes the first hit; requiring uniqueness is
/// stricter and is what the specimens support — measured on all twenty §10.3
/// snapshots, the plain signature occurs exactly once in each of the eleven
/// Mysterious images and once in each of the two family-A ones, never twice.
/// §6.2 calls the same property "a free and complete cross-check" on the
/// Commodore 64 side.
fn find_signature(image: &[u8]) -> Result<u16, LoadError> {
    let mut found = None;
    for (at, window) in image.windows(DICTIONARY_SIGNATURE.len()).enumerate() {
        if window != DICTIONARY_SIGNATURE {
            continue;
        }
        if found.is_some() {
            return Err(bad("more than one AUTO/GO dictionary signature in the image"));
        }
        found = Some(at);
    }
    let at = found.ok_or(LoadError::UnsupportedDialect(Dialect::C64OrZxSnapshot))?;
    u16::try_from(usize::from(IMAGE_BASE) + at)
        .map_err(|_| bad("the dictionary signature lies outside the 48K address space"))
}

// ── The header ────────────────────────────────────────────────────────────────

/// Read the twenty-one-word window `raw`, which begins at `addr`, under
/// `shape`'s field order.
///
/// Takes the window rather than the image so the scan pays one bounds-checked
/// read per address instead of one per (address, order) pair.
///
/// The four orders are §4.5's, as [`HeaderShape`] spells them for the
/// Commodore 64 releases of the same eleven titles; trying all four and keeping
/// the one that lands is how this module answers §4.6's "header field order
/// (one of eleven): **No**".
fn read_header(raw: &[u8], addr: u16, shape: HeaderShape) -> ZxHeader {
    let w = |i: usize| u16::from_le_bytes([raw[i * 2], raw[i * 2 + 1]]);
    let b = |i: usize| u16::from(raw[i]);
    match shape {
        HeaderShape::Early => ZxHeader {
            addr,
            shape,
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
        HeaderShape::Mysterious | HeaderShape::Arrow2 => ZxHeader {
            addr,
            shape,
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
            treasure_room: 0,
        },
        HeaderShape::TenLittleIndians => ZxHeader {
            addr,
            shape,
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
    }
}

/// (message count + 1) NUL-terminated strings from `addr`: how many bytes they
/// occupy, or `None` when the block runs off the image.
///
/// The address one past the last NUL, which §4.4's back-to-back string blocks
/// make the next table's address. Used by `tables_land` as the check that a
/// candidate header's counts are the right ones: room descriptions must end
/// exactly where the message pointer says, messages exactly at the item
/// pointer, item descriptions exactly at the picture pointer.
fn strings_end(image: &[u8], addr: u16, count: usize) -> Option<u16> {
    let mut at = addr;
    for _ in 0..count {
        loop {
            let b = byte_at(image, at)?;
            at = at.checked_add(1)?;
            if b == 0 {
                break;
            }
        }
    }
    Some(at)
}

/// Do the nine pointers that follow a candidate header describe tables whose
/// every span matches its counts?
///
/// This is §4.6's own recipe for confirming a pointer block — "check the
/// candidate addresses against tables you can already locate by other means …
/// and confirm that reading forward from each candidate with §4.4's encodings
/// makes every table end exactly where the next begins" — and it is what makes
/// the field order and the table reading order derivable rather than
/// tabulated. Eight independent conditions:
///
/// 1. the verb pointer **is** the §4.1 signature hit;
/// 2. the two room-description pointers agree;
/// 3. the room connections span (room count + 1) × 6 bytes;
/// 4. the item locations span (item count + 1) bytes;
/// 5. so does the driver's second copy of them;
/// 6. each dictionary block spans (word count + 1) × (word length + 1) bytes;
/// 7. the action table, taken as the connections less (action count + 1)
///    sixteen-byte records, starts after the header's own pointer block;
/// 8. the three string blocks each end exactly on the next table's pointer.
///
/// A wrong field order gives wrong counts, and the counts appear in six of the
/// eight. Measured: over the whole 48K image, at every address, under all four
/// of §4.5's candidate orders, exactly one (order, address) pair satisfies all
/// eight in each of the eleven specimens — and none at all in the nine other
/// snapshots of the same archive.
fn tables_land(image: &[u8], h: &ZxHeader, dictionary: u16) -> Option<ZxTables> {
    let slot = |i: usize| word_at(image, h.addr.checked_add(u16::try_from(i * 2).ok()?)?);
    let verbs = slot(SLOT_VERBS)?;
    if verbs != dictionary {
        return None;
    }
    let rooms = slot(SLOT_ROOMS)?;
    if rooms != slot(SLOT_ROOMS_AGAIN)? {
        return None;
    }
    let connections = slot(SLOT_CONNECTIONS)?;
    let locations = slot(SLOT_LOCATIONS)?;
    let locations_copy = slot(SLOT_LOCATIONS_COPY)?;
    let nouns = slot(SLOT_NOUNS)?;
    let messages = slot(SLOT_MESSAGES)?;
    let items = slot(SLOT_ITEMS)?;
    let pictures = slot(SLOT_PICTURES)?;

    let rooms_plus_one = h.rooms.checked_add(1)?;
    let items_plus_one = h.items.checked_add(1)?;
    let cells = h.words.checked_add(1)?.checked_mul(h.word_length.checked_add(1)?)?;
    if locations.checked_sub(connections)? != rooms_plus_one.checked_mul(EXITS_PER_ROOM)? {
        return None;
    }
    if locations_copy.checked_sub(locations)? != items_plus_one {
        return None;
    }
    if verbs.checked_sub(locations_copy)? < items_plus_one {
        return None;
    }
    if nouns.checked_sub(verbs)? != cells || rooms.checked_sub(nouns)? != cells {
        return None;
    }

    let span = h.actions.checked_add(1)?.checked_mul(ACTION_RECORD)?;
    let actions = connections.checked_sub(span)?;
    // The header's own window ends the pointer block; the action table must
    // begin past it, or the "header" is somewhere inside the action table.
    if actions < h.addr.checked_add(u16::try_from(HEADER_WINDOW * 2).ok()?)? {
        return None;
    }

    if strings_end(image, rooms, usize::from(rooms_plus_one))? != messages {
        return None;
    }
    if strings_end(image, messages, usize::from(h.messages.checked_add(1)?))? != items {
        return None;
    }
    if strings_end(image, items, usize::from(items_plus_one))? != pictures {
        return None;
    }

    Some(ZxTables {
        actions,
        connections,
        locations,
        locations_copy,
        verbs,
        nouns,
        rooms,
        messages,
        items,
        pictures,
    })
}

/// Find the one header in `image48k`, with no catalogue: §4.1's signature, then
/// §4.6's plausibility scan under each of §4.5's four candidate field orders,
/// then `tables_land`'s eight checks on the driver's pointer block.
///
/// # Errors
///
/// * [`LoadError::UnsupportedDialect`] with [`Dialect::C64OrZxSnapshot`] when
///   the image carries no plain dictionary signature, or carries one but no
///   candidate header survives — which is what a family-A ZX release (§8.1)
///   and a compressed-action one (§5.1) each do, and §11's "a dictionary
///   signature that matches but for which no catalogued release validates is
///   the normal outcome for an unknown release of a known game".
/// * [`LoadError::BadDialectData`] when the signature is not unique, or when
///   **two** candidates survive — an ambiguity this module refuses rather than
///   resolves, because nothing in the format breaks the tie.
pub fn locate(image48k: &[u8]) -> Result<ZxLayout, LoadError> {
    const SHAPES: [HeaderShape; 4] = [
        HeaderShape::Early,
        HeaderShape::Mysterious,
        HeaderShape::Arrow2,
        HeaderShape::TenLittleIndians,
    ];
    let dictionary = find_signature(image48k)?;
    let mut found: Option<ZxLayout> = None;
    for (off, raw) in image48k.windows(HEADER_WINDOW * 2).enumerate() {
        let Ok(addr) = u16::try_from(usize::from(IMAGE_BASE) + off) else { break };
        // Slot 0 is a table address, so a window whose first word cannot be
        // one is not a header under ANY of the four field orders. Cheap, and
        // it is `tables_land`'s own requirement stated two bytes early.
        if u16::from_le_bytes([raw[0], raw[1]]) < IMAGE_BASE {
            continue;
        }
        for shape in SHAPES {
            let header = read_header(raw, addr, shape);
            if !plausible(&header) {
                continue;
            }
            let Some(tables) = tables_land(image48k, &header, dictionary) else { continue };
            let candidate = ZxLayout { header, tables };
            match found {
                Some(prior) if prior != candidate => {
                    return Err(bad(
                        "two different headers fit the image, and nothing breaks the tie",
                    ))
                }
                Some(_) => {}
                None => found = Some(candidate),
            }
        }
    }
    found.ok_or(LoadError::UnsupportedDialect(Dialect::C64OrZxSnapshot))
}

// ── Identification, for a display title only ──────────────────────────────────

/// Which of the eleven titles this image is, by §6.1's header counts.
///
/// `None` for an uncatalogued release, which loads and plays exactly the same —
/// see [`ZxRelease`]. Errors are swallowed on purpose: a caller asking "what is
/// this called" is not asking whether it loads.
pub fn identify(image48k: &[u8]) -> Option<&'static ZxRelease> {
    let h = locate(image48k).ok()?.header;
    RELEASES.iter().find(|r| {
        r.items == h.items
            && r.actions == h.actions
            && r.words == h.words
            && r.rooms == h.rooms
            && r.max_carry == h.max_carry
            && r.word_length == h.word_length
            && r.messages == h.messages
    })
}

/// [`identify`] over a `.z80` snapshot, container step included.
pub fn identify_z80(file: &[u8]) -> Option<&'static ZxRelease> {
    let image = crate::z80::decompress_z80(file).ok()?;
    identify(&image)
}

/// Is this 48K memory image one this module reads?
///
/// One question: does [`locate`] find a header? There is no checksum and no
/// catalogue to consult — see the module docs.
#[must_use]
pub fn looks_like_zx_mysterious(image48k: &[u8]) -> bool {
    locate(image48k).is_ok()
}

/// Is this `.z80` **snapshot** one this module reads?
/// [`looks_like_zx_mysterious`] over [`crate::decompress_z80`].
#[must_use]
pub fn looks_like_zx_mysterious_z80(file: &[u8]) -> bool {
    crate::z80::looks_like_z80(file)
        && crate::z80::decompress_z80(file).is_ok_and(|image| looks_like_zx_mysterious(&image))
}

// ── Table encodings (§4.4) ────────────────────────────────────────────────────

/// Read `count` dictionary cells from `addr`: a fixed grid of
/// (word length + 1)-byte cells, each carrying its word left-aligned and
/// NUL-padded, with a `*` in the cell's **first byte** marking a synonym of the
/// nearest preceding canonical word and occupying one of the cell's own bytes
/// (§4.2).
///
/// **§4.2's leading-NUL alignment escape is deliberately NOT applied here**,
/// on §4.2's own instruction: it is safe only where an empty cell is spelled
/// with spaces, and "on the ZX Spectrum releases of the same titles the
/// opposite holds — all-NUL cells are common … and the escape must not be
/// applied there, because it would eat the first byte of every one of them".
/// Measured on the eleven §10.3 specimens: up to **47** all-NUL cells in one
/// dictionary (§4.2 says 27, measured on fewer), and with the escape off every
/// verb and noun block ends exactly on the pointer that follows it.
///
/// Reading also does not stop at a byte above 127 (§4.2's terminator
/// condition): the block's length is known from the header, and no cell in any
/// of the eleven holds such a byte.
fn read_cells(image: &[u8], addr: u16, width: usize, count: usize) -> Option<Vec<String>> {
    let raw = read_at(image, addr, width.checked_mul(count)?)?;
    Some(
        raw.chunks_exact(width)
            .map(|cell| {
                let (synonym, body) = match cell.split_first() {
                    Some((b'*', rest)) => (true, rest),
                    _ => (false, cell),
                };
                let text: String = body
                    .iter()
                    .take_while(|&&b| b != 0)
                    .map(|&b| if b < 0x80 { b as char } else { '?' })
                    .collect();
                if synonym {
                    format!("*{text}")
                } else {
                    text
                }
            })
            .collect(),
    )
}

/// Read `count` NUL-terminated byte strings from `addr` (§4.4).
///
/// A byte above 127 ends the string's **text** — §4.4's "a byte above 127
/// aborts the read" — while the scan continues to the NUL so the records after
/// it stay aligned. None of the eleven exercises it: every room description,
/// message and item name in the corpus is plain ASCII.
fn read_strings(image: &[u8], addr: u16, count: usize) -> Option<Vec<String>> {
    let mut out = Vec::with_capacity(count);
    let mut at = addr;
    for _ in 0..count {
        let mut text = String::new();
        let mut aborted = false;
        loop {
            let b = byte_at(image, at)?;
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
    Some(out)
}

// ── The loader ────────────────────────────────────────────────────────────────

/// Read a ZX Spectrum *Mysterious Adventures* 48K memory image into a
/// [`Database`].
///
/// `image48k` is [`crate::decompress_z80`]'s output: 49,152 bytes based at
/// `$4000`. [`parse_zx_mysterious_z80`] is the same thing over a `.z80` file.
///
/// # What this forces, and what it does not
///
/// §9.2's two lamp options travel with every Mysterious database
/// (`Database::mysterious`), and §9.3's **second person** travels with this
/// one: "an interpreter should expose the choice as an option and force it on
/// for a recognised Mysterious Adventures **ZX Spectrum** release … the wording
/// follows the platform, not the series". So `Database::second_person` is set
/// here and left clear by [`crate::c64::parse_c64_mysterious`], whose eleven
/// files carry a first-person block of their own (§6.4).
///
/// §6.4's other two ZX facts — the item separator `" - "` and a newline as the
/// message separator, neither of which is in the file — are presentation
/// rather than database, and [`crate::Presentation`] is the host's choice; this
/// loader does not touch it.
///
/// # Which §5.3 repairs fire: none
///
/// Every load-time repair §5.3 lists for this series is a Commodore 64 fact,
/// and this loader would *detect* rather than need each one:
///
/// * the **direction-noun repair** is not needed on either platform — noun
///   cells 0-6 read `ANY NORT SOUT EAST WEST UP DOWN` straight out of the grid
///   in all eleven ZX images, exactly as §5.3 measured on the Commodore 64
///   ones, and overwriting them would destroy stored data;
/// * *Escape from Pulsar 7*'s **wrong action count** (195 for 190) is a
///   Commodore 64 defect; the ZX release's stored 220 passes both arithmetics
///   below;
/// * *The Time Machine*'s **missing 63rd item description** is likewise
///   Commodore 64 only: the ZX release's item block holds all
///   (item count + 1) strings and ends exactly on the picture pointer, which
///   is one of `tables_land`'s eight checks.
///
/// # Errors
///
/// * [`LoadError::UnsupportedDialect`] with [`Dialect::C64OrZxSnapshot`] when
///   [`locate`] finds no header — a ZX snapshot of some other Scott Adams
///   game, refused by name so a host can say what it is.
/// * [`LoadError::BadDialectData`] when a header was found but the tables it
///   implies do not check out: an action record whose vocabulary word is
///   impossible, a room connection pointing outside the room table, a start
///   room indexing nothing.
pub fn parse_zx_mysterious(image48k: &[u8]) -> Result<Database, LoadError> {
    let ZxLayout { header, tables } = locate(image48k)?;
    let rooms_plus_one = usize::from(header.rooms) + 1;
    let items_plus_one = usize::from(header.items) + 1;

    // Actions. `tables.actions` is the §4.6 arithmetic — the connections less
    // (action count + 1) records — so the table lands on the next one by
    // construction whatever the count says. What pins the COUNT is the pair of
    // checks below, and they catch an error in either direction: every record
    // this count claims must carry a possible vocabulary word (verb × 150 +
    // noun, so below 150 × 150), and the sixteen bytes immediately BEFORE the
    // table must not, or the count is one too small and the table really
    // starts there. Measured on all eleven: every record passes and the
    // preceding sixteen bytes read as a vocabulary word of 31,563 or 31,606,
    // which is the driver's code.
    const VOCAB_LIMIT: u16 = 150 * 150;
    let span = usize::from(header.actions + 1) * usize::from(ACTION_RECORD);
    let raw = read_at(image48k, tables.actions, span)
        .ok_or_else(|| bad("the action table runs past the end of the image"))?;
    let mut action_table = Vec::with_capacity(usize::from(header.actions) + 1);
    for record in raw.chunks_exact(usize::from(ACTION_RECORD)) {
        let w = |i: usize| u16::from_le_bytes([record[i * 2], record[i * 2 + 1]]);
        if w(0) >= VOCAB_LIMIT {
            return Err(bad("an action record's vocabulary word is impossible"));
        }
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
    let before = tables
        .actions
        .checked_sub(ACTION_RECORD)
        .and_then(|a| word_at(image48k, a))
        .ok_or_else(|| bad("the action table starts at the very beginning of the image"))?;
    if before < VOCAB_LIMIT {
        return Err(bad("one more action record fits in front of the table than the count allows"));
    }

    // Dictionary: two contiguous blocks of (word count + 1) cells, each with a
    // pointer of its own (§4.2's "every verb cell, then every noun cell", and
    // see the module docs on why the split needs no catalogue here).
    let width = usize::from(header.word_length) + 1;
    let cells = usize::from(header.words) + 1;
    let verbs = read_cells(image48k, tables.verbs, width, cells)
        .ok_or_else(|| bad("the verb cells run past the end of the image"))?;
    let nouns = read_cells(image48k, tables.nouns, width, cells)
        .ok_or_else(|| bad("the noun cells run past the end of the image"))?;

    // Room descriptions and connections.
    let descs = read_strings(image48k, tables.rooms, rooms_plus_one)
        .ok_or_else(|| bad("the room descriptions run past the end of the image"))?;
    let exits = read_at(image48k, tables.connections, rooms_plus_one * usize::from(EXITS_PER_ROOM))
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

    let messages = read_strings(image48k, tables.messages, usize::from(header.messages) + 1)
        .ok_or_else(|| bad("the messages run past the end of the image"))?;

    // Items: descriptions, then the separate one-byte location table. The
    // driver's second copy of the locations (slot 15) is deliberately not the
    // one read — see `SLOT_LOCATIONS_COPY`.
    let texts = read_strings(image48k, tables.items, items_plus_one)
        .ok_or_else(|| bad("the item descriptions run past the end of the image"))?;
    let locations = read_at(image48k, tables.locations, items_plus_one)
        .ok_or_else(|| bad("the item locations run past the end of the image"))?;
    let mut item_table = Vec::with_capacity(items_plus_one);
    for (mut text, &loc) in texts.into_iter().zip(locations) {
        let treasure = text.starts_with('*');
        // §2.5's rule, unchanged (§4.4). These releases spell an item's noun
        // as the dictionary cell it names, `*` and all, where the published
        // conversions simply dropped the marker: a `/*BUSH/` auto-noun is
        // preserved verbatim and matches no typed word, so the item behaves
        // exactly as the conversion's markerless one does.
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
        // an unsigned word spells −1. None of the eleven ZX releases uses it;
        // *Arrow of Death part 1*'s 32,766 is the largest, and stays itself.
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
        // §9.2: every Mysterious Adventures release forces both lamp options.
        mysterious: true,
        // §9.3: and a ZX Spectrum one forces second-person wording.
        second_person: true,
        ti99: None,
        saga_us: None,
    })
}

/// Read a ZX Spectrum *Mysterious Adventures* `.z80` **snapshot**:
/// [`crate::decompress_z80`] and then [`parse_zx_mysterious`].
///
/// This is the entry point a host holding a file's bytes wants, and it is what
/// [`Database::parse`](crate::Database::parse) reaches for; §7.1's "decompress
/// first" rule is the whole reason it exists, because a snapshot's RLE passes
/// literal text through and the raw file contains the dictionary signature "at
/// offsets that mean nothing".
///
/// # Errors
///
/// [`LoadError::BadDialectData`] naming the container failure when the
/// snapshot itself does not decompress (a 128K machine, an unknown hardware
/// mode, a truncated page), and otherwise [`parse_zx_mysterious`]'s own
/// refusals.
pub fn parse_zx_mysterious_z80(file: &[u8]) -> Result<Database, LoadError> {
    let image = crate::z80::decompress_z80(file).map_err(|e| match e {
        crate::z80::Z80Error::UnsupportedHardwareMode(_) => {
            bad("the .z80 snapshot is not a 48K machine")
        }
        crate::z80::Z80Error::UnknownHeaderLength(_) => {
            bad("the .z80 snapshot claims an unrecognised container version")
        }
        _ => bad("the .z80 snapshot does not decompress to a 48K image"),
    })?;
    parse_zx_mysterious(&image)
}

// ── Family B pictures (§8.2) ──────────────────────────────────────────────────

/// The **optimised Sinclair palette**, which §8.2 says every ZX release
/// selects, with the identity remap it also specifies — so `PALETTE[i]` is the
/// RGB a stored index *i* resolves to, and unlike
/// [`crate::c64::PALETTE`] there is no remap table
/// composed into it.
///
/// §8.1 tabulates it: indices 0-7 normal, 8-15 bright. The "measured Sinclair
/// palette" §8.1 also gives is selected only by the German *Gremlins* release,
/// which is not this series.
pub const PALETTE: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (0, 0, 202),
    (202, 0, 0),
    (202, 0, 202),
    (0, 202, 0),
    (0, 202, 202),
    (202, 202, 0),
    (202, 202, 202),
    (0, 0, 0),
    (0, 0, 255),
    (255, 0, 20),
    (255, 0, 255),
    (0, 255, 0),
    (0, 255, 255),
    (255, 255, 0),
    (255, 255, 255),
];

/// Where the Family B picture block starts: the first `$FF` at or after the
/// pointer block's slot 0, which is one past the item descriptions.
///
/// §8.2: "the block begins with a byte of 0xFF; each image is a
/// background-colour byte followed by an opcode stream terminated by 0xFF, and
/// that terminator simultaneously introduces the next image". Measured on all
/// eleven §10.3 specimens: exactly one zero byte separates the two, where the
/// Commodore 64 releases carry 61, 76 or 101 (§6.2).
fn picture_block(image48k: &[u8], tables: &ZxTables) -> Result<usize, LoadError> {
    let from = offset_of(image48k, tables.pictures)
        .ok_or_else(|| bad("the picture pointer resolves outside the image"))?;
    let window = image48k
        .get(from..(from + PICTURE_BLOCK_SEARCH).min(image48k.len()))
        .ok_or_else(|| bad("the picture pointer is at the very end of the image"))?;
    let at = window
        .iter()
        .position(|&b| b == PICTURE_BLOCK_MARK)
        .ok_or_else(|| bad("no picture block after the item descriptions"))?;
    Ok(from + at)
}

/// Locate a release's artwork and decode it to display lists — §8.2's opcode
/// streams, one per room, resolved into [`PictureList`]s the caller can draw at
/// any size.
///
/// The decode itself is [`crate::c64::decode_family_b_lists_at_most`]: **the
/// stream is the same format at a different address**, and the only ZX
/// difference is the palette ([`PALETTE`] rather than the Commodore 64's with
/// its remap). The cap is the room count, because a Spectrum image is a fixed
/// 48K whose unused RAM follows the block — where a Commodore 64 program file
/// simply ends — and §8.6's Family B rule is "pure identity: room *n* shows
/// vector image *n* − 1", so a room count of *R* wants exactly *R* images.
/// Measured on all eleven: *R* images decode cleanly and what follows is either
/// filler or, in *Perseus and Andromeda*, one further image no room can show.
///
/// A block that runs out part-way through "leaves every remaining room with no
/// picture" (§8.2), so a short list is a partial load and not an error: the
/// caller is told how many pictures there are.
///
/// # Errors
///
/// [`parse_zx_mysterious`]'s own refusals, for the same reasons.
pub fn decode_picture_lists(image48k: &[u8]) -> Result<Vec<PictureList>, LoadError> {
    let layout = locate(image48k)?;
    let at = picture_block(image48k, &layout.tables)?;
    Ok(decode_family_b_lists_at_most(&image48k[at..], usize::from(layout.header.rooms)))
}

/// Locate and decode a release's artwork at §8.2's own 255 x 94 canvas —
/// [`decode_picture_lists`] plus
/// [`PictureList::rasterise`](crate::c64::PictureList::rasterise).
///
/// # Errors
///
/// [`parse_zx_mysterious`]'s own refusals, for the same reasons.
pub fn decode_pictures(image48k: &[u8]) -> Result<Vec<Picture>, LoadError> {
    Ok(decode_picture_lists(image48k)?.iter().map(PictureList::rasterise).collect())
}

/// [`decode_picture_lists`] over a `.z80` snapshot, container step included.
///
/// # Errors
///
/// [`parse_zx_mysterious_z80`]'s own refusals, for the same reasons.
pub fn decode_picture_lists_z80(file: &[u8]) -> Result<Vec<PictureList>, LoadError> {
    let image = crate::z80::decompress_z80(file)
        .map_err(|_| bad("the .z80 snapshot does not decompress to a 48K image"))?;
    decode_picture_lists(&image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, Presentation, StepResult, Vm};

    // ── A hand-built 48K image, constructed from the format above ─────────────
    //
    // Nothing here is copied from a specimen: the builder lays out one table
    // after another the way §4.3's early family orders them, writes the nine
    // pointers the driver plants, and leaves every other byte zero — which is
    // also what keeps the §4.6 scan's answer unique, since a zeroed window has
    // a count of 0 and fails `plausible` on its first test.

    const HEADER_AT: u16 = 0x6000;
    const ACTIONS_AT: u16 = 0x7000;
    const ROOMS: u16 = 10;
    const ITEMS: u16 = 10;
    const ACTIONS: u16 = 100;
    const WORDS: u16 = 50;
    const WORD_LENGTH: u16 = 4;
    const MESSAGES: u16 = 3;
    const MAX_CARRY: u16 = 6;
    const START_ROOM: u16 = 1;
    const TREASURES: u16 = 1;
    const LAMP: u16 = 200;
    const TREASURE_ROOM: u16 = 2;

    /// §8.2's move opcode, spelled here so the fixture reads as the format
    /// rather than as a magic number.
    const OP_MOVE_FIXTURE: u8 = 0xC0;
    /// §8.2's fill opcode.
    const OP_FILL_FIXTURE: u8 = 0xC1;

    /// The image under construction.
    struct Fixture {
        mem: Vec<u8>,
    }

    impl Fixture {
        fn new() -> Fixture {
            Fixture { mem: vec![0; crate::z80::IMAGE_LEN] }
        }

        fn put(&mut self, addr: u16, bytes: &[u8]) {
            let at = usize::from(addr) - usize::from(IMAGE_BASE);
            self.mem[at..at + bytes.len()].copy_from_slice(bytes);
        }

        fn put_word(&mut self, addr: u16, value: u16) {
            self.put(addr, &value.to_le_bytes());
        }

        /// A NUL-terminated string block, answering where it ended.
        fn put_strings(&mut self, addr: u16, strings: &[&str]) -> u16 {
            let mut at = addr;
            for s in strings {
                self.put(at, s.as_bytes());
                at += u16::try_from(s.len()).unwrap() + 1; // the NUL is already zero
            }
            at
        }

        /// (word count + 1) cells of (word length + 1) bytes, `head` spelling
        /// the first few.
        fn put_cells(&mut self, addr: u16, head: &[&str]) -> u16 {
            let width = usize::from(WORD_LENGTH) + 1;
            for (i, word) in head.iter().enumerate() {
                self.put(addr + u16::try_from(i * width).unwrap(), word.as_bytes());
            }
            addr + u16::try_from((usize::from(WORDS) + 1) * width).unwrap()
        }
    }

    /// Build the image, and answer it with the addresses it used.
    fn fixture() -> (Vec<u8>, ZxTables) {
        let mut f = Fixture::new();

        // §4.5's early field order, at HEADER_AT.
        f.put_word(HEADER_AT + 2, ITEMS);
        f.put_word(HEADER_AT + 4, ACTIONS);
        f.put_word(HEADER_AT + 6, WORDS);
        f.put_word(HEADER_AT + 8, ROOMS);
        f.put_word(HEADER_AT + 10, MAX_CARRY);
        f.put_word(HEADER_AT + 12, START_ROOM);
        f.put_word(HEADER_AT + 14, TREASURES);
        f.put_word(HEADER_AT + 16, WORD_LENGTH);
        f.put_word(HEADER_AT + 18, LAMP);
        f.put_word(HEADER_AT + 20, MESSAGES);
        f.put_word(HEADER_AT + 22, TREASURE_ROOM);

        // Actions: (count + 1) sixteen-byte records. Record 0 is verb 1 noun 2
        // with one condition and one command pair; the rest stay zero, which
        // is a legal "AUTO/ANY, no conditions, no commands" line.
        let span = (ACTIONS + 1) * ACTION_RECORD;
        f.put_word(ACTIONS_AT, 150 + 2);
        f.put_word(ACTIONS_AT + 2, 20 * 3 + 5); // condition code 5, value 3
        f.put_word(ACTIONS_AT + 12, 52 * 150 + 66); // commands 52 and 66
        // The sixteen bytes in FRONT of the table must not read as a record,
        // or the action count could be one too small — the check
        // `parse_zx_mysterious` makes and the driver's own code satisfies.
        f.put_word(ACTIONS_AT - ACTION_RECORD, 0xFFFF);

        // Room connections: (rooms + 1) six-byte records; room 1 leads north
        // to room 2 and room 2 south back to room 1.
        let connections = ACTIONS_AT + span;
        f.put(connections + 6, &[2, 0, 0, 0, 0, 0]);
        f.put(connections + 12, &[0, 1, 0, 0, 0, 0]);

        // Item locations, and the driver's working copy of them: item 0 in
        // room 1, item 1 carried, item 2 out of play.
        let locations = connections + (ROOMS + 1) * EXITS_PER_ROOM;
        f.put(locations, &[1, STORED_CARRIED, 0]);
        let locations_copy = locations + ITEMS + 1;
        f.put(locations_copy, &[1, STORED_CARRIED, 0]);

        // The dictionary: two blocks of (words + 1) five-byte cells, the verb
        // block opening on §4.1's signature. Cell 2 of each block is a
        // synonym, spelled with the `*` inside the cell.
        let verbs = locations_copy + ITEMS + 1 + 8;
        let nouns = f.put_cells(verbs, &["AUTO", "GO", "*WALK", "TAKE"]);
        let rooms = f.put_cells(nouns, &["ANY", "NORT", "SOUT", "LAMP"]);

        // §4.4's three string blocks, back to back, each ending where the next
        // pointer says.
        let messages = f.put_strings(
            rooms,
            &[
                "limbo",
                "dusty study",
                "*You are nowhere at all",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ],
        );
        let items = f.put_strings(messages, &["first", "second", "third", "fourth"]);
        let pictures = f.put_strings(
            items,
            &[
                "*A brass lamp/LAMP/",
                "a rusty key/KEY/",
                "scenery",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ],
        );

        // The nine pointers the driver plants, plus slot 0.
        f.put_word(HEADER_AT, pictures);
        f.put_word(HEADER_AT + 24, rooms);
        f.put_word(HEADER_AT + 26, connections);
        f.put_word(HEADER_AT + 28, locations);
        f.put_word(HEADER_AT + 30, locations_copy);
        f.put_word(HEADER_AT + 32, verbs);
        f.put_word(HEADER_AT + 34, nouns);
        f.put_word(HEADER_AT + 36, rooms);
        f.put_word(HEADER_AT + 38, messages);
        f.put_word(HEADER_AT + 40, items);

        // §8.2's picture block: the leading `$FF`, then one image per room.
        // Image 0 is a background, a move, a line and a fill; the rest are
        // bare backgrounds. Room *n* shows image *n* − 1 (§8.6), so ten rooms
        // want ten images.
        let mut at = pictures + 1; // one filler byte, as the specimens have
        f.put(at, &[PICTURE_BLOCK_MARK]);
        at += 1;
        f.put(at, &[3, OP_MOVE_FIXTURE, 100, 10, 0x70, 20, OP_FILL_FIXTURE, 5, 100, 12, 0xFF]);
        at += 11;
        for _ in 1..ROOMS {
            f.put(at, &[7, 0xFF]);
            at += 2;
        }

        let tables = ZxTables {
            actions: ACTIONS_AT,
            connections,
            locations,
            locations_copy,
            verbs,
            nouns,
            rooms,
            messages,
            items,
            pictures,
        };
        (f.mem, tables)
    }

    // ── The located layout ────────────────────────────────────────────────────

    #[test]
    fn locate_finds_one_header_and_the_whole_pointer_block() {
        let (image, tables) = fixture();
        let layout = locate(&image).expect("the hand-built image locates");
        assert_eq!(layout.header.addr, HEADER_AT);
        assert_eq!(layout.header.shape, HeaderShape::Early);
        assert_eq!(layout.header.items, ITEMS);
        assert_eq!(layout.header.actions, ACTIONS);
        assert_eq!(layout.header.words, WORDS);
        assert_eq!(layout.header.rooms, ROOMS);
        assert_eq!(layout.header.max_carry, MAX_CARRY);
        assert_eq!(layout.header.start_room, START_ROOM);
        assert_eq!(layout.header.treasures, TREASURES);
        assert_eq!(layout.header.word_length, WORD_LENGTH);
        assert_eq!(layout.header.lamp, LAMP);
        assert_eq!(layout.header.messages, MESSAGES);
        assert_eq!(layout.header.treasure_room, TREASURE_ROOM);
        assert_eq!(layout.tables, tables);
        assert!(looks_like_zx_mysterious(&image));
    }

    #[test]
    fn the_hand_built_image_reads_as_the_database_it_spells() {
        let (image, _) = fixture();
        let db = parse_zx_mysterious(&image).expect("the fixture parses");

        assert_eq!(db.max_carry, i32::from(MAX_CARRY));
        assert_eq!(db.start_room, usize::from(START_ROOM));
        assert_eq!(db.num_treasures, i32::from(TREASURES));
        assert_eq!(db.word_length, usize::from(WORD_LENGTH));
        assert_eq!(db.light_time, i32::from(LAMP));
        assert_eq!(db.treasure_room, usize::from(TREASURE_ROOM));
        assert_eq!(db.adventure_number, 0);

        // §4.4's action packing, the reference format's arithmetic in
        // little-endian words.
        assert_eq!(db.actions.len(), usize::from(ACTIONS) + 1);
        let a = &db.actions[0];
        assert_eq!((a.verb, a.noun), (1, 2));
        assert_eq!((a.conditions[0].code, a.conditions[0].value), (5, 3));
        assert_eq!(a.commands, [52, 66, 0, 0]);

        // §4.2's grid, both blocks, synonyms spelled with the `*` inside the
        // cell — and (word count + 1) cells in each, which is the split this
        // module reads rather than tabulates.
        assert_eq!(db.verbs.len(), usize::from(WORDS) + 1);
        assert_eq!(db.nouns.len(), usize::from(WORDS) + 1);
        assert_eq!(&db.verbs[..4], ["AUTO", "GO", "*WALK", "TAKE"]);
        assert_eq!(&db.nouns[..4], ["ANY", "NORT", "SOUT", "LAMP"]);
        assert_eq!(db.verbs[4], "", "an all-NUL cell is an empty cell, not a skipped byte");

        // Rooms: exits room-major, and §4.4's leading `*` stripped into
        // `literal`.
        assert_eq!(db.rooms.len(), usize::from(ROOMS) + 1);
        assert_eq!(db.rooms[1].desc, "dusty study");
        assert_eq!(db.rooms[1].exits, [2, 0, 0, 0, 0, 0]);
        assert!(!db.rooms[1].literal);
        assert_eq!(db.rooms[2].desc, "You are nowhere at all");
        assert!(db.rooms[2].literal);

        assert_eq!(db.messages.len(), usize::from(MESSAGES) + 1);
        assert_eq!(db.messages, ["first", "second", "third", "fourth"]);

        // §2.5's auto-noun and the `*` treasure marker, and §4.4's item
        // locations with 255 meaning carried.
        assert_eq!(db.items.len(), usize::from(ITEMS) + 1);
        assert_eq!(db.items[0].text, "*A brass lamp");
        assert_eq!(db.items[0].auto_noun.as_deref(), Some("LAMP"));
        assert!(db.items[0].treasure);
        assert_eq!(db.items[0].start_loc, 1);
        assert_eq!(db.items[1].auto_noun.as_deref(), Some("KEY"));
        assert_eq!(db.items[1].start_loc, crate::database::CARRIED);
        assert_eq!(db.items[2].auto_noun, None);
        assert_eq!(db.items[2].start_loc, 0);

        // §9.2 and §9.3, the two facts that travel with the database.
        assert!(db.mysterious, "§9.2's two lamp options must be forced");
        assert!(db.second_person, "§9.3: a ZX Mysterious release is second person");
        assert!(db.ti99.is_none());
        assert!(db.saga_us.is_none());
    }

    #[test]
    fn the_forced_options_reach_the_vm() {
        let (image, _) = fixture();
        let db = parse_zx_mysterious(&image).unwrap();
        // Every flag OFF in what the host asks for; the database forces three.
        let vm =
            Vm::new_full(db, false, 1, Options::new().with_presentation(Presentation::ScottFree));
        assert!(vm.options().you_are, "§9.3's wording is the platform's, and forced");
        assert!(vm.options().scott_light, "§9.2's countdown");
        assert!(vm.options().prehistoric_lamp, "§9.2's lamp destruction");
        // Presentation stays the host's: §6.4's separators are presentation,
        // and this loader does not touch them.
        assert_eq!(vm.options().presentation, Presentation::ScottFree);
    }

    #[test]
    fn the_hand_built_world_plays() {
        let (image, _) = fixture();
        let mut vm = Vm::new(parse_zx_mysterious(&image).unwrap());
        assert_eq!(vm.step(), StepResult::NeedLine);
        assert_eq!(vm.current_room(), 1);
        let _ = vm.take_output();
        vm.supply_line("go north");
        assert_eq!(vm.step(), StepResult::NeedLine);
        assert_eq!(vm.current_room(), 2, "the exits table drives movement");
    }

    // ── Pictures (§8.2, §8.6) ─────────────────────────────────────────────────

    #[test]
    fn the_picture_block_decodes_one_image_per_room() {
        let (image, _) = fixture();
        let lists = decode_picture_lists(&image).expect("the fixture's pictures decode");
        assert_eq!(lists.len(), usize::from(ROOMS), "§8.6: room n shows image n − 1");
        // §8.2's derived line colour: 7 when the background is 0, else 0.
        assert_eq!(lists[0].background, 3);
        assert_eq!(lists[0].line, 0);
        assert_eq!(lists[1].background, 7);
        // One move (drawing nothing), one line, one fill.
        assert_eq!(lists[0].ops.len(), 2);
        assert_eq!(
            lists[0].ops[0],
            crate::c64::PictureOp::Line { from: (10, 190 - 100), to: (20, 190 - 0x70) }
        );
        assert_eq!(
            lists[0].ops[1],
            crate::c64::PictureOp::Fill { seed: (12, 190 - 100), colour: 5 }
        );
        // And they rasterise on §8.2's own canvas.
        let drawn = decode_pictures(&image).unwrap();
        assert_eq!(drawn.len(), usize::from(ROOMS));
        assert_eq!(drawn[0].width, crate::c64::PICTURE_WIDTH);
        assert_eq!(drawn[0].height, crate::c64::PICTURE_HEIGHT);
    }

    #[test]
    fn the_palette_is_the_optimised_sinclair_one_with_no_remap() {
        // §8.1's table, spot-checked at both ends of both halves, and the
        // identity remap §8.2 specifies for ZX releases: index 7 is the plain
        // white the artwork's line colour resolves through, not the Commodore
        // 64 palette's yellow.
        assert_eq!(PALETTE[0], (0, 0, 0));
        assert_eq!(PALETTE[7], (202, 202, 202));
        assert_eq!(PALETTE[15], (255, 255, 255));
        assert_ne!(PALETTE[7], crate::c64::PALETTE[7]);
    }

    // ── Refusals (§11) ────────────────────────────────────────────────────────

    #[test]
    fn an_image_with_no_signature_is_refused_by_name() {
        let image = vec![0u8; crate::z80::IMAGE_LEN];
        assert_eq!(
            locate(&image),
            Err(LoadError::UnsupportedDialect(Dialect::C64OrZxSnapshot)),
            "no dictionary signature at all"
        );
        assert!(!looks_like_zx_mysterious(&image));
    }

    #[test]
    fn a_second_signature_is_refused_rather_than_taken_as_the_first() {
        let (mut image, _) = fixture();
        // §4.1's search is unanchored and takes the first hit; this module
        // requires uniqueness, so planting a second copy is a refusal and not
        // a silently different dictionary.
        let elsewhere = usize::from(0x5000 - IMAGE_BASE);
        image[elsewhere..elsewhere + DICTIONARY_SIGNATURE.len()]
            .copy_from_slice(DICTIONARY_SIGNATURE);
        assert!(matches!(locate(&image), Err(LoadError::BadDialectData(..))));
    }

    #[test]
    fn a_signature_with_no_header_around_it_is_refused_by_name() {
        // What a family-A ZX release (§8.1) and a compressed-action one (§5.1)
        // both do: the dictionary is there and no early-shape header fits.
        let mut image = vec![0u8; crate::z80::IMAGE_LEN];
        let at = usize::from(0x8000 - IMAGE_BASE);
        image[at..at + DICTIONARY_SIGNATURE.len()].copy_from_slice(DICTIONARY_SIGNATURE);
        assert_eq!(
            parse_zx_mysterious(&image),
            Err(LoadError::UnsupportedDialect(Dialect::C64OrZxSnapshot))
        );
    }

    #[test]
    fn an_action_count_one_too_small_is_caught_by_the_preceding_record() {
        let (mut image, _) = fixture();
        // Blank the sixteen bytes in front of the action table so they read as
        // a possible record: the count would then be one too small, and this
        // is the check that says so rather than reading a table shifted by one
        // record.
        let at = usize::from(ACTIONS_AT - ACTION_RECORD - IMAGE_BASE);
        image[at..at + usize::from(ACTION_RECORD)].fill(0);
        assert!(matches!(parse_zx_mysterious(&image), Err(LoadError::BadDialectData(..))));
    }

    #[test]
    fn a_connection_pointing_outside_the_room_table_is_refused() {
        let (mut image, tables) = fixture();
        let at = usize::from(tables.connections - IMAGE_BASE);
        image[at + 6] = u8::try_from(ROOMS).unwrap() + 1;
        assert!(matches!(parse_zx_mysterious(&image), Err(LoadError::BadDialectData(..))));
    }

    // ── Robustness ────────────────────────────────────────────────────────────

    #[test]
    fn every_single_byte_flip_either_loads_or_refuses() {
        // 200 pseudo-random single-byte edits of the hand-built image: the
        // loader may load, may refuse, and must never panic. (The
        // real-specimen half of this lives in `zx_specimens.rs`, which skips
        // without the corpus; this one runs everywhere.)
        let (base, _) = fixture();
        let mut seed = 0x1234_5678u32;
        for _ in 0..200 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let at = (seed >> 8) as usize % base.len();
            let value = (seed >> 3) as u8;
            let mut image = base.clone();
            image[at] = value;
            let _ = parse_zx_mysterious(&image);
            let _ = decode_picture_lists(&image);
            let _ = identify(&image);
        }
    }

    #[test]
    fn a_truncated_image_is_refused_rather_than_indexed() {
        let (image, _) = fixture();
        for len in [0, 1, 0x2000, 0x6000, image.len() - 1] {
            let _ = parse_zx_mysterious(&image[..len]);
        }
    }

    // ── Identification, which is for titles only ──────────────────────────────

    #[test]
    fn the_hand_built_image_is_not_one_of_the_eleven_and_still_loads() {
        let (image, _) = fixture();
        assert!(identify(&image).is_none(), "the fixture is no catalogued release");
        assert!(parse_zx_mysterious(&image).is_ok(), "and loading never consults the table");
    }

    #[test]
    fn the_release_table_is_keyed_uniquely() {
        // §6.1's identity is the seven counts; if two rows shared them the
        // table could name the wrong game.
        let mut keys: Vec<_> = RELEASES
            .iter()
            .map(|r| (r.items, r.actions, r.words, r.rooms, r.max_carry, r.word_length, r.messages))
            .collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "two releases share the seven header counts");
        assert_eq!(RELEASES.len(), 11);
    }
}
