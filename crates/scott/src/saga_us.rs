//! Reads the **US S.A.G.A. binary database** — the American "Scott Adams
//! Graphic Adventure" disk releases for the Atari 8-bit, the Apple II and the
//! Commodore 64, plus the Questprobe *Hulk* — into the same [`Database`] every
//! other Scott Adams dialect decodes to.
//!
//! The input is whatever the disk container hands over, unshifted: the whole
//! ATR image for an Atari release, the whole DOS 3.3 file for an Apple II one,
//! the whole program file (load-address bytes included) for the Commodore 64.
//! [`SagaPlatform`] carries the one per-platform constant that turns those
//! bytes into the database array, chosen — as the specification puts it — so
//! that "the header always lands 0x38 bytes into the array".
//!
//! # Provenance
//!
//! Everything here is implemented from
//! [`docs/internals/scott-dialects-spec.md`](../../../docs/internals/scott-dialects-spec.md)
//! §12 (the whole of it: §12.1's region order, §12.2 detection, §12.3 the
//! per-platform reach, §12.4 the header and its 29-byte consume rule, §12.5
//! the dictionary, §12.6 the length-prefixed strings, §12.7 the pointer tables
//! and the base, §12.8 the column-major actions, §12.9 the direction-major
//! connections, §12.11 the runtime differences, §12.14 the refusals), together
//! with §2 for the reference-format conventions it inherits. That document was
//! written under the clean-room protocol in `docs/internals/clean-room.md`.
//! **No GPL interpreter's source was read to write this module** — lanthorn is
//! BSD-3-Clause and every established Scott Adams interpreter is GPL, so the
//! specification plus the specimens are the only channels by which a fact
//! about this format reached this file.
//!
//! # The layout, in one picture
//!
//! ```text
//!   0x00  front matter: version and adventure number   (§12.2)
//!   0x38  header, 15 words, of which 29 BYTES are consumed  (§12.4)
//!         gap  →  dictionary: nouns then verbs, found by scanning for `ANY`
//!         →  room descriptions  →  messages  →  item descriptions   (§12.6)
//!         →  separator  →  item locations  →  item pointers  →  locations again
//!         →  actions, COLUMN-major  →  room pointers                (§12.7-8)
//!         →  room connections, DIRECTION-major  →  message pointers (§12.9)
//! ```
//!
//! # What is not here
//!
//! **Pictures.** §12.10: the database says nothing about them at all — no
//! room-image list, no item-flag list, no item-image list. The artwork lives
//! in separate files on the disk, and the per-title (usage, index, offset,
//! length) lists a decoder would need "are not recoverable from the database".
//! [`SagaUs`] carries the two picture facts that ARE derivable — a room's
//! picture is its own number, and the *Hulk* remap — and nothing else.

use crate::database::CARRIED;
use crate::loader::{extract_auto_noun, Dialect, LoadError};
use crate::{Action, Condition, Database, Item, Room};

// ── Fixed offsets and sizes (§12.1, §12.4) ────────────────────────────────────

/// How many bytes of front matter precede the header (§12.1 region 1), and so
/// the header's own offset within the array (§12.4).
const HEADER_AT: usize = 0x38;

/// How many little-endian words the header holds (§12.4). Only the first
/// eleven carry fields; words 11 to 14 carry none, and on the Commodore 64
/// *Hulk* they are already the dictionary.
const HEADER_WORDS: usize = 15;

/// **A reader consumes twenty-nine bytes of header, not thirty** (§12.4): the
/// fifteenth word's low byte is re-read, and the next region begins at header
/// start + 29. On the *Hulk* the noun cell `ANY` begins at exactly that byte,
/// so a reader advancing a full thirty steps over the `A` and never finds the
/// dictionary at all.
const HEADER_CONSUMED: usize = 29;

/// The first noun cell, and the only thing that locates the dictionary (§12.5):
/// there is no signature table and no back-off, and the dictionary begins on
/// the `A`.
const DICTIONARY_ANCHOR: &[u8] = b"ANY";

/// The 228-byte allocation the message pointers sit in (§12.7). The Commodore
/// 64 *Hulk*'s database file ends exactly this far past its connection table.
const MESSAGE_POINTER_ALLOCATION: usize = 0xE4;

/// Bytes per action record, whatever the layout (§12.8): the table is
/// 16 x (actions + 1) bytes column-major, exactly as row-major would be.
const ACTION_RECORD: usize = 16;

/// Exits per room (§12.9), stored as six blocks of one direction each.
const EXITS_PER_ROOM: usize = 6;

/// The stored byte meaning "the player is carrying this" (§12.7), normalised
/// to [`CARRIED`] at load time as everywhere else in this crate.
const STORED_CARRIED: u16 = 255;

/// A condition word is `code + 20 x value` (§2.3); §12.8 notes the stored
/// words are "already in reference form".
const CONDITION_RADIX: u16 = 20;

/// The placeholder a zero-length string decodes to (§12.6: "**A length of 0
/// means the string `.`** — the reference format's placeholder").
const EMPTY_STRING: &str = ".";

// ── §12.4's sanity limits ─────────────────────────────────────────────────────

/// The header ranges §12.4 gives, all inclusive. A header failing any of them
/// means "not this format", not "a corrupt file" (§12.14).
const COUNT_RANGES: [(&str, u16, u16); 5] = [
    ("items", 10, 500),
    ("actions", 100, 500),
    ("words", 50, 200),
    ("rooms", 10, 100),
    ("messages", 10, 255),
];

/// The three further limits §12.4's whole-image scan adds to the five above —
/// max carried, word length, and a start room within the room count. Scanning
/// an Atari side-A image for a window satisfying all eight yields **exactly
/// one** hit, at file offset 0x04F9, which is what makes [`SagaPlatform`]'s
/// mastering constant checkable rather than merely asserted.
const MAX_CARRY_RANGE: (u16, u16) = (1, 20);
/// The word-length half of the same scan (§12.4).
const WORD_LENGTH_RANGE: (u16, u16) = (3, 6);

// ── Platforms (§12.3) ─────────────────────────────────────────────────────────

/// Which container a US S.A.G.A. database arrived in, and so where in the
/// bytes the array starts (§12.3).
///
/// The container work itself is the host's — this crate reads no disk images
/// — but each platform's array offset is a property of the *format* rather
/// than of the container, "chosen so that the header always lands 0x38 bytes
/// into the array", so it belongs here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SagaPlatform {
    /// **Atari 8-bit** (§12.3). The bytes are the whole ATR disk image, header
    /// and all, and the database is the range beginning at file offset
    /// **0x04C1** and running to the end, taken as-is.
    ///
    /// It is not a file in the disk's own directory — walking the Atari DOS
    /// catalogue of these images finds only `DOS.SYS`, `AUTORUN.SYS` and (on
    /// *Strange Odyssey*) `NIR.SYS` — and the range carries no per-sector link
    /// trailers, which is what lets a flat slice work at all. That is a
    /// property of how these disks were mastered rather than of the container.
    /// Side A holds the database and side B the pictures.
    Atari8Bit,
    /// **Apple II** (§12.3). The bytes are the raw sector concatenation of the
    /// DOS 3.3 file on the **boot** side whose name is `DATABASE` or matches
    /// `A?.DAT`, its four-byte binary prologue (load address, then data
    /// length) included, and the array begins at offset **0x135** of that.
    ///
    /// Every one of the seven releases is 42 catalogue sectors of which 41
    /// hold data, loads at `$4000`, and declares a data length of 10,335
    /// bytes.
    AppleII,
    /// **Commodore 64** (§12.3). The bytes are the program file read out of
    /// the disk image through its block chain — for the *Hulk*, the file named
    /// `SHULK.DB` — **including** its two load-address bytes, so the array
    /// begins at offset **0**. The header falls at 0x38 with no gap after it
    /// at all.
    Commodore64,
}

impl SagaPlatform {
    /// Where in the container's bytes the database array begins (§12.3).
    pub const fn array_offset(self) -> usize {
        match self {
            SagaPlatform::Atari8Bit => 0x04C1,
            SagaPlatform::AppleII => 0x135,
            SagaPlatform::Commodore64 => 0,
        }
    }

    /// A short human name for the platform, for a host reporting what it found.
    pub const fn label(self) -> &'static str {
        match self {
            SagaPlatform::Atari8Bit => "Atari 8-bit",
            SagaPlatform::AppleII => "Apple II",
            SagaPlatform::Commodore64 => "Commodore 64",
        }
    }

    /// Every platform, in the order [`detect_saga_us`] probes them.
    pub const ALL: [SagaPlatform; 3] =
        [SagaPlatform::Atari8Bit, SagaPlatform::AppleII, SagaPlatform::Commodore64];
}

// ── The release identity (§12.2, §12.11) ──────────────────────────────────────

/// Which US S.A.G.A. release a [`Database`] was loaded from, and the handful
/// of runtime facts §12.11 makes properties of the database rather than of the
/// host.
///
/// **Only the pair identifies a title** (§12.2). Neither number does it alone:
/// adventure 1 is *Adventureland* at version 416 and the *Hulk* at version
/// 127. The version is the release build number and the adventure number is
/// the Adventure International series number.
/// Every field is `pub` and the struct is exhaustive, for the same reason
/// [`Database`]'s are: a host may build a [`Database`] by hand, and one
/// standing in for a S.A.G.A. release has to be able to say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SagaUs {
    /// The release build number — 416, 408, 306, 119, 115, 119 and 125 for the
    /// seven Atari titles, the same six plus 122 for the Apple II, and 127 for
    /// the Commodore 64 *Hulk* (§12.2, §12.12).
    pub version: u16,
    /// The Adventure International series number (§12.2).
    pub adventure: u16,
    /// Which container this release came out of, which §12.11 needs: the
    /// *Hulk*'s room-picture remap applies on the Commodore 64 and Atari 8-bit
    /// releases but **not** on the Apple II one.
    pub platform: SagaPlatform,
}

impl SagaUs {
    /// The picture index a room's view is drawn from (§12.11).
    ///
    /// **A room's picture is the room number**, with no room-image byte and no
    /// 255 sentinel — §12.10: "there is no table to consult". The one
    /// exception is the *Hulk*, which "remaps five pairs of rooms onto other
    /// rooms' pictures — 5 and 6 onto 3, 7 and 8 onto 4, 10 and 11 onto 9, 13
    /// and 14 onto 2, 17 and 18 onto 16 — on the Commodore 64 and Atari 8-bit
    /// releases but **not** on the Apple II one".
    ///
    /// A host still owns everything else about drawing: after the room picture
    /// come the object pictures of the items presently in the room, and the
    /// per-title (usage, index, offset, length) lists that needs are not in
    /// the database at all (§12.10).
    pub fn room_picture(&self, room: usize) -> usize {
        if !self.remaps_hulk_rooms() {
            return room;
        }
        hulk_room_picture(room)
    }

    /// Whether this is the US *Hulk* on a platform that remaps room pictures
    /// (§12.11) — the Commodore 64 and Atari 8-bit releases, not the Apple II
    /// one. §12.2: "version 127 with adventure 1 is the US *Hulk*".
    pub fn remaps_hulk_rooms(&self) -> bool {
        self.version == 127
            && self.adventure == 1
            && !matches!(self.platform, SagaPlatform::AppleII)
    }

    /// The box title for this release, platform folded in — "Voodoo Castle
    /// (Atari 8-bit)" — from §12.12's per-release table (SQ-1470).
    ///
    /// **The platform has to be part of it.** Only the (version, adventure)
    /// pair identifies a title (see this struct's own doc), but the SAME
    /// title exists on more than one platform — *Adventureland* is (416, 1)
    /// on both the Atari 8-bit and the Apple II — so a host distinguishing
    /// two disks' saves (container mounting is the host's business; see the
    /// module doc) needs the platform in the answer too, not just the title.
    /// *Claymorgue Castle* is the opposite case: its own version differs BY
    /// platform (125 Atari, 122 Apple II), so the two arms below never
    /// collide with each other even without the platform guard the others
    /// need.
    ///
    /// A `&'static str` table, not `format!`, so a host's own bundled-title
    /// lookup (which wants to hold one without allocating) can use it as-is.
    /// `None` for a release outside §12.12's seven, which cannot happen for
    /// this crate's own detector but leaves a caller a graceful fallback
    /// instead of an unwrap.
    pub fn display_title(&self) -> Option<&'static str> {
        match (self.version, self.adventure, self.platform) {
            (416, 1, SagaPlatform::Atari8Bit) => Some("Adventureland (Atari 8-bit)"),
            (416, 1, SagaPlatform::AppleII) => Some("Adventureland (Apple II)"),
            (408, 2, SagaPlatform::Atari8Bit) => Some("Pirate Adventure (Atari 8-bit)"),
            (408, 2, SagaPlatform::AppleII) => Some("Pirate Adventure (Apple II)"),
            (306, 3, SagaPlatform::Atari8Bit) => Some("Mission Impossible (Atari 8-bit)"),
            (306, 3, SagaPlatform::AppleII) => Some("Mission Impossible (Apple II)"),
            (119, 4, SagaPlatform::Atari8Bit) => Some("Voodoo Castle (Atari 8-bit)"),
            (119, 4, SagaPlatform::AppleII) => Some("Voodoo Castle (Apple II)"),
            (115, 5, SagaPlatform::Atari8Bit) => Some("The Count (Atari 8-bit)"),
            (115, 5, SagaPlatform::AppleII) => Some("The Count (Apple II)"),
            (119, 6, SagaPlatform::Atari8Bit) => Some("Strange Odyssey (Atari 8-bit)"),
            (119, 6, SagaPlatform::AppleII) => Some("Strange Odyssey (Apple II)"),
            (125, 13, SagaPlatform::Atari8Bit) => {
                Some("The Sorcerer of Claymorgue Castle (Atari 8-bit)")
            }
            (122, 13, SagaPlatform::AppleII) => {
                Some("The Sorcerer of Claymorgue Castle (Apple II)")
            }
            (127, 1, SagaPlatform::Commodore64) => Some("The Hulk (Commodore 64)"),
            _ => None,
        }
    }
}

/// §12.11's *Hulk* room-picture remap, on its own: rooms 5 and 6 draw picture
/// 3, 7 and 8 draw 4, 10 and 11 draw 9, 13 and 14 draw 2, and 17 and 18 draw
/// 16; every other room draws its own number.
///
/// **The one place this table is written down.** Two callers need it and they
/// reach the release by different routes: [`SagaUs::room_picture`], for the
/// Commodore 64 and Atari 8-bit releases, whose §12 binary database
/// identifies itself; and
/// [`crate::saga_dos::DosRelease::room_picture`](crate::saga_dos::DosRelease::room_picture),
/// for the MS-DOS one, whose database is the plain reference text format and
/// says nothing about itself at all (§10.7). A second copy of five pairs is a
/// second place for them to go stale.
///
/// Says nothing about WHICH releases remap — that is
/// [`SagaUs::remaps_hulk_rooms`] and `DosRelease::remaps_hulk_rooms`, because
/// the answer differs by platform (the Apple II release does not).
pub fn hulk_room_picture(room: usize) -> usize {
    match room {
        5 | 6 => 3,
        7 | 8 => 4,
        10 | 11 => 9,
        13 | 14 => 2,
        17 | 18 => 16,
        other => other,
    }
}

/// The picture drawn in the dark (§12.11).
///
/// **Darkness does not blank the graphics window** in these releases. Where
/// other dialects paint black, these draw a dedicated darkness image and
/// return.
pub const DARKNESS_PICTURE: usize = 0;

/// The picture the inventory command draws behind the carried objects
/// (§12.11).
///
/// **The inventory command draws a picture**: beyond listing what is carried,
/// it clears the graphics window, draws this index as a room picture, then
/// draws the inventory-object picture of every carried item, and waits for the
/// player to press ENTER before restoring the room view. On the Atari 8-bit
/// this image comes from the companion disk rather than from the database
/// side.
pub const INVENTORY_PICTURE: usize = 98;

// ── Picture files (§8.3, §8.6) ────────────────────────────────────────────────

/// What a family-C picture is FOR (§8.6), read off its file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PictureUsage {
    /// A **room** picture, shown when the player is in the room with that
    /// index — leading `R`. A leading `S` "carries no usage and defaults to a
    /// room picture" (§8.6) and reports this too.
    Room,
    /// An **object in a room**: overlaid on the room picture when the item
    /// with that index is in the player's room — leading `B`, trailing `R`.
    ObjectInRoom,
    /// An **object in the inventory**: drawn on the inventory screen when the
    /// item with that index is carried — leading `B`, trailing `I`.
    ObjectInInventory,
}

/// One picture file's name, taken apart (§8.3's Commodore 64 rule and §8.6's
/// usage convention).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PictureFile {
    /// What the picture is for.
    pub usage: PictureUsage,
    /// The picture index — a room number for [`PictureUsage::Room`], an item
    /// number otherwise. Three reserved values (§8.6): 0 is the darkness
    /// picture ([`DARKNESS_PICTURE`]), 98 the inventory backdrop
    /// ([`INVENTORY_PICTURE`]) and 99 the title picture.
    pub index: u16,
}

/// Take a Commodore 64 disk entry's name apart (§8.3), or `None` if it is not
/// a picture file.
///
/// §8.3's rule: "a file is a picture if its name is at least four characters,
/// its first character is `R`, `B` or `S`, and its second through fourth are
/// digits; the picture index is the three-digit field at positions 3-5". So
/// `R01000` is room picture 0, `R01099` the title picture, `B01013R` object 13
/// drawn in a room and `B01013I` object 13 drawn in the inventory.
///
/// **The three-digit field starts at position 3 and so overlaps the digits the
/// predicate tests**: positions 1-3 must be digits and positions 3-5 are the
/// index, which is why `R01000`'s index is 0 and not 10. The `01` in the middle
/// is the Adventure International series number, and this rule reads it as
/// part of neither field.
///
/// Case-insensitive on the letters, because a Commodore directory stores names
/// in PETSCII and a host may have upper- or lower-cased them on the way here.
///
/// This is deliberately **not** the MS-DOS rule (§8.5 uses two-digit room
/// indices, and §10.7 found a release using three-digit names with no `01`
/// prefix at all) — this crate reads the Commodore 64 releases.
pub fn parse_picture_file_name(name: &str) -> Option<PictureFile> {
    let b = name.as_bytes();
    if b.len() < 6 {
        return None;
    }
    if !b[1..4].iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let digits = std::str::from_utf8(&b[3..6]).ok()?;
    if !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let index: u16 = digits.parse().ok()?;
    let usage = match b[0].to_ascii_uppercase() {
        b'R' | b'S' => PictureUsage::Room,
        b'B' => match b.last()?.to_ascii_uppercase() {
            b'R' => PictureUsage::ObjectInRoom,
            b'I' => PictureUsage::ObjectInInventory,
            _ => return None,
        },
        _ => return None,
    };
    Some(PictureFile { usage, index })
}

/// Is `name` a family-C picture file (§8.3)? [`parse_picture_file_name`]
/// without the parts, for a host walking a disk directory to count or collect
/// them.
pub fn is_picture_file_name(name: &str) -> bool {
    parse_picture_file_name(name).is_some()
}

/// The file name a **room** picture with index `n` is stored under on `platform`
/// (§8.3, §8.6), or `None` when the platform does not name its pictures at all.
///
/// The Commodore 64 answer is `R01nnn` — `R` for the room usage, `01` for the
/// Adventure International series number the *Hulk* disk uses, and `n` in three
/// digits. It is the inverse of [`parse_picture_file_name`] for the room usage,
/// and the lookup a host needs once [`SagaUs::room_picture`] has given it a
/// picture index.
///
/// `None` for the **Atari 8-bit**, which has no filesystem walk: §8.3 puts its
/// pictures at hard-coded byte offsets into the companion picture side, and
/// §12.10 says those per-title lists "are not recoverable from the database".
/// `None` for the **Apple II**, whose pictures are family D (§8.4) under their
/// own names. Neither is a defect in this function; both are the honest answer
/// that a name is not how you find that platform's artwork.
///
/// `None` for an index above 999, which no three-digit field can spell.
pub fn picture_file_name(platform: SagaPlatform, n: usize) -> Option<String> {
    match platform {
        SagaPlatform::Commodore64 => (n <= 999).then(|| format!("R01{n:03}")),
        SagaPlatform::Atari8Bit | SagaPlatform::AppleII => None,
    }
}

// ── Detection (§12.2) ─────────────────────────────────────────────────────────

/// A refusal for bytes that ARE this format but did not check out (§12.14).
fn bad(what: &'static str) -> LoadError {
    LoadError::BadDialectData(Dialect::SagaUsDatabase, what)
}

/// One little-endian word at `off`.
fn word(a: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*a.get(off)?, *a.get(off + 1)?]))
}

/// §12.2's front-matter scan: **read the first 0x38 bytes as twenty-eight
/// little-endian words; the first non-zero value below 500 is the format
/// version and the next value below 500 is the adventure number.**
///
/// The zero skip is load-bearing: a scan that lets a zero stand as the version
/// answers (0, 0) on the Apple II *Claymorgue* database, whose words at
/// offsets 0x02 through 0x21 are every one of them zero. A database whose
/// adventure number comes out zero is not one of these.
///
/// Measured, the pair sits at a fixed place per platform but not across
/// platforms — offsets 0x22 and 0x26 on all fourteen Atari and Apple
/// databases, 0x34 and 0x36 on the Commodore 64 *Hulk* — which is why this is
/// a scan rather than two constants.
fn front_matter(array: &[u8]) -> Option<(u16, u16)> {
    let mut version: Option<u16> = None;
    for off in (0..HEADER_AT).step_by(2) {
        let v = word(array, off)?;
        match version {
            None if v != 0 && v < 500 => version = Some(v),
            None => {}
            Some(version) if v < 500 => return Some((version, v)),
            Some(_) => {}
        }
    }
    None
}

/// The eleven counts §4.5's **US** field order assigns (§12.4): word length 0,
/// words 1, actions 2, items 3, messages 4, rooms 5, max carried 6, start room
/// 7, treasure count 8, lamp turns 9, and the treasure room in the **high
/// byte** of word 10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Header {
    word_length: u16,
    words: u16,
    actions: u16,
    items: u16,
    messages: u16,
    rooms: u16,
    max_carry: u16,
    start_room: u16,
    treasures: u16,
    lamp: u16,
    treasure_room: u16,
}

/// Read the fifteen-word header at [`HEADER_AT`] (§12.4).
fn read_header(array: &[u8]) -> Option<Header> {
    let raw = array.get(HEADER_AT..HEADER_AT + HEADER_WORDS * 2)?;
    let w = |i: usize| u16::from_le_bytes([raw[i * 2], raw[i * 2 + 1]]);
    Some(Header {
        word_length: w(0),
        words: w(1),
        actions: w(2),
        items: w(3),
        messages: w(4),
        rooms: w(5),
        max_carry: w(6),
        start_room: w(7),
        treasures: w(8),
        lamp: w(9),
        treasure_room: w(10) >> 8,
    })
}

/// §12.4's sanity limits, all inclusive, plus the three §12.4's whole-image
/// scan adds. Failing any of them means "not this format" (§12.14).
fn header_plausible(h: &Header) -> bool {
    let field = |name: &str| match name {
        "items" => h.items,
        "actions" => h.actions,
        "words" => h.words,
        "rooms" => h.rooms,
        _ => h.messages,
    };
    COUNT_RANGES.iter().all(|&(name, lo, hi)| (lo..=hi).contains(&field(name)))
        && (MAX_CARRY_RANGE.0..=MAX_CARRY_RANGE.1).contains(&h.max_carry)
        && (WORD_LENGTH_RANGE.0..=WORD_LENGTH_RANGE.1).contains(&h.word_length)
        && h.start_room <= h.rooms
}

/// Where the dictionary begins: scan forward from the end of the header for
/// the literal bytes `ANY` (§12.5).
///
/// Measured, the scan walks **0 bytes** on the Commodore 64 *Hulk*, whose noun
/// block starts exactly at header + 29, and **179 bytes** on every Atari and
/// Apple release, whose dictionaries all begin at array offset 0x108.
fn find_dictionary(array: &[u8]) -> Option<usize> {
    let from = HEADER_AT + HEADER_CONSUMED;
    let hay = array.get(from..)?;
    hay.windows(DICTIONARY_ANCHOR.len())
        .position(|w| w == DICTIONARY_ANCHOR)
        .map(|at| from + at)
}

/// Is this what a container handed over a US S.A.G.A. database, read at
/// `platform`'s offset (§12.2)?
///
/// The cheap probe only: §12.2's front matter, §12.4's header limits, and the
/// presence of §12.5's `ANY`. It says nothing about whether the tables
/// themselves check out — [`parse_saga_us`] answers that, and a file passing
/// here and failing there is a *damaged* database, which is a different
/// report to a host than an unrecognised one.
pub fn looks_like_saga_us(file: &[u8], platform: SagaPlatform) -> bool {
    let Some(array) = file.get(platform.array_offset()..) else {
        return false;
    };
    front_matter(array).is_some_and(|(_, adventure)| adventure != 0)
        && read_header(array).is_some_and(|h| header_plausible(&h))
        && find_dictionary(array).is_some()
}

/// Which platform's offset, if any, makes these bytes a US S.A.G.A. database
/// (§12.2, §12.3).
///
/// Probes [`SagaPlatform::ALL`] in order and answers with the first that
/// passes [`looks_like_saga_us`]. Measured over the fifteen specimens plus the
/// twelve TI-99/4A images, the two *Mysterious Adventures* compilation disks,
/// the two Questprobe disk images and the reference `.dat` conversions,
/// **exactly one** platform ever answers for a file and none of the non-S.A.G.A.
/// files answers at all — so the ambiguity the three offsets could in
/// principle create does not arise on any known release.
pub fn detect_saga_us(file: &[u8]) -> Option<SagaPlatform> {
    SagaPlatform::ALL.into_iter().find(|&p| looks_like_saga_us(file, p))
}

// ── Table encodings (§12.5-12.9) ──────────────────────────────────────────────

/// Read `count` dictionary cells from `at`, returning them and the offset one
/// past the last byte consumed (§12.5).
///
/// **A cell is the word length in CHARACTERS**, not a fixed-width byte grid —
/// the difference from §4.2's memory-image tables, and the reason this reader
/// is its own. Two escapes consume an extra byte each: a NUL where a cell
/// should begin is skipped and the following byte taken as the first
/// character, and a `*` marking a synonym restarts the character count so that
/// the word after it still gets the full width. A NUL *inside* a cell is
/// ordinary padding and counts as one of the word length's characters, which
/// is what makes `55 50 00 00` read `UP` — two letters and two pads, the
/// second of which the next cell's escape absorbs.
///
/// **Each escape fires at most once per cell.** Letting the leading-NUL escape
/// repeat swallows a run of pad bytes that is really an empty dictionary entry,
/// and every entry after it is then one slot early: measured on *Pirate
/// Adventure*, a repeating escape loses the empty verb cells at indices 58, 62,
/// 65 and 66 and reads room text as vocabulary by index 65.
///
/// A byte above 127 terminates (§12.5). No cell in any of the fifteen
/// specimens contains one, and the specification does not say what a reader
/// should then do about the cell's alignment, so this refuses rather than
/// guessing.
fn read_cells(
    array: &[u8],
    at: usize,
    width: usize,
    count: usize,
) -> Result<(Vec<String>, usize), LoadError> {
    let mut out = Vec::with_capacity(count);
    let mut i = at;
    while out.len() < count {
        let mut chars: Vec<u8> = Vec::with_capacity(width);
        let mut synonym = false;
        let mut skipped = false;
        while chars.len() < width {
            let b = *array.get(i).ok_or_else(|| bad("the dictionary runs past the end"))?;
            i += 1;
            if b > 127 {
                return Err(bad("a dictionary cell holds a byte above 127"));
            }
            if b == 0 && chars.is_empty() && !synonym && !skipped {
                skipped = true;
                continue;
            }
            if b == b'*' && chars.is_empty() && !synonym {
                synonym = true;
                continue;
            }
            chars.push(b);
        }
        while chars.last() == Some(&0) {
            chars.pop();
        }
        let text: String = chars.into_iter().map(char::from).collect();
        out.push(if synonym { format!("*{text}") } else { text });
    }
    Ok((out, i))
}

/// Read `count` length-prefixed strings from `at`, returning each string with
/// the offset its length byte sat at, and the offset one past the last byte
/// consumed (§12.6).
///
/// One byte of length, then that many bytes of text, no terminator. A length
/// of 0 means the string [`EMPTY_STRING`] and consumes no further bytes. The
/// per-string offsets are what §12.7's pointer tables are checked against.
fn read_strings(
    array: &[u8],
    at: usize,
    count: usize,
    what: &'static str,
) -> Result<(Vec<(String, usize)>, usize), LoadError> {
    let mut out = Vec::with_capacity(count);
    let mut i = at;
    for _ in 0..count {
        let start = i;
        let len = usize::from(*array.get(i).ok_or_else(|| bad(what))?);
        i += 1;
        if len == 0 {
            out.push((EMPTY_STRING.to_string(), start));
            continue;
        }
        let raw = array.get(i..i + len).ok_or_else(|| bad(what))?;
        i += len;
        out.push((raw.iter().map(|&b| if b < 0x80 { b as char } else { '?' }).collect(), start));
    }
    Ok((out, i))
}

/// `count` little-endian words from `at`, and the offset one past them.
fn read_words(
    array: &[u8],
    at: usize,
    count: usize,
    what: &'static str,
) -> Result<(Vec<u16>, usize), LoadError> {
    let raw = array.get(at..at + count * 2).ok_or_else(|| bad(what))?;
    let words = raw.as_chunks::<2>().0.iter().copied().map(u16::from_le_bytes).collect();
    Ok((words, at + count * 2))
}

// ── The loader ────────────────────────────────────────────────────────────────

/// Read a US S.A.G.A. binary database into a [`Database`].
///
/// `file` is what the container handed over, unshifted — the whole ATR image,
/// the whole Apple II DOS 3.3 file, or the whole Commodore program file — and
/// `platform` supplies the one constant that turns it into the database array
/// (§12.3, [`SagaPlatform::array_offset`]). [`detect_saga_us`] answers the
/// platform for a caller that does not know it.
///
/// The whole array is read **sequentially** in §12.1's region order, which is
/// itself a check on the header: "on every specimen each region begins exactly
/// where the previous one ended". §12.7's three pointer tables are then
/// resolved against the string starts that read produced, which is §12.14's
/// own cheap check and the thing that catches a wrong array start.
///
/// # Errors
///
/// * [`LoadError::UnsupportedDialect`] with [`Dialect::SagaUsDatabase`] when
///   the bytes are not this format at all: an adventure number of zero, or a
///   header failing §12.4's limits, or no `ANY` anywhere past the header
///   (§12.14 — "hand the file to the next detector rather than reporting
///   corruption").
/// * [`LoadError::BadDialectData`] when they ARE this format and did not check
///   out: a table running past the end of the array, a pointer table that does
///   not resolve onto the strings already read, a connection pointing outside
///   the room table, a start room that indexes nothing.
///
/// # A damaged specimen
///
/// The Atari *Mission Impossible* side A carries about fifty corrupt bytes in
/// the middle of its room-description block, and its pointer tables therefore
/// do not resolve: this refuses it with [`LoadError::BadDialectData`], which is
/// what §12.14 asks for ("if they do not resolve onto the string starts already
/// read, stop"). The Apple II `A3.DAT` of the same title is clean. Recovering
/// the disk's second, undamaged copy of the database is a container matter and
/// is not attempted here.
pub fn parse_saga_us(file: &[u8], platform: SagaPlatform) -> Result<Database, LoadError> {
    let unsupported = || LoadError::UnsupportedDialect(Dialect::SagaUsDatabase);
    let array = file.get(platform.array_offset()..).ok_or_else(unsupported)?;

    // §12.2, §12.4: detection. Everything here is "not this format".
    let (version, adventure) = front_matter(array).ok_or_else(unsupported)?;
    if adventure == 0 {
        return Err(unsupported());
    }
    let header = read_header(array).ok_or_else(unsupported)?;
    if !header_plausible(&header) {
        return Err(unsupported());
    }
    let dictionary_at = find_dictionary(array).ok_or_else(unsupported)?;

    // §12.5: (words + 1) noun cells, THEN (words + 1) verb cells — the reverse
    // of §4.2's memory-image order, which is why §4.1's signature lands in the
    // middle of the block here rather than at its start.
    let cells = usize::from(header.words) + 1;
    let width = usize::from(header.word_length);
    let (nouns, after_nouns) = read_cells(array, dictionary_at, width, cells)?;
    let (verbs, rooms_at) = read_cells(array, after_nouns, width, cells)?;

    // §12.6: three blocks of length-prefixed strings, back to back.
    let room_count = usize::from(header.rooms) + 1;
    let message_count = usize::from(header.messages) + 1;
    let item_count = usize::from(header.items) + 1;
    let (room_texts, messages_at) =
        read_strings(array, rooms_at, room_count, "the room descriptions run past the end")?;
    let (message_texts, items_at) =
        read_strings(array, messages_at, message_count, "the messages run past the end")?;
    let (item_texts, after_items) = read_item_strings(array, items_at, item_count)?;

    // §12.7: one separator byte, then item locations, item-description
    // pointers, and an identical second copy of the locations.
    let mut cursor = after_items + 1;
    if array.get(after_items).is_none() {
        return Err(bad("the item block runs past the end"));
    }
    let (locations, next) =
        read_words(array, cursor, item_count, "the item locations run past the end")?;
    cursor = next;
    let (item_pointers, next) =
        read_words(array, cursor, item_count, "the item pointers run past the end")?;
    cursor = next;
    let (locations_again, next) =
        read_words(array, cursor, item_count, "the second item-location copy runs past the end")?;
    cursor = next;
    if locations_again != locations {
        return Err(bad("the two item-location tables disagree"));
    }

    // §12.8: the actions, column-major.
    let action_count = usize::from(header.actions) + 1;
    let action_span =
        action_count.checked_mul(ACTION_RECORD).ok_or_else(|| bad("the action count is absurd"))?;
    let actions = read_actions(array, cursor, action_count)?;
    cursor += action_span;

    // §12.7: the room-description pointers sit between the action table and
    // the connections.
    let (room_pointers, next) =
        read_words(array, cursor, room_count, "the room pointers run past the end")?;
    cursor = next;

    // §12.9: the connections, direction-major.
    let connections_at = cursor;
    let connection_span = room_count
        .checked_mul(2 * EXITS_PER_ROOM)
        .ok_or_else(|| bad("the room count is absurd"))?;
    if array.len() < connections_at + connection_span {
        return Err(bad("the room connections run past the end"));
    }
    cursor = connections_at + connection_span;

    // §12.7: the message pointers, in a 228-byte allocation.
    let (message_pointers, _) =
        read_words(array, cursor, message_count, "the message pointers run past the end")?;
    if array.len() < cursor + MESSAGE_POINTER_ALLOCATION {
        return Err(bad("the message-pointer allocation runs past the end"));
    }

    // §12.7: the base is DERIVABLE — subtract the room block's offset from the
    // first room pointer — and the three pointer tables then validate the
    // whole sequential read, entry for entry. §12.14 names exactly this as the
    // cheap check for a wrong array start.
    let base = i64::from(*room_pointers.first().ok_or_else(|| bad("no room pointers"))?)
        - rooms_at as i64;
    let resolves = |pointers: &[u16], starts: &[usize]| {
        pointers.iter().zip(starts).all(|(&p, &s)| i64::from(p) - base == s as i64)
    };
    let room_starts: Vec<usize> = room_texts.iter().map(|&(_, at)| at).collect();
    let item_starts: Vec<usize> = item_texts.iter().map(|r| r.start).collect();
    let message_starts: Vec<usize> = message_texts.iter().map(|&(_, at)| at).collect();
    if !resolves(&room_pointers, &room_starts)
        || !resolves(&item_pointers, &item_starts)
        || !resolves(&message_pointers, &message_starts)
    {
        return Err(bad("a pointer table does not resolve onto the strings read"));
    }

    // ── Into the reference format's own model ────────────────────────────────
    let mut rooms = Vec::with_capacity(room_count);
    for (index, (desc, _)) in room_texts.into_iter().enumerate() {
        let mut exits = [0usize; EXITS_PER_ROOM];
        for (direction, slot) in exits.iter_mut().enumerate() {
            let at = connections_at + 2 * room_count * direction + 2 * index;
            let exit = usize::from(array[at]);
            if exit >= room_count {
                return Err(bad("a room connection points outside the room table"));
            }
            *slot = exit;
        }
        // §12.6: "a leading `*` on a room description means print literally" —
        // the reference format's own convention, unchanged.
        let literal = desc.starts_with('*');
        rooms.push(Room {
            desc: desc.strip_prefix('*').unwrap_or(&desc).to_string(),
            literal,
            exits,
        });
    }

    let mut items = Vec::with_capacity(item_count);
    for (record, &location) in item_texts.into_iter().zip(&locations) {
        let mut text = record.text;
        // §12.6: the reference format's conventions carry over unchanged — a
        // leading `*` marks a treasure and is examined BEFORE the auto-noun
        // split (§2.5), and the auto-get word is §2.5's rule as written.
        let treasure = text.starts_with('*');
        let auto_noun = extract_auto_noun(&mut text);
        items.push(Item {
            text,
            treasure,
            auto_noun,
            start_loc: if location == STORED_CARRIED { CARRIED } else { i32::from(location) },
        });
    }

    let start_room = usize::from(header.start_room);
    if start_room >= rooms.len() {
        return Err(bad("the start room indexes no room"));
    }

    Ok(Database {
        max_carry: i32::from(header.max_carry),
        start_room,
        num_treasures: i32::from(header.treasures),
        word_length: usize::from(header.word_length),
        // Sign-extended the way `c64` does it, so a stored $FFFF would mean the
        // reference format's -1, "never runs out". No release here uses it —
        // the largest lamp in the fifteen is 15,000 — but the two loaders
        // should not read one word two ways.
        light_time: i32::from(header.lamp as i16),
        treasure_room: usize::from(header.treasure_room),
        actions,
        verbs,
        nouns,
        rooms,
        messages: message_texts.into_iter().map(|(text, _)| text).collect(),
        items,
        // §12.2's pair is the trailer's own adventure number, which a memory
        // image (§4.3) has nowhere to put and this format does.
        adventure_number: i32::from(adventure),
        ti99: None,
        // §12.11: "the two lamp options §9.2 describes are not forced. These
        // are Adventure International releases, not Mysterious Adventures
        // ones, and take the host's settings."
        mysterious: false,
        saga_us: Some(SagaUs { version, adventure, platform }),
    })
}

/// One item's description record as §12.6 frames it.
struct ItemRecord {
    text: String,
    /// Where the record's length byte sits — what §12.7's item-description
    /// pointers are checked against.
    start: usize,
}

/// Read the item descriptions (§12.6), which are the one string block whose
/// records are not all the same shape.
///
/// **An item whose description carries an auto-get word is followed by one
/// extra byte, outside the length prefix, and that byte is the item's own
/// index.** Measured on *Adventureland*, 31 of the 66 records carry it and 35
/// do not, the split falling exactly on whether the text contains a `/`; the
/// same holds on all fifteen databases, and §12.7's pointer table confirms the
/// framing independently. It is a back-reference, not a picture number —
/// nothing in this format associates an item with a picture (§12.10) — so it
/// is consumed and discarded. The specimen suite is where its value is pinned
/// against the index; making the parse itself refuse over it would buy no
/// coverage §12.7's pointers do not already give, at the price of refusing an
/// unknown release over a byte nothing reads.
fn read_item_strings(
    array: &[u8],
    at: usize,
    count: usize,
) -> Result<(Vec<ItemRecord>, usize), LoadError> {
    let overrun = "the item descriptions run past the end";
    let mut out = Vec::with_capacity(count);
    let mut i = at;
    for _ in 0..count {
        let start = i;
        let len = usize::from(*array.get(i).ok_or_else(|| bad(overrun))?);
        i += 1;
        let text: String = if len == 0 {
            EMPTY_STRING.to_string()
        } else {
            let raw = array.get(i..i + len).ok_or_else(|| bad(overrun))?;
            i += len;
            raw.iter().map(|&b| if b < 0x80 { b as char } else { '?' }).collect()
        };
        if text.contains('/') {
            array.get(i).ok_or_else(|| bad(overrun))?;
            i += 1;
        }
        out.push(ItemRecord { text, start });
    }
    Ok((out, i))
}

/// Read the action table, which is **transposed** (§12.8).
///
/// Instead of (actions + 1) records of sixteen bytes it is a fixed sequence of
/// columns, each column holding one field of every record: verb, noun, the
/// four command bytes, then the five conditions as little-endian words. With
/// *n* records and the table at *T*, record *k*'s verb is at *T* + *k*, its
/// noun at *T* + *n* + *k*, its command *c* at *T* + (2 + *c*)*n* + *k*, and
/// its condition *j* at *T* + 6*n* + 2*jn* + 2*k*. The whole table is 16*n*
/// bytes, exactly as row-major would be.
///
/// Reassembly into reference-format numbers is unchanged (§12.8): the
/// vocabulary word is verb x 150 + noun, the first command word is
/// 150 x (command 1) + (command 2) and the second 150 x (command 3) +
/// (command 4), and the condition words are already in reference form.
fn read_actions(array: &[u8], at: usize, count: usize) -> Result<Vec<Action>, LoadError> {
    let span = count * ACTION_RECORD;
    let table = array.get(at..at + span).ok_or_else(|| bad("the action table runs past the end"))?;
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        let column = |c: usize| u16::from(table[c * count + k]);
        let mut conditions = [Condition { code: 0, value: 0 }; 5];
        for (j, condition) in conditions.iter_mut().enumerate() {
            let off = 6 * count + 2 * j * count + 2 * k;
            let v = u16::from_le_bytes([table[off], table[off + 1]]);
            condition.code = (v % CONDITION_RADIX) as u8;
            condition.value = v / CONDITION_RADIX;
        }
        out.push(Action {
            verb: column(0),
            noun: column(1),
            conditions,
            commands: [column(2), column(3), column(4), column(5)],
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-built database in §12's own encoding: two words, three rooms,
    /// two messages, two items, two actions, on the Commodore 64 platform
    /// (array offset 0, so the bytes below ARE the array).
    ///
    /// Every offset is computed as the builder writes, so the fixture and the
    /// reader cannot silently agree about a wrong layout.
    struct Builder {
        bytes: Vec<u8>,
        base: u16,
    }

    impl Builder {
        fn new(base: u16) -> Builder {
            Builder { bytes: Vec::new(), base }
        }
        fn address(&self) -> u16 {
            self.base + self.bytes.len() as u16
        }
        fn byte(&mut self, b: u8) {
            self.bytes.push(b);
        }
        fn word(&mut self, w: u16) {
            self.bytes.extend_from_slice(&w.to_le_bytes());
        }
        /// A length-prefixed string, answering with the address of its length
        /// byte — what §12.7's pointer tables hold.
        fn string(&mut self, s: &str) -> u16 {
            let at = self.address();
            self.byte(s.len() as u8);
            self.bytes.extend_from_slice(s.as_bytes());
            at
        }
        /// One §12.5 dictionary cell at width `w`: the word's characters,
        /// NUL-padded to `w`, preceded by `*` for a synonym. The pad the next
        /// cell's leading-NUL escape absorbs is written by the caller.
        fn cell(&mut self, word: &str, w: usize) {
            let (synonym, text) = match word.strip_prefix('*') {
                Some(rest) => (true, rest),
                None => (false, word),
            };
            if synonym {
                self.byte(b'*');
            }
            for i in 0..w {
                self.byte(text.as_bytes().get(i).copied().unwrap_or(0));
            }
        }
    }

    /// Word length 3, words 50 (51 cells), rooms 10, items 10, messages 10,
    /// actions 100 — the smallest shape §12.4's limits will accept.
    const WORDS: u16 = 50;
    const ROOMS: u16 = 10;
    const ITEMS: u16 = 10;
    const MESSAGES: u16 = 10;
    const ACTIONS: u16 = 100;
    const WORD_LENGTH: u16 = 3;
    const BASE: u16 = 0x3000;

    fn noun(i: usize) -> String {
        match i {
            0 => "ANY".into(),
            1 => "NOR".into(),
            2 => "SOU".into(),
            3 => "LAM".into(),
            4 => "*LMP".into(),
            _ => format!("N{i:02}"),
        }
    }

    fn verb(i: usize) -> String {
        match i {
            0 => "AUT".into(),
            1 => "GO".into(),
            2 => "*RUN".into(),
            3 => "GET".into(),
            _ => format!("V{i:02}"),
        }
    }

    fn room_text(i: usize) -> String {
        match i {
            0 => String::new(), // the zero-length placeholder, which reads "."
            1 => "dismal swamp".into(),
            2 => "*Outside a hut".into(),
            _ => format!("room {i}"),
        }
    }

    fn message_text(i: usize) -> String {
        if i == 0 {
            String::new()
        } else {
            format!("message {i}")
        }
    }

    fn item_text(i: usize) -> String {
        match i {
            0 => String::new(),
            2 => "*Pot of RUBIES*/RUB/".into(),
            3 => "Brass lamp/LAMP/".into(),
            _ => format!("scenery {i}"),
        }
    }

    /// Build a complete, self-consistent database in §12's encoding.
    fn fixture() -> Vec<u8> {
        let cells = usize::from(WORDS) + 1;
        let room_count = usize::from(ROOMS) + 1;
        let item_count = usize::from(ITEMS) + 1;
        let message_count = usize::from(MESSAGES) + 1;
        let action_count = usize::from(ACTIONS) + 1;
        let width = usize::from(WORD_LENGTH);

        let mut b = Builder::new(BASE);
        // Region 1: front matter. Zeros, then the version/adventure pair at
        // 0x22 and 0x26 — where §12.2 measured it on the Atari and Apple
        // databases — with a zero between them the scan must step over.
        b.bytes.resize(0x22, 0);
        b.word(416);
        b.word(0xFFFF); // above 500: skipped
        b.word(7);
        b.bytes.resize(HEADER_AT, 0);
        // Region 2: the header, US field order.
        for w in [
            WORD_LENGTH,
            WORDS,
            ACTIONS,
            ITEMS,
            MESSAGES,
            ROOMS,
            6,    // max carried
            1,    // start room
            2,    // treasures
            125,  // lamp turns
            0x300, // treasure room 3 in the HIGH byte
            0,
            0,
            0,
            0,
        ] {
            b.word(w);
        }
        // Region 3: the gap. The header consumed 29 bytes, so one byte of the
        // fifteenth word is re-read; pad to a dictionary that starts later,
        // the way the Atari and Apple releases do.
        b.bytes.truncate(HEADER_AT + HEADER_CONSUMED);
        b.bytes.resize(HEADER_AT + HEADER_CONSUMED + 11, 0);
        // Region 4: nouns then verbs. Each cell writes width characters; the
        // pad byte between cells is the next cell's leading-NUL escape.
        for i in 0..cells {
            b.cell(&noun(i), width);
            if i + 1 < cells {
                b.byte(0);
            }
        }
        b.byte(0);
        for i in 0..cells {
            b.cell(&verb(i), width);
            if i + 1 < cells {
                b.byte(0);
            }
        }
        // Regions 5-7: the three string blocks.
        let room_pointers: Vec<u16> = (0..room_count).map(|i| b.string(&room_text(i))).collect();
        let message_pointers: Vec<u16> =
            (0..message_count).map(|i| b.string(&message_text(i))).collect();
        let mut item_pointers = Vec::with_capacity(item_count);
        for i in 0..item_count {
            let text = item_text(i);
            item_pointers.push(b.string(&text));
            if text.contains('/') {
                b.byte(i as u8); // §12.6's back-reference byte
            }
        }
        // Region 8: the separator.
        b.byte(0);
        // Regions 9-11: locations, item pointers, locations again.
        let locations: Vec<u16> =
            (0..item_count).map(|i| if i == 3 { 255 } else { (i % 4) as u16 }).collect();
        for &l in &locations {
            b.word(l);
        }
        for &p in &item_pointers {
            b.word(p);
        }
        for &l in &locations {
            b.word(l);
        }
        // Region 12: the actions, column-major.
        let verbs: Vec<u8> = (0..action_count).map(|k| (k % 5) as u8).collect();
        let nouns: Vec<u8> = (0..action_count).map(|k| (k % 7) as u8).collect();
        for &v in &verbs {
            b.byte(v);
        }
        for &n in &nouns {
            b.byte(n);
        }
        for c in 0..4u8 {
            for k in 0..action_count {
                b.byte(((k + usize::from(c)) % 11) as u8);
            }
        }
        for j in 0..5usize {
            for k in 0..action_count {
                b.word(((j * 7 + k) % 400) as u16);
            }
        }
        // Region 13: the room pointers.
        for &p in &room_pointers {
            b.word(p);
        }
        // Region 14: the connections, direction-major.
        for direction in 0..EXITS_PER_ROOM {
            for room in 0..room_count {
                b.word(((room + direction) % room_count) as u16);
            }
        }
        // Region 15: the message pointers, in a 228-byte allocation.
        let at = b.bytes.len();
        for &p in &message_pointers {
            b.word(p);
        }
        b.bytes.resize(at + MESSAGE_POINTER_ALLOCATION, 0);
        b.bytes
    }

    #[test]
    fn hand_built_database_decodes_to_the_expected_model() {
        let bytes = fixture();
        let db = parse_saga_us(&bytes, SagaPlatform::Commodore64).expect("parses");

        // The header, in §4.5's US field order (§12.4).
        assert_eq!(db.word_length, 3);
        assert_eq!(db.max_carry, 6);
        assert_eq!(db.start_room, 1);
        assert_eq!(db.num_treasures, 2);
        assert_eq!(db.light_time, 125);
        assert_eq!(db.treasure_room, 3, "the treasure room is word 10's HIGH byte");
        assert_eq!(db.adventure_number, 7);
        assert_eq!(
            db.saga_us,
            Some(SagaUs { version: 416, adventure: 7, platform: SagaPlatform::Commodore64 })
        );
        assert!(!db.mysterious, "§12.11: the two lamp options are NOT forced");
        assert!(db.ti99.is_none());

        // §12.5: nouns and verbs, with the `*` synonym convention preserved.
        assert_eq!(db.nouns.len(), usize::from(WORDS) + 1);
        assert_eq!(db.verbs.len(), usize::from(WORDS) + 1);
        assert_eq!(&db.nouns[..5], &["ANY", "NOR", "SOU", "LAM", "*LMP"]);
        assert_eq!(&db.verbs[..4], &["AUT", "GO", "*RUN", "GET"]);
        assert_eq!(db.match_noun("lmp"), Some(3), "a synonym resolves to its canonical index");

        // §12.6: the zero-length placeholder, the literal-`*` room convention,
        // the treasure marker and the auto-get noun.
        assert_eq!(db.rooms.len(), usize::from(ROOMS) + 1);
        assert_eq!(db.rooms[0].desc, ".");
        assert_eq!(db.rooms[1].desc, "dismal swamp");
        assert!(!db.rooms[1].literal);
        assert_eq!(db.rooms[2].desc, "Outside a hut");
        assert!(db.rooms[2].literal);
        assert_eq!(db.messages.len(), usize::from(MESSAGES) + 1);
        assert_eq!(db.messages[0], ".");
        assert_eq!(db.messages[1], "message 1");
        assert_eq!(db.items.len(), usize::from(ITEMS) + 1);
        assert_eq!(db.items[2].text, "*Pot of RUBIES*");
        assert!(db.items[2].treasure);
        assert_eq!(db.items[2].auto_noun.as_deref(), Some("RUB"));
        assert_eq!(db.items[3].auto_noun.as_deref(), Some("LAMP"));
        assert_eq!(db.items[3].start_loc, CARRIED, "255 means carried");
        assert_eq!(db.items[4].start_loc, 0);

        // §12.9: direction-major connections, north first.
        assert_eq!(db.rooms[0].exits, [0, 1, 2, 3, 4, 5]);
        assert_eq!(db.rooms[1].exits, [1, 2, 3, 4, 5, 6]);

        // §12.8: column-major actions, reassembled into reference numbers.
        assert_eq!(db.actions.len(), usize::from(ACTIONS) + 1);
        assert_eq!(db.actions[0].verb, 0);
        assert_eq!(db.actions[0].noun, 0);
        assert_eq!(db.actions[0].commands, [0, 1, 2, 3]);
        assert_eq!(db.actions[1].verb, 1);
        assert_eq!(db.actions[1].noun, 1);
        assert_eq!(db.actions[1].commands, [1, 2, 3, 4]);
        // Condition j of record k was written as (7j + k) % 400, and a stored
        // word is code + 20 x value.
        for (j, condition) in db.actions[3].conditions.iter().enumerate() {
            let stored = ((j * 7 + 3) % 400) as u16;
            assert_eq!(condition.code, (stored % 20) as u8);
            assert_eq!(condition.value, stored / 20);
        }
    }

    #[test]
    fn the_fixture_is_recognised_at_its_own_platform_and_nowhere_else() {
        let bytes = fixture();
        assert_eq!(detect_saga_us(&bytes), Some(SagaPlatform::Commodore64));
        assert!(looks_like_saga_us(&bytes, SagaPlatform::Commodore64));
        assert!(!looks_like_saga_us(&bytes, SagaPlatform::Atari8Bit));
        assert!(!looks_like_saga_us(&bytes, SagaPlatform::AppleII));
        // §12.3: the same array at an Atari container's offset means the whole
        // image, so prefixing the mastering constant's worth of bytes makes it
        // an Atari specimen and nothing else.
        let mut atari = vec![0u8; SagaPlatform::Atari8Bit.array_offset()];
        atari.extend_from_slice(&bytes);
        assert_eq!(detect_saga_us(&atari), Some(SagaPlatform::Atari8Bit));
    }

    #[test]
    fn the_header_is_consumed_as_twenty_nine_bytes_not_thirty() {
        // §12.4's worked case: a dictionary that begins EXACTLY at header + 29,
        // the way the Commodore 64 Hulk's does. A reader advancing a full
        // thirty bytes steps over the `A` of `ANY`.
        let mut bytes = fixture();
        let gap = HEADER_AT + HEADER_CONSUMED;
        let dictionary = find_dictionary(&bytes).expect("the fixture has a dictionary");
        bytes.drain(gap..dictionary);
        assert_eq!(find_dictionary(&bytes), Some(gap));
        let db = parse_saga_us(&bytes, SagaPlatform::Commodore64).expect("still parses");
        assert_eq!(&db.nouns[..3], &["ANY", "NOR", "SOU"]);
    }

    #[test]
    fn the_leading_nul_escape_fires_once_per_cell_so_empty_entries_survive() {
        // Two empty cells in a row, at width 3: the escape absorbs one pad
        // byte and the three that follow are the empty word itself. A reader
        // whose escape repeats swallows the run and reads the next word early.
        let bytes = [
            b'A', b'N', b'Y', 0, // ANY, then the next cell's escape pad
            0, 0, 0, 0, // an empty cell, and its own trailing pad
            b'U', b'P', 0, 0, // UP, padded, then the next cell's escape pad
            b'*', b'R', b'U', b'N',
        ];
        let (cells, end) = read_cells(&bytes, 0, 3, 4).expect("reads");
        assert_eq!(cells, ["ANY", "", "UP", "*RUN"]);
        assert_eq!(end, bytes.len());
    }

    #[test]
    fn a_zero_adventure_number_and_a_bad_header_are_not_this_format() {
        // §12.14: "hand the file to the next detector rather than reporting
        // corruption".
        let unsupported = LoadError::UnsupportedDialect(Dialect::SagaUsDatabase);

        let mut zeroed = fixture();
        zeroed[..HEADER_AT].fill(0);
        assert_eq!(parse_saga_us(&zeroed, SagaPlatform::Commodore64), Err(unsupported.clone()));
        assert!(!looks_like_saga_us(&zeroed, SagaPlatform::Commodore64));

        let mut bad_header = fixture();
        // Rooms = 0, outside §12.4's 10-100.
        bad_header[HEADER_AT + 10] = 0;
        bad_header[HEADER_AT + 11] = 0;
        assert_eq!(parse_saga_us(&bad_header, SagaPlatform::Commodore64), Err(unsupported.clone()));

        // §12.14: no `ANY` past the header is the same answer, and §4.1's
        // signatures are explicitly NOT a fallback.
        let mut no_anchor = fixture();
        let at = find_dictionary(&no_anchor).unwrap();
        no_anchor[at] = b'Z';
        // The next `ANY` in the fixture's own vocabulary would be a false
        // anchor, so blank the whole dictionary region's leading cell run.
        for b in no_anchor[at..at + 4].iter_mut() {
            *b = b'Z';
        }
        assert!(matches!(
            parse_saga_us(&no_anchor, SagaPlatform::Commodore64),
            Err(LoadError::UnsupportedDialect(_)) | Err(LoadError::BadDialectData(..))
        ));

        // Bytes far too short for even the front matter.
        assert_eq!(parse_saga_us(&[0u8; 4], SagaPlatform::Commodore64), Err(unsupported.clone()));
        assert_eq!(parse_saga_us(&[], SagaPlatform::Atari8Bit), Err(unsupported));
    }

    #[test]
    fn a_pointer_table_that_does_not_resolve_is_a_damaged_database() {
        // §12.14's cheap check: the three pointer tables must resolve onto the
        // string starts a sequential read produced. This is what refuses the
        // damaged Atari Mission Impossible specimen.
        let mut bytes = fixture();
        // Corrupt one room-description length byte, which shifts every string
        // start after it.
        let at = find_dictionary(&bytes).unwrap();
        let rooms_at = bytes[at..].windows(12).position(|w| w == b"dismal swamp").unwrap() + at - 1;
        bytes[rooms_at] = 11;
        assert!(matches!(
            parse_saga_us(&bytes, SagaPlatform::Commodore64),
            Err(LoadError::BadDialectData(Dialect::SagaUsDatabase, _))
        ));
    }

    #[test]
    fn truncation_anywhere_is_refused_and_never_panics() {
        let bytes = fixture();
        for cut in (1..bytes.len()).step_by(7) {
            let _ = parse_saga_us(&bytes[..cut], SagaPlatform::Commodore64);
            let _ = looks_like_saga_us(&bytes[..cut], SagaPlatform::Commodore64);
            let _ = detect_saga_us(&bytes[..cut]);
        }
    }

    #[test]
    fn two_hundred_seeded_byte_flips_are_refused_or_parsed_but_never_panic() {
        // A tiny xorshift so the corpus is reproducible without a dependency.
        let mut state: u32 = 0x5A6A_1234;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        let clean = fixture();
        let mut parsed = 0;
        for _ in 0..200 {
            let mut bytes = clean.clone();
            let flips = 1 + (next() % 4) as usize;
            for _ in 0..flips {
                let at = (next() as usize) % bytes.len();
                bytes[at] ^= 1 << (next() % 8);
            }
            match parse_saga_us(&bytes, SagaPlatform::Commodore64) {
                Ok(db) => {
                    parsed += 1;
                    // Whatever survived must still be internally consistent.
                    assert!(db.start_room < db.rooms.len());
                    assert!(db.rooms.iter().all(|r| r.exits.iter().all(|&e| e < db.rooms.len())));
                }
                Err(LoadError::BadDialectData(Dialect::SagaUsDatabase, _))
                | Err(LoadError::UnsupportedDialect(Dialect::SagaUsDatabase)) => {}
                Err(other) => panic!("unexpected error {other:?}"),
            }
        }
        // The corpus must exercise both outcomes, or it is not testing much:
        // most flips land in text or table bytes a parse accepts.
        assert!(parsed > 0 && parsed < 200, "flips produced {parsed} clean parses of 200");
    }

    #[test]
    fn the_hulk_remaps_room_pictures_on_two_platforms_of_three() {
        // §12.11's five pairs, and §12.2's "version 127 with adventure 1 is
        // the US Hulk".
        for platform in [SagaPlatform::Commodore64, SagaPlatform::Atari8Bit] {
            let hulk = SagaUs { version: 127, adventure: 1, platform };
            assert!(hulk.remaps_hulk_rooms());
            for (room, picture) in
                [(5, 3), (6, 3), (7, 4), (8, 4), (10, 9), (11, 9), (13, 2), (14, 2), (17, 16), (18, 16)]
            {
                assert_eq!(hulk.room_picture(room), picture, "room {room} on {platform:?}");
            }
            assert_eq!(hulk.room_picture(1), 1, "an unremapped room is its own picture");
            assert_eq!(hulk.room_picture(19), 19);
        }
        let apple = SagaUs { version: 127, adventure: 1, platform: SagaPlatform::AppleII };
        assert!(!apple.remaps_hulk_rooms());
        assert_eq!(apple.room_picture(5), 5);
        // Adventure 1 at version 416 is Adventureland, not the Hulk (§12.2).
        let land =
            SagaUs { version: 416, adventure: 1, platform: SagaPlatform::Commodore64 };
        assert!(!land.remaps_hulk_rooms());
        assert_eq!(land.room_picture(5), 5);
    }

    #[test]
    fn the_two_picture_constants_are_the_ones_section_12_11_names() {
        assert_eq!(DARKNESS_PICTURE, 0);
        assert_eq!(INVENTORY_PICTURE, 98);
    }

    /// §12.12's per-release table, checked against [`SagaUs::display_title`]
    /// (SQ-1470): every (version, adventure) §12.12 pins, on every platform it
    /// pins it for, names the title that table gives — and the title always
    /// ends with that platform's own [`SagaPlatform::label`], so the two
    /// cannot read differently.
    #[test]
    fn display_title_matches_the_per_release_table() {
        let cases: [(u16, u16, SagaPlatform, &str); 15] = [
            (416, 1, SagaPlatform::Atari8Bit, "Adventureland"),
            (416, 1, SagaPlatform::AppleII, "Adventureland"),
            (408, 2, SagaPlatform::Atari8Bit, "Pirate Adventure"),
            (408, 2, SagaPlatform::AppleII, "Pirate Adventure"),
            (306, 3, SagaPlatform::Atari8Bit, "Mission Impossible"),
            (306, 3, SagaPlatform::AppleII, "Mission Impossible"),
            (119, 4, SagaPlatform::Atari8Bit, "Voodoo Castle"),
            (119, 4, SagaPlatform::AppleII, "Voodoo Castle"),
            (115, 5, SagaPlatform::Atari8Bit, "The Count"),
            (115, 5, SagaPlatform::AppleII, "The Count"),
            (119, 6, SagaPlatform::Atari8Bit, "Strange Odyssey"),
            (119, 6, SagaPlatform::AppleII, "Strange Odyssey"),
            (125, 13, SagaPlatform::Atari8Bit, "The Sorcerer of Claymorgue Castle"),
            (122, 13, SagaPlatform::AppleII, "The Sorcerer of Claymorgue Castle"),
            (127, 1, SagaPlatform::Commodore64, "The Hulk"),
        ];
        for (version, adventure, platform, title) in cases {
            let saga = SagaUs { version, adventure, platform };
            let want = format!("{title} ({})", platform.label());
            assert_eq!(saga.display_title(), Some(want.as_str()), "{version}/{adventure}/{platform:?}");
        }
        // A pair §12.12 does not carry answers `None` rather than guessing —
        // e.g. Claymorgue's Atari version (125) paired with the Apple II
        // platform is not a release that exists.
        let unknown = SagaUs { version: 125, adventure: 13, platform: SagaPlatform::AppleII };
        assert_eq!(unknown.display_title(), None);
    }

    // §8.3's Commodore 64 picture-file rule and §8.6's usage convention, on
    // the names `QUESTPR1.D64` actually carries (§10.7, §12.10).
    #[test]
    fn picture_file_names_take_apart_per_8_3() {
        use PictureUsage::*;
        let cases = [
            ("R01000", Room, 0),  // the darkness picture
            ("R01001", Room, 1),
            ("R01012", Room, 12),
            ("R01098", Room, 98), // the inventory backdrop
            ("R01099", Room, 99), // the title picture
            ("B01013R", ObjectInRoom, 13),
            ("B01250R", ObjectInRoom, 250),
            ("B01001I", ObjectInInventory, 1),
            ("B01051I", ObjectInInventory, 51),
            ("S01007", Room, 7), // §8.6: a leading S defaults to a room
        ];
        for (name, usage, index) in cases {
            assert_eq!(parse_picture_file_name(name), Some(PictureFile { usage, index }), "{name}");
            assert!(is_picture_file_name(name), "{name}");
        }
        // The three-digit field starts at position 3, so the `01` series
        // number is part of neither field: R01000 is picture 0, not 10.
        assert_eq!(parse_picture_file_name("R01000").unwrap().index, 0);
        // A Commodore directory holds PETSCII; a host may have folded case.
        assert_eq!(parse_picture_file_name("r01001"), parse_picture_file_name("R01001"));
        assert_eq!(parse_picture_file_name("b01013r"), parse_picture_file_name("B01013R"));
    }

    #[test]
    fn non_picture_names_are_refused() {
        for name in [
            "SHULK.DB", // the database itself
            "SAGA.TED",
            "SAGA.C64",
            "THE HULK", // the BASIC loader
            "R01",      // too short for a three-digit field at 3-5
            "R0100",    // still too short
            "RABC000",  // positions 1-3 are not digits
            "Q01000",   // not R, B or S
            "B01013X",  // a B name with neither trailing R nor I
            "B01013",   // a B name with no usage letter at all
            "",
        ] {
            assert_eq!(parse_picture_file_name(name), None, "{name:?}");
            assert!(!is_picture_file_name(name), "{name:?}");
        }
    }

    #[test]
    fn room_picture_file_name_is_the_inverse_on_the_commodore_64() {
        assert_eq!(picture_file_name(SagaPlatform::Commodore64, 0).as_deref(), Some("R01000"));
        assert_eq!(picture_file_name(SagaPlatform::Commodore64, 99).as_deref(), Some("R01099"));
        for n in [0usize, 1, 12, 98, 99, 250, 999] {
            let name = picture_file_name(SagaPlatform::Commodore64, n).expect("names it");
            assert_eq!(
                parse_picture_file_name(&name),
                Some(PictureFile { usage: PictureUsage::Room, index: n as u16 }),
                "round trip for {n}"
            );
        }
        assert_eq!(picture_file_name(SagaPlatform::Commodore64, 1000), None, "no four-digit field");
        // Neither of the other two platforms finds its pictures by name: the
        // Atari's are at hard-coded offsets (§8.3) and the Apple II's are
        // family D (§8.4).
        assert_eq!(picture_file_name(SagaPlatform::Atari8Bit, 1), None);
        assert_eq!(picture_file_name(SagaPlatform::AppleII, 1), None);
    }

    // §12.11's five remapped pairs, and the platforms they apply on.
    #[test]
    fn the_hulk_room_picture_remap() {
        let hulk = SagaUs { version: 127, adventure: 1, platform: SagaPlatform::Commodore64 };
        for (room, want) in
            [(5, 3), (6, 3), (7, 4), (8, 4), (10, 9), (11, 9), (13, 2), (14, 2), (17, 16), (18, 16)]
        {
            assert_eq!(hulk.room_picture(room), want, "room {room}");
        }
        for room in [0usize, 1, 2, 3, 4, 9, 12, 15, 16, 19, 20] {
            assert_eq!(hulk.room_picture(room), room, "room {room} is its own picture");
        }
        assert!(hulk.remaps_hulk_rooms());
        let atari = SagaUs { platform: SagaPlatform::Atari8Bit, ..hulk };
        assert_eq!(atari.room_picture(5), 3, "the Atari release remaps too");
        // Not on the Apple II, and not for any other title.
        let apple = SagaUs { platform: SagaPlatform::AppleII, ..hulk };
        assert_eq!(apple.room_picture(5), 5);
        let adventureland =
            SagaUs { version: 416, adventure: 1, platform: SagaPlatform::Commodore64 };
        assert_eq!(adventureland.room_picture(5), 5);
    }
}
