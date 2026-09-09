//! **Apple II DOS 3.3 volumes** — the 5.25-inch filesystem that sits under a
//! flat `.dsk` / `.do` sector dump (SQ-1458).
//!
//! # Sources
//!
//! Public format documentation only:
//!
//! * *Beneath Apple DOS*, Don Worth and Pieter Lechner (Quality Software, 1981),
//!   whose chapter 4 is the canonical layout of the VTOC, the catalogue and the
//!   track/sector list — long out of print and freely published as a scan.
//! * The *Apple II DOS 3.3 Manual* (Apple Computer, 1980) for the file types and
//!   the binary file's load-address prologue.
//! * This repository's own clean-room summary,
//!   [`docs/internals/scott-dialects-spec.md`](../../../docs/internals/scott-dialects-spec.md)
//!   §7.4, which states the same structures functionally, together with the
//!   filename normalisation rule quoted below.
//!
//! **No GPL interpreter source was read for this module**, per
//! `docs/internals/clean-room.md`.
//!
//! # The medium, and its relationship to the two Apple II rows already here
//!
//! A 5.25-inch Apple II disk is 35 tracks of 16 256-byte sectors — **143,360
//! bytes** — and this crate already reads two different things wearing that
//! shape and that `.dsk` spelling: a **ProDOS volume** whose blocks have been
//! shuffled by the drive's numbering ([`crate::prodos`], via
//! [`crate::dos_order`]), and a **raw self-booting Infocom disk** with no
//! filesystem at all ([`crate::infocom_boot`]). This is the third, and the one
//! the spelling is actually named after: an ordinary DOS 3.3 volume, with a
//! catalogue and files in it.
//!
//! **No de-interleave is needed for a `.dsk`, and that is the definition of the
//! ordering rather than a convenience.** §7.4 says it outright: *"when reading a
//! flat DOS-ordered image the logical number **is** the file index"*. So track
//! *t* sector *s* is at `t × 4096 + s × 256`, full stop —
//! [`crate::dos_order::prodos_order`] exists to hand those same bytes to the
//! *ProDOS* reader, and is not wanted here.
//!
//! # A ProDOS-ordered DOS 3.3 disk is REFUSED, and this is why
//!
//! §7.4 lists it among its three important negatives: *"**ProDOS sector
//! ordering** is indistinguishable from DOS 3.3 ordering by content, and a
//! reader assuming DOS 3.3 will parse a ProDOS-ordered image structurally and
//! yield wrong data."* That was written to be believed, and this module tried it
//! anyway — a "cheap" second arm that flipped the image with
//! [`crate::dos_order`] whenever the first test failed. **It cannot work, and the
//! reason is arithmetic on the interleave table itself**: sectors 0 and 15 are
//! its only fixed points (`SECTOR_OF[0] = 0`, `SECTOR_OF[15] = 15`), so **track
//! 17 sector 0 and track 17 sector 15 are in the same place in both orderings**
//! — which is exactly the VTOC and the first catalogue sector. A
//! ProDOS-ordered DOS 3.3 disk therefore passes every test below, names its
//! files correctly, and then reads each one's track/sector list out of the wrong
//! sector: the arm found a volume and handed back rubbish, which is worse than
//! refusing it.
//!
//! Telling them apart needs something the corpus does not supply — no image in
//! `stories/` is a ProDOS-ordered DOS 3.3 disk, so there is nothing to measure a
//! discriminator against, and a heuristic scored on synthetic disks would be a
//! guess dressed as a reader. So this module claims the ordering it can be
//! checked on, and `.po` stays [`crate::medium::DiskImage::ProDos`]'s spelling.
//!
//! **`.woz` is not read**, and is named rather than silently mishandled: a
//! bit-preserving image is a GCR bitstream that has to be nibble-decoded before
//! there are sectors at all (§7.4 gives the whole procedure), which is the same
//! shape of work [`crate::g64`] does for the Commodore and is a quest of its
//! own. Nothing in `stories/` is one — the fourteen Apple sides in
//! `stories/scott-dialects/apple/` are all flat 143,360-byte dumps and the WOZ
//! signature is absent from every one of them.
//!
//! # The structures
//!
//! **Volume table of contents**, track 17 sector 0:
//!
//! ```text
//!   +0x01  track of the first catalogue sector      (17 on every disk here)
//!   +0x02  its sector                               (15)
//!   +0x03  the DOS release that INITed the disk     (3)
//!   +0x06  the volume number                        (1..254)
//!   +0x27  maximum track/sector pairs in a list     (122)
//!   +0x34  tracks per disk                          (35)
//!   +0x35  sectors per track                        (16)
//!   +0x36  bytes per sector, little-endian          (256)
//!   +0x38  the free-sector bitmap, four bytes a track
//! ```
//!
//! **Catalogue sectors** chain through `+0x01`/`+0x02` — a next-track of 0 ends
//! it — and carry **seven 35-byte entries from `+0x0B`**:
//!
//! ```text
//!   +0x00  track of the file's track/sector list; 0x00 never used, 0xFF deleted
//!   +0x01  its sector
//!   +0x02  file type, with bit 7 meaning locked
//!   +0x03  a 30-byte filename in high ASCII, space-padded
//!   +0x21  the file's length IN SECTORS, little-endian
//! ```
//!
//! **Track/sector lists** chain the same way and hold **up to 122 pairs from
//! `+0x0C`, track byte first**. A pair of `(0, 0)` is a *sparse* sector: the file
//! holds 256 zero bytes there and no sector is read. If another list follows,
//! all 122 pairs count including trailing zeros; if this is the last, the run
//! ends at the last non-zero pair.
//!
//! **The sector count in a catalogue entry includes the file's own list
//! sectors**, which is why nothing here uses it to size a read: the file's data
//! is the concatenation of the sectors its pairs name, and that comes out one
//! sector shorter than the count on every ordinary file. `A1.DAT` is 42 sectors
//! and 10,496 bytes — 41 × 256 — on every Apple side in the corpus.
//!
//! # Filename normalisation
//!
//! §7.4's rule, applied per byte before comparison and quoted rather than
//! recalled: *"if bit 7 is set and the byte is 0xA0 or above, clear bit 7; if bit
//! 7 is set and the byte is below 0xA0, clear bit 7 and add 0x20; if bit 7 is
//! clear, mask to the low six bits, exclusive-OR with 0x20, then add 0x20"* —
//! then strip trailing spaces. Its worked example: stored `C4 C1 D4 C1 A0 A0`
//! becomes `DATA`. That is this module's `normalise`, and it is the only
//! spelling of a name
//! this module ever reports or matches.
//!
//! # A binary file's four-byte prologue
//!
//! A type-`B` file opens with its **load address** and **length**, both
//! little-endian, and its data from byte 4. Both spellings are offered:
//! [`Dos33::read`] hands back the file exactly as DOS stores it — prologue
//! included, padded out to whole sectors — and [`Dos33::read_binary`] hands back
//! the address with the declared length of data behind it. `A1.DAT` is 10,496
//! stored bytes, and `$4000` with 10,335 behind it.
//!
//! **`read` is the raw one because the mastering constants index the raw array,
//! and that is measured, not assumed.** §7.4 gives the S.A.G.A. Apple database
//! offset as `0x135` with the game header proper at `0x016D`, and does not say
//! which array. On `A1.DAT` off Adventureland's boot side, ten little-endian
//! words at **raw** `0x016D` read `3, 69, 169, 65, 75, 33, 6, 11, 13, 125` —
//! Adventureland's word length and table counts, exactly as the specimen README
//! records them — while the same offset into the *stripped* array reads
//! `169, 65, 75, …`, four bytes late. So a loader reaching through
//! [`crate::medium::MountedDisk::read_named`] gets the array those constants are
//! written against, and needs to know nothing about prologues.

use crate::infocom_pics::InfocomPics;

/// Tracks on a 5.25-inch DOS 3.3 disk.
pub const TRACKS: usize = 35;

/// Sectors per track.
pub const SECTORS: usize = 16;

/// Bytes per sector.
pub const SECTOR: usize = 256;

/// The one length a flat 5.25-inch dump has: `35 × 16 × 256`.
pub const IMAGE_LEN: usize = TRACKS * SECTORS * SECTOR;

/// The track the VTOC lives on.
const VTOC_TRACK: usize = 17;

/// Maximum track/sector pairs a list sector holds, and what the VTOC declares
/// at `+0x27`.
const MAX_PAIRS: usize = 122;

/// Where a list sector's pairs begin.
const FIRST_PAIR: usize = 0x0c;

/// Where a catalogue sector's entries begin.
const FIRST_ENTRY: usize = 0x0b;

/// Bytes in one catalogue entry.
const ENTRY: usize = 35;

/// Entries per catalogue sector.
const ENTRIES: usize = 7;

/// A catalogue-entry track byte of this means the file was deleted.
const DELETED: u8 = 0xff;

/// The most catalogue sectors a walk will follow, and the most list sectors a
/// file's chain will — §7.4's own ceilings, so a corrupt image cannot spin.
const MAX_CATALOGUE_SECTORS: usize = 64;
const MAX_LIST_SECTORS: usize = 32;

/// What a DOS 3.3 catalogue says a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    /// `T` — text.
    Text,
    /// `I` — Integer BASIC.
    IntegerBasic,
    /// `A` — Applesoft BASIC.
    Applesoft,
    /// `B` — binary, and the only type with a load address in front of it.
    Binary,
    /// `S`, `R`, `a` and `b`: the four types DOS defines and nothing in this
    /// corpus uses, kept as one variant because nothing here tells them apart.
    Other(u8),
}

impl FileType {
    /// The letter DOS's own `CATALOG` prints.
    pub fn letter(self) -> char {
        match self {
            FileType::Text => 'T',
            FileType::IntegerBasic => 'I',
            FileType::Applesoft => 'A',
            FileType::Binary => 'B',
            FileType::Other(0x08) => 'S',
            FileType::Other(0x10) => 'R',
            FileType::Other(0x20) => 'a',
            FileType::Other(_) => 'b',
        }
    }

    /// From a catalogue entry's type byte, with the lock bit already off.
    fn of(byte: u8) -> FileType {
        match byte & 0x7f {
            0x00 => FileType::Text,
            0x01 => FileType::IntegerBasic,
            0x02 => FileType::Applesoft,
            0x04 => FileType::Binary,
            other => FileType::Other(other),
        }
    }
}

/// Errors that can arise while mounting a DOS 3.3 volume.
#[derive(Debug, PartialEq, Eq)]
pub enum Dos33Error {
    /// Not a DOS 3.3 volume this reader recognises — the wrong length, or no
    /// sane VTOC and catalogue at the DOS 3.3 sector order.
    NotDos33,
}

/// One file in the catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dos33Entry {
    /// The name, normalised by §7.4's per-byte rule and trimmed — `A1.DAT`,
    /// `DATABASE`, `PAK.INVEN`, `RESET APPLESOFT$7801`.
    pub name: String,
    /// What DOS says it is.
    pub file_type: FileType,
    /// Whether the catalogue's lock bit is set.
    pub locked: bool,
    /// The catalogue's own length **in sectors**, which INCLUDES the file's
    /// track/sector list sectors. See the module docs; it is reported because
    /// `CATALOG` reports it, never used to size a read.
    pub sectors: usize,
    /// Bytes the chain yields — `sectors` less its list sectors, times 256.
    pub size: usize,
    /// The first track/sector list sector.
    list: (usize, usize),
}

/// A mounted Apple II DOS 3.3 volume.
#[derive(Debug)]
pub struct Dos33 {
    /// Sectors in DOS 3.3 order, whatever ordering arrived.
    image: Vec<u8>,
    volume: u8,
    files: Vec<Dos33Entry>,
}

/// Is `raw` an Apple II DOS 3.3 volume?
///
/// A sane VTOC **and** a catalogue that walks and names at least one file. Both
/// halves are load-bearing on this corpus:
///
/// * The VTOC alone is not enough, because three of the fourteen Apple sides in
///   `stories/scott-dialects/apple/` — the side A of the scrambled titles — have
///   game data where a VTOC should be and would otherwise read as volumes with
///   nonsense in them.
/// * The catalogue alone is not enough either, because seven bytes of plausible
///   chain link is not a filesystem.
///
/// **This is what keeps the three Apple II rows disjoint.** Every ProDOS `.dsk`
/// and the one raw self-booting disk in `stories/` fails the VTOC test outright
/// — measured: not one of the fifteen has 35/16/256/122 in the four fields at
/// `+0x34`, `+0x35`, `+0x36` and `+0x27` — so the order the rows sit in
/// `crate::medium`'s one format table stays a formality, exactly as SQ-0868
/// arranged for
/// the pair before this one.
///
/// **A ProDOS-ordered image is not claimed** — see the module docs for the
/// interleave arithmetic that makes it unrecognisable rather than merely
/// unimplemented, and for why the arm that tried was removed.
pub fn looks_like_dos33(raw: &[u8]) -> bool {
    raw.len() == IMAGE_LEN && volume_is_sane(raw)
}

/// Does `raw` open a VTOC and a catalogue that names something?
///
/// The one place the medium is recognised — [`looks_like_dos33`] and
/// [`Dos33::mount`] both come through here, so the two cannot drift apart.
fn volume_is_sane(raw: &[u8]) -> bool {
    let vtoc = &raw[VTOC_TRACK * SECTORS * SECTOR..][..SECTOR];
    let bytes_per_sector = usize::from(u16::from_le_bytes([vtoc[0x36], vtoc[0x37]]));
    if usize::from(vtoc[0x34]) != TRACKS
        || usize::from(vtoc[0x35]) != SECTORS
        || bytes_per_sector != SECTOR
        || usize::from(vtoc[0x27]) != MAX_PAIRS
    {
        return false;
    }
    // §7.4: "A first-catalogue track of 35 or more, or sector of 16 or more,
    // means this is not such a disk."
    let (track, sector) = (usize::from(vtoc[0x01]), usize::from(vtoc[0x02]));
    if track >= TRACKS || sector >= SECTORS {
        return false;
    }
    !catalogue(raw, track, sector).is_empty()
}

/// Walk the catalogue chain from `(track, sector)` and return every entry that
/// is one, in catalogue order.
fn catalogue(raw: &[u8], mut track: usize, mut sector: usize) -> Vec<Dos33Entry> {
    let mut out = Vec::new();
    let mut seen: Vec<(usize, usize)> = Vec::new();
    while track != 0 {
        if track >= TRACKS || sector >= SECTORS || seen.contains(&(track, sector)) {
            break;
        }
        seen.push((track, sector));
        if seen.len() > MAX_CATALOGUE_SECTORS {
            break;
        }
        let block = &raw[(track * SECTORS + sector) * SECTOR..][..SECTOR];
        for slot in 0..ENTRIES {
            let e = &block[FIRST_ENTRY + slot * ENTRY..][..ENTRY];
            // 0x00 never used, 0xFF deleted — neither names a live file.
            if e[0] == 0 || e[0] == DELETED {
                continue;
            }
            let (list_track, list_sector) = (usize::from(e[0]), usize::from(e[1]));
            if list_track >= TRACKS || list_sector >= SECTORS {
                continue;
            }
            let Some(name) = normalise(&e[3..0x21]) else { continue };
            let pairs = track_sector_list(raw, list_track, list_sector);
            out.push(Dos33Entry {
                name,
                file_type: FileType::of(e[2]),
                locked: e[2] & 0x80 != 0,
                sectors: usize::from(u16::from_le_bytes([e[0x21], e[0x22]])),
                size: pairs.len() * SECTOR,
                list: (list_track, list_sector),
            });
        }
        (track, sector) = (usize::from(block[1]), usize::from(block[2]));
    }
    out
}

/// The track/sector pairs a file's list chain names, in file order.
///
/// A `(0, 0)` pair is kept: it is a sparse sector, 256 zero bytes the disk does
/// not store. §7.4's two rules about trailing zeros are the whole of the
/// subtlety — a list with another behind it keeps all 122 pairs, and the last
/// one ends at its last non-zero pair.
fn track_sector_list(raw: &[u8], mut track: usize, mut sector: usize) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    let mut seen: Vec<(usize, usize)> = Vec::new();
    while track != 0 {
        if track >= TRACKS || sector >= SECTORS || seen.contains(&(track, sector)) {
            break;
        }
        seen.push((track, sector));
        if seen.len() > MAX_LIST_SECTORS {
            break;
        }
        let block = &raw[(track * SECTORS + sector) * SECTOR..][..SECTOR];
        let mut here: Vec<(usize, usize)> = (0..MAX_PAIRS)
            .map(|i| (usize::from(block[FIRST_PAIR + 2 * i]), usize::from(block[FIRST_PAIR + 2 * i + 1])))
            .collect();
        let (next_track, next_sector) = (usize::from(block[1]), usize::from(block[2]));
        if next_track == 0 {
            while here.last() == Some(&(0, 0)) {
                here.pop();
            }
        }
        pairs.extend(here);
        (track, sector) = (next_track, next_sector);
    }
    pairs
}

impl Dos33 {
    /// Cheap sniff — see [`looks_like_dos33`].
    pub fn looks_like_dos33(raw: &[u8]) -> bool {
        looks_like_dos33(raw)
    }

    /// Open the volume and read its catalogue.
    pub fn mount(raw: Vec<u8>) -> Result<Dos33, Dos33Error> {
        if !looks_like_dos33(&raw) {
            return Err(Dos33Error::NotDos33);
        }
        let image = raw;
        let vtoc = &image[VTOC_TRACK * SECTORS * SECTOR..][..SECTOR];
        let (volume, track, sector) = (vtoc[0x06], usize::from(vtoc[0x01]), usize::from(vtoc[0x02]));
        let files = catalogue(&image, track, sector);
        Ok(Dos33 { image, volume, files })
    }

    /// The volume number the VTOC carries at `+0x06`, `1..=254` by convention
    /// and `254` on every Apple side in the corpus.
    ///
    /// **Not a volume NAME** — DOS 3.3 has none, which is why
    /// [`crate::medium`]'s `volume_name` answers `None` for this format. A
    /// number is not a name to splice into a sentence.
    pub fn volume_number(&self) -> u8 {
        self.volume
    }

    /// Every file in the catalogue, in catalogue order.
    pub fn files(&self) -> &[Dos33Entry] {
        &self.files
    }

    /// One sector by track and sector, in DOS 3.3 numbering.
    pub fn sector(&self, track: usize, sector: usize) -> Option<&[u8]> {
        if track >= TRACKS || sector >= SECTORS {
            return None;
        }
        Some(&self.image[(track * SECTORS + sector) * SECTOR..][..SECTOR])
    }

    /// A file's bytes **exactly as DOS stores them** — whole sectors, and for a
    /// type-`B` file the four-byte load prologue still in front.
    pub fn read(&self, entry: &Dos33Entry) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(entry.size);
        for (track, sector) in track_sector_list(&self.image, entry.list.0, entry.list.1) {
            match self.sector(track, sector) {
                // A sparse pair: the file holds 256 zero bytes the disk does not.
                _ if (track, sector) == (0, 0) => out.extend(std::iter::repeat_n(0u8, SECTOR)),
                Some(block) => out.extend_from_slice(block),
                None => return None,
            }
        }
        Some(out)
    }

    /// A **binary** file's load address and its declared bytes, with the
    /// four-byte prologue removed — or `None` when the entry is not type `B`, or
    /// the length it declares does not fit what the chain yielded.
    ///
    /// The other half of [`Dos33::read`]; the module docs say which a caller
    /// wants. `A1.DAT` answers `($4000, 10_335 bytes)` out of 10,496 stored.
    pub fn read_binary(&self, entry: &Dos33Entry) -> Option<(u16, Vec<u8>)> {
        if entry.file_type != FileType::Binary {
            return None;
        }
        let raw = self.read(entry)?;
        if raw.len() < 4 {
            return None;
        }
        let address = u16::from_le_bytes([raw[0], raw[1]]);
        let len = usize::from(u16::from_le_bytes([raw[2], raw[3]]));
        (4 + len <= raw.len()).then(|| (address, raw[4..4 + len].to_vec()))
    }

    /// Every file that reads, in catalogue order — raw, per [`Dos33::read`].
    pub fn contents(&self) -> Vec<(String, Vec<u8>)> {
        self.files.iter().filter_map(|e| self.read(e).map(|b| (e.name.clone(), b))).collect()
    }

    /// One file by its normalised name, case-insensitively.
    pub fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        let e = self.files.iter().find(|e| e.name.eq_ignore_ascii_case(name))?;
        self.read(e)
    }

    /// The Z-machine story on this volume, if one of its files is one.
    ///
    /// **Nothing in the corpus is.** Infocom's Apple II presses in `stories/`
    /// are ProDOS volumes and one raw self-booting disk, neither of which is
    /// this format; the fourteen sides here are Scott Adams releases whose
    /// database is not Z-code. The question is answered because every format
    /// answers it, through this crate's one test for what a story is.
    pub fn story(&self) -> Option<(String, Vec<u8>)> {
        self.contents().into_iter().find(|(_, b)| crate::adf::looks_like_story(b))
    }

    /// **No Infocom picture archive**, on the same ground [`crate::d64`] and
    /// [`crate::atr`] state: no Version 6 game was pressed for a DOS 3.3 Apple
    /// II, so there is no evidence about where one would keep an archive. The
    /// artwork these Scott Adams disks *do* carry is a different format
    /// entirely (`scott-dialects-spec.md` §8.4) and is not this reader's.
    pub fn pictures(&self) -> Option<(String, InfocomPics)> {
        None
    }
}

/// §7.4's filename normalisation, applied per byte, then trailing spaces
/// stripped. `None` only when nothing is left.
///
/// **The mapping cannot fail**: each of its three cases lands inside printable
/// ASCII whatever byte goes in, so a NUL comes out as `@` and a graphics
/// character comes out as something. The bound below is a guard against a
/// future edit, not a filter — what tells a catalogue entry from game data
/// written over one is [`volume_is_sane`]'s VTOC test and the entry's own
/// track/sector bounds.
///
/// The spec's worked example is the test below: `C4 C1 D4 C1 A0 A0` → `DATA`.
fn normalise(field: &[u8]) -> Option<String> {
    let mut out = String::with_capacity(field.len());
    for &b in field {
        let c = if b & 0x80 != 0 {
            if b >= 0xa0 { b & 0x7f } else { (b & 0x7f) + 0x20 }
        } else {
            ((b & 0x3f) ^ 0x20) + 0x20
        };
        if !(0x20..0x7f).contains(&c) {
            return None;
        }
        out.push(char::from(c));
    }
    let name = out.trim_end().to_string();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a DOS 3.3 volume from the format documentation, so this reader's
    /// tests need no fixture.
    ///
    /// `pub(crate)` for [`crate::medium`]'s census.
    pub(crate) struct DiskBuilder {
        image: Vec<u8>,
        /// The next free sector, as a linear index. Track 0 is the boot image,
        /// tracks 1 and 2 are DOS, track 17 the directory — data starts at 3.
        next: usize,
        used: usize,
    }

    /// The catalogue chain a freshly INITed disk has: 17/15 down to 17/1.
    const CATALOGUE: [(usize, usize); 15] = [
        (17, 15), (17, 14), (17, 13), (17, 12), (17, 11), (17, 10), (17, 9), (17, 8),
        (17, 7), (17, 6), (17, 5), (17, 4), (17, 3), (17, 2), (17, 1),
    ];

    impl DiskBuilder {
        pub(crate) fn new() -> DiskBuilder {
            let mut image = vec![0u8; IMAGE_LEN];
            let vtoc = VTOC_TRACK * SECTORS * SECTOR;
            image[vtoc] = 4;
            image[vtoc + 0x01] = CATALOGUE[0].0 as u8;
            image[vtoc + 0x02] = CATALOGUE[0].1 as u8;
            image[vtoc + 0x03] = 3;
            image[vtoc + 0x06] = 254;
            image[vtoc + 0x27] = MAX_PAIRS as u8;
            image[vtoc + 0x34] = TRACKS as u8;
            image[vtoc + 0x35] = SECTORS as u8;
            image[vtoc + 0x36..vtoc + 0x38].copy_from_slice(&(SECTOR as u16).to_le_bytes());
            // The catalogue chain, 17/15 down to 17/1 and then nothing.
            for pair in CATALOGUE.windows(2) {
                let at = (pair[0].0 * SECTORS + pair[0].1) * SECTOR;
                image[at + 1] = pair[1].0 as u8;
                image[at + 2] = pair[1].1 as u8;
            }
            DiskBuilder { image, next: 3 * SECTORS, used: 0 }
        }

        fn at(&self, track: usize, sector: usize) -> usize {
            (track * SECTORS + sector) * SECTOR
        }

        fn take(&mut self) -> (usize, usize) {
            let (track, sector) = (self.next / SECTORS, self.next % SECTORS);
            self.next += 1;
            assert!(track < TRACKS && track != VTOC_TRACK, "the sample disk ran out of room");
            (track, sector)
        }

        /// A file with a real track/sector list and a real catalogue entry.
        pub(crate) fn add_file(&mut self, name: &str, kind: u8, data: &[u8]) {
            let (list_track, list_sector) = self.take();
            let chunks: Vec<&[u8]> =
                if data.is_empty() { vec![&[]] } else { data.chunks(SECTOR).collect() };
            assert!(chunks.len() <= MAX_PAIRS, "the sample builder writes one list sector");
            let list = self.at(list_track, list_sector);
            for (i, chunk) in chunks.iter().enumerate() {
                let (track, sector) = self.take();
                let at = self.at(track, sector);
                self.image[at..at + chunk.len()].copy_from_slice(chunk);
                self.image[list + FIRST_PAIR + 2 * i] = track as u8;
                self.image[list + FIRST_PAIR + 2 * i + 1] = sector as u8;
            }
            // The catalogue's own count includes this list sector — the fact
            // §7.4 warns about and the reader deliberately does not use.
            let sectors = chunks.len() + 1;
            let slot = self.used;
            self.used += 1;
            let (cat_track, cat_sector) = CATALOGUE[slot / ENTRIES];
            let e = self.at(cat_track, cat_sector) + FIRST_ENTRY + (slot % ENTRIES) * ENTRY;
            self.image[e] = list_track as u8;
            self.image[e + 1] = list_sector as u8;
            self.image[e + 2] = kind;
            // High ASCII, space-padded, which is how DOS stores a name.
            self.image[e + 3..e + 0x21].fill(0xa0);
            for (i, c) in name.bytes().enumerate() {
                self.image[e + 3 + i] = c | 0x80;
            }
            self.image[e + 0x21..e + 0x23].copy_from_slice(&(sectors as u16).to_le_bytes());
        }

        pub(crate) fn finish(self) -> Vec<u8> {
            self.image
        }
    }

    /// A mountable DOS 3.3 volume carrying `files` — [`crate::medium`]'s census
    /// sample. Everything is stored as a text file, so no prologue is implied.
    pub(crate) fn sample_disk(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut disk = DiskBuilder::new();
        for (name, bytes) in files {
            disk.add_file(name, 0x00, bytes);
        }
        disk.finish()
    }

    /// §7.4's own worked example, and the shapes around it.
    #[test]
    fn the_filename_rule_is_the_specs_worked_example() {
        assert_eq!(normalise(&[0xc4, 0xc1, 0xd4, 0xc1, 0xa0, 0xa0]).as_deref(), Some("DATA"));
        // High ASCII below 0xA0 gets 0x20 added: 0x81 is 0x01 + 0x20 = '!'.
        assert_eq!(normalise(&[0x81]).as_deref(), Some("!"));
        // Bit 7 clear is the inverse-video/flashing encoding.
        assert_eq!(normalise(&[0x04]).as_deref(), Some("D"));
        // All spaces is not a name.
        assert_eq!(normalise(&[0xa0; 30]), None);
        // **The rule maps every byte into printable ASCII by construction**, so
        // it rejects nothing on its own — a NUL comes out as `@`. That is worth
        // pinning rather than assuming away: what keeps a catalogue entry apart
        // from game data written over one is `volume_is_sane`'s VTOC test and
        // the entry's own track/sector bounds, not this.
        assert_eq!(normalise(&[0x00]).as_deref(), Some("@"));
    }

    /// A whole round trip: a text file, a binary file with a prologue, and one
    /// that spans several sectors.
    #[test]
    fn a_synthetic_volume_round_trips_through_its_own_catalogue() {
        let long: Vec<u8> = (0..900u32).map(|i| (i % 251) as u8).collect();
        let mut binary = vec![0x00, 0x40, 0x40, 0x00];
        binary.extend((0..64u32).map(|i| (i % 7) as u8));
        let mut disk = DiskBuilder::new();
        disk.add_file("HELO", 0x02, b"10 PRINT");
        disk.add_file("A1.DAT", 0x04, &binary);
        disk.add_file("LONG", 0x00, &long);
        let volume = Dos33::mount(disk.finish()).expect("it mounts");

        assert_eq!(volume.volume_number(), 254);
        let names: Vec<&str> = volume.files().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["HELO", "A1.DAT", "LONG"]);
        assert_eq!(volume.files()[0].file_type, FileType::Applesoft);
        assert_eq!(volume.files()[0].file_type.letter(), 'A');
        assert_eq!(volume.files()[1].file_type, FileType::Binary);
        assert!(!volume.files()[1].locked);
        // The catalogue counts the list sector; the read does not.
        assert_eq!(volume.files()[2].sectors, 5, "four data sectors and one list");
        assert_eq!(volume.files()[2].size, 4 * SECTOR);

        // Raw: whole sectors, prologue and padding included.
        let raw = volume.read_named("a1.dat").expect("case-insensitive");
        assert_eq!(raw.len(), SECTOR);
        assert_eq!(&raw[..binary.len()], &binary[..]);
        // Stripped: the address and exactly the declared bytes.
        let (address, data) = volume.read_binary(&volume.files()[1]).expect("a B file");
        assert_eq!(address, 0x4000);
        assert_eq!(data, binary[4..]);
        // …and a file that is not binary has no prologue to strip.
        assert_eq!(volume.read_binary(&volume.files()[0]), None);

        assert_eq!(volume.read_named("LONG").map(|b| b[..900].to_vec()), Some(long));
        assert_eq!(volume.read_named("nothing"), None);
        assert_eq!(volume.contents().len(), 3);
    }

    /// The sniff wants a VTOC **and** a catalogue with something in it. Both
    /// halves, said separately, because both are load-bearing on the corpus.
    #[test]
    fn a_vtoc_alone_is_not_a_volume() {
        let empty = DiskBuilder::new().finish();
        assert!(!looks_like_dos33(&empty), "a catalogue naming nothing is not evidence");
        assert_eq!(Dos33::mount(empty).err(), Some(Dos33Error::NotDos33));

        let good = sample_disk(&[("HELO", b"x")]);
        assert!(looks_like_dos33(&good));

        // Break each VTOC field in turn; each one alone refuses the disk.
        for (offset, value) in [(0x34, 40), (0x35, 13), (0x27, 121), (0x01, TRACKS as u8)] {
            let mut broken = good.clone();
            broken[VTOC_TRACK * SECTORS * SECTOR + offset] = value;
            assert!(!looks_like_dos33(&broken), "VTOC +{offset:#04x}");
        }
        let mut broken = good.clone();
        broken[VTOC_TRACK * SECTORS * SECTOR + 0x36] = 0x80; // 128 bytes a sector
        assert!(!looks_like_dos33(&broken));

        assert!(!looks_like_dos33(&[]));
        assert!(!looks_like_dos33(&vec![0u8; IMAGE_LEN]));
        assert!(!looks_like_dos33(&vec![0u8; IMAGE_LEN + 1]), "only this geometry");
    }

    /// A deleted entry and a never-used one are both stepped over, and the
    /// entries behind them still read.
    #[test]
    fn deleted_and_unused_catalogue_slots_are_skipped() {
        let mut image = sample_disk(&[("ONE", b"1"), ("TWO", b"2"), ("THREE", b"3")]);
        let first = (CATALOGUE[0].0 * SECTORS + CATALOGUE[0].1) * SECTOR + FIRST_ENTRY;
        image[first] = DELETED;
        let volume = Dos33::mount(image).expect("it mounts");
        let names: Vec<&str> = volume.files().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["TWO", "THREE"]);
    }

    /// **Why a ProDOS-ordered DOS 3.3 disk is refused**, measured rather than
    /// asserted — the module header's claim, made falsifiable.
    ///
    /// Sectors 0 and 15 are the interleave's only fixed points, so the VTOC and
    /// the first catalogue sector sit at the same offsets in both orderings and
    /// every structural test below passes on a shuffled disk. What does NOT
    /// survive is the data: the files are named and their contents come out of
    /// the wrong sectors. §7.4 says exactly this, and a reader that "found" such
    /// a volume would hand back rubbish rather than refuse it.
    #[test]
    fn a_prodos_ordered_dump_looks_like_a_volume_and_reads_the_wrong_bytes() {
        let dos = sample_disk(&[("HELO", b"10 PRINT"), ("A1.DAT", b"data")]);
        let po = crate::dos_order::prodos_order(&dos).expect("the right geometry");
        assert_ne!(po, dos, "the two orderings really do differ");

        // The VTOC and the first catalogue sector are byte-identical in both,
        // which is the whole problem stated as an equality.
        let vtoc = VTOC_TRACK * SECTORS * SECTOR;
        assert_eq!(po[vtoc..vtoc + SECTOR], dos[vtoc..vtoc + SECTOR]);
        let catalogue = (CATALOGUE[0].0 * SECTORS + CATALOGUE[0].1) * SECTOR;
        assert_eq!(po[catalogue..catalogue + SECTOR], dos[catalogue..catalogue + SECTOR]);

        // So the shuffled disk passes the sniff and names its files correctly…
        assert!(looks_like_dos33(&po), "a ProDOS-ordered disk is not distinguishable here");
        let volume = Dos33::mount(po).expect("it mounts");
        let names: Vec<&str> = volume.files().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["HELO", "A1.DAT"]);
        // …and hands back something that is not the file. THAT is the refusal:
        // this reader claims the ordering it can be checked on, and no row
        // claims `.po` on its behalf.
        let got = volume.read_named("A1.DAT").expect("it answers to the name");
        assert_ne!(
            got.get(..4),
            Some(&b"data"[..]),
            "if this ever passes, the interleave has changed and the refusal can be revisited"
        );
    }

    /// A file out of the gitignored `stories/`, or `None` with a SKIP note.
    fn fixture(name: &str) -> Option<Vec<u8>> {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/scott-dialects/apple")
            .join(name);
        match std::fs::read(&p) {
            Ok(b) => Some(b),
            Err(_) => {
                eprintln!("SKIP: gitignored fixture missing at {}", p.display());
                None
            }
        }
    }

    /// The seven boot sides, and the database file each one carries under one of
    /// §7.4's recognised names. `stories/` is gitignored, so this skips on CI.
    #[test]
    fn every_apple_boot_side_names_its_database_file() {
        // (side, database file, its stored size, its load address and length)
        let expected: [(&str, &str, usize, u16, usize); 7] = [
            ("Scott Adams Graphic Adventure 1 - Adventureland v2.1-416 (4am crack) side B - boot.dsk", "A1.DAT", 10_496, 0x4000, 10_335),
            ("Scott Adams Graphic Adventure 2 - Pirate Adventure v2.1-408 (4am crack) side B - boot.dsk", "A2.DAT", 10_496, 0x4000, 10_335),
            ("Scott Adams Graphic Adventure 3 - Mission Impossible v2.1-306 (4am crack) side B (boot).dsk", "A3.DAT", 10_496, 0x4000, 10_335),
            ("Scott Adams Graphic Adventure 4 - Voodoo Castle v2.1-119 (4am crack) side B (boot).dsk", "A4.DAT", 10_496, 0x4000, 10_335),
            ("Scott Adams Graphic Adventure 5 - The Count v2.1-115 (4am crack) side B - boot.dsk", "DATABASE", 10_496, 0x4000, 10_335),
            ("Scott Adams Graphic Adventure 6 - Strange Odyssey v2.1-119 (4am crack) side B - boot.dsk", "A6.DAT", 10_496, 0x4000, 10_335),
            ("Scott Adams Graphic Adventure 13 - The Sorcerer of Claymorgue Castle v2.2-122 (4am crack) side B (boot).dsk", "DATABASE", 10_496, 0x4000, 10_335),
        ];
        let mut seen = 0;
        for (side, database, stored, address, declared) in expected {
            let Some(raw) = fixture(side) else { continue };
            seen += 1;
            assert_eq!(raw.len(), IMAGE_LEN, "{side}");
            assert!(looks_like_dos33(&raw), "{side} is a DOS 3.3 volume");
            let volume = Dos33::mount(raw).expect("it mounts");
            assert_eq!(volume.volume_number(), 254, "{side}");
            let entry = volume
                .files()
                .iter()
                .find(|e| e.name == database)
                .unwrap_or_else(|| panic!("{side}: no {database}"));
            assert_eq!(entry.file_type, FileType::Binary, "{side}");
            assert_eq!(entry.sectors, 42, "{side}: the catalogue's own count");
            assert_eq!(entry.size, stored, "{side}");
            assert_eq!(volume.read_named(database).map(|b| b.len()), Some(stored), "{side}");
            assert_eq!(
                volume.read_binary(entry).map(|(a, b)| (a, b.len())),
                Some((address, declared)),
                "{side}: the four-byte prologue"
            );
            // Every name listed reads back, on the whole side.
            for (name, bytes) in volume.contents() {
                assert_eq!(volume.read_named(&name).as_ref(), Some(&bytes), "{side}: {name}");
            }
        }
        if seen == 0 {
            eprintln!("SKIP: no Apple II S.A.G.A. boot sides present");
        }
    }

    /// **The plain/scrambled split, read off the disks the way the specimen
    /// README settled it** — §7.4's own string test, which is the model this
    /// crate follows everywhere: validate before trusting a fixed offset.
    ///
    /// `M2` on the three scrambled titles carries a descrambling table and is
    /// 6,656 bytes; on the four plain ones it is 3,584 and the offset is off the
    /// end of the file. Pinned here because it is the cheapest possible proof
    /// that this reader is following the right chains.
    #[test]
    fn the_m2_string_test_separates_the_plain_releases_from_the_scrambled() {
        const MARKER: &[u8] = b"COPYRIGHT 1983 NORMAN L. SAILER";
        let sides: [(&str, bool, usize); 7] = [
            ("Scott Adams Graphic Adventure 1 - Adventureland v2.1-416 (4am crack) side B - boot.dsk", false, 3_584),
            ("Scott Adams Graphic Adventure 2 - Pirate Adventure v2.1-408 (4am crack) side B - boot.dsk", false, 3_584),
            ("Scott Adams Graphic Adventure 3 - Mission Impossible v2.1-306 (4am crack) side B (boot).dsk", false, 3_584),
            ("Scott Adams Graphic Adventure 4 - Voodoo Castle v2.1-119 (4am crack) side B (boot).dsk", true, 6_656),
            ("Scott Adams Graphic Adventure 5 - The Count v2.1-115 (4am crack) side B - boot.dsk", true, 6_656),
            ("Scott Adams Graphic Adventure 6 - Strange Odyssey v2.1-119 (4am crack) side B - boot.dsk", false, 3_584),
            ("Scott Adams Graphic Adventure 13 - The Sorcerer of Claymorgue Castle v2.2-122 (4am crack) side B (boot).dsk", true, 6_656),
        ];
        let mut seen = 0;
        for (side, scrambled, size) in sides {
            let Some(raw) = fixture(side) else { continue };
            seen += 1;
            let volume = Dos33::mount(raw).expect("it mounts");
            let m2 = volume.read_named("M2").unwrap_or_else(|| panic!("{side}: no M2"));
            assert_eq!(m2.len(), size, "{side}");
            let marks = m2.len() >= 0x172c + MARKER.len() && &m2[0x172c..0x172c + MARKER.len()] == MARKER;
            assert_eq!(marks, scrambled, "{side}");
        }
        if seen == 0 {
            eprintln!("SKIP: no Apple II S.A.G.A. boot sides present");
        }
    }

    /// **The three side As that are not volumes at all**, said as an assertion
    /// rather than left out: the scrambled titles put their pictures on the boot
    /// disk and their side A is not a DOS 3.3 disk, which is what the specimen
    /// README found independently and what keeps this sniff honest.
    #[test]
    fn the_scrambled_titles_have_no_readable_side_a() {
        let sides = [
            ("Scott Adams Graphic Adventure 4 - Voodoo Castle v2.1-119 (4am crack) side A.dsk", false),
            ("Scott Adams Graphic Adventure 5 - The Count v2.1-115 (4am crack) side A.dsk", false),
            ("Scott Adams Graphic Adventure 13 - The Sorcerer of Claymorgue Castle v2.2-122 (4am crack) side A.dsk", false),
            ("Scott Adams Graphic Adventure 1 - Adventureland v2.1-416 (4am crack) side A.dsk", true),
            ("Scott Adams Graphic Adventure 2 - Pirate Adventure v2.1-408 (4am crack) side A.dsk", true),
            ("Scott Adams Graphic Adventure 3 - Mission Impossible v2.1-306 (4am crack) side A.dsk", true),
            ("Scott Adams Graphic Adventure 6 - Strange Odyssey v2.1-119 (4am crack) side A.dsk", true),
        ];
        let mut seen = 0;
        for (side, readable) in sides {
            let Some(raw) = fixture(side) else { continue };
            seen += 1;
            assert_eq!(looks_like_dos33(&raw), readable, "{side}");
        }
        if seen == 0 {
            eprintln!("SKIP: no Apple II S.A.G.A. sides present");
        }
    }
}
