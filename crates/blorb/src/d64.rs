//! Infocom's **Commodore releases** — a story on 1541 floppies, outside any
//! filesystem, and on more than one of them (SQ-0869).
//!
//! # The medium
//!
//! A `.d64` is a sector-by-sector dump of a Commodore 1541 5.25-inch disk: 35
//! tracks, `21/19/18/17` sectors on the four zones, 256 bytes each,
//! **174,848 bytes** in total. Unlike the Apple II's `.dsk` there is no
//! interleave to undo — the `.d64` container is defined to hold each track's
//! sectors in ascending logical order, which is the order this module reads them
//! in and the reason [`crate::dos_order`] has no counterpart here.
//!
//! Commodore DOS keeps its Block Availability Map at track 18 sector 0 and its
//! directory chain from track 18 sector 1. **All three Infocom disks in the
//! corpus have one and none of them uses it**, which is the single most
//! important fact about this format:
//!
//! ```text
//!   TRINITY1.D64   disk name `TRINITY`            DOS `2A`   directory: unreadable
//!   TRINITY2.D64   disk name `SIDE 2`             DOS `2A`   directory: unreadable
//!   Hitchhiker's   disk name `HITCHHIKER GUIDE`   DOS `TG`   directory: three entries
//! ```
//!
//! *Trinity*'s directory sector holds story data — the game is written straight
//! over it — and its BAM claims 681 of 664 usable blocks free while 387 and 681
//! sectors respectively are written. *Hitchhiker's* keeps a decorative directory
//! whose one file, `THE HITCHHIKER'S`, is three blocks of BASIC loader, and its
//! DOS version bytes read `TG` rather than the standard `2A` — a mastering or
//! copy-protection marker. So the story is never a CBM file and is never
//! reachable through the directory: it is raw sectors, and where they are has to
//! be measured.
//!
//! # Where the story is, measured
//!
//! Exactly one sector boundary on each header-bearing disk opens a Z-machine
//! header ([`header_candidates`]), and the two presses lay their sectors out
//! **differently**:
//!
//! ```text
//!   Hitchhiker's r47 s840914   v3   112,622 bytes   440 sectors
//!       track 5 sector 0 onward, SIXTEEN sectors per track (`s0`..`s15`),
//!       skipping tracks 17 and 18, ending at track 34 sector 7.
//!
//!   Trinity r12 s860926        v4   262,064 bytes  1024 sectors
//!       SIDE 1: track 3 sector 0 .. track 19 sector 10, EVERY sector,
//!               skipping only the BAM at track 18 sector 0        (344)
//!       SIDE 2: track 1 sector 0 .. track 35 sector 14, likewise  (680)
//! ```
//!
//! The 1984 press spends 16 of each track's 21 sectors and leaves the rest
//! formatted-blank, which is visible in the image as a `s16`..`s20` gap on every
//! story track; the 1986 press spends all of them. Neither is guessed: the two
//! plans are [`Plan`], and a mount tries each and keeps the one whose reassembly
//! **verifies against the story's own header checksum**
//! ([`crate::infocom_packed::verified`], shared rather than copied).
//!
//! Where a press stops on a disk needs no table either. A 1541 `FORMAT` leaves
//! every data block as `$4B` followed by 255 × `$01`, and that is exactly what
//! *Trinity*'s SIDE 1 holds from track 19 sector 11 to the end of the disk. The
//! reader stops at the first never-written block ([`never_written`]) and moves
//! to the next volume, so the 344/680 split above falls out of the media rather
//! than being asserted about them.
//!
//! # Two disks, and why *Trinity* genuinely needs both
//!
//! This is arithmetic, not a judgement. *Trinity* is Version 4, so its `$1A`
//! length field counts in units of **four** (ZMSD §11.1.6): `0xFFEC × 4 =
//! 262,064` bytes. One 1541 disk holds 174,848 bytes in total, interpreter and
//! all. The story does not fit on one disk and no arrangement of it could.
//!
//! *Hitchhiker's* is Version 3, counts in units of two, and its 112,622 bytes
//! fit on its single disk with room to spare.
//!
//! Reassembly across the set is [`story_across`], reached from
//! [`crate::medium::MountedDisk::mount_set`] like the Apple II's packed
//! container beside it. Which side leads is not taken on trust from the caller's
//! ordering: only one side of a release carries a header, so the head is the
//! segment that has one.
//!
//! # Settling the layout, three ways
//!
//! The header checksum is a byte **sum** and is therefore blind to ordering — it
//! pins *which* sectors a story is made of and says nothing about the order they
//! were put in. SQ-0868 found a sector swap on the Apple disk that passed every
//! checksum-based test there was. So, as there:
//!
//! 1. **An order-sensitive fingerprint.** FNV-1a over all 440 and all 1024
//!    sectors in order, pinned in the tests below.
//! 2. **A structure the layout would corrupt.** The dictionary at the header's
//!    own pointer decodes as a textbook one under the right plan and as nonsense
//!    under the wrong one: `, . "` with 7-byte entries and 969 words for
//!    *Hitchhiker's*, `. , " ! ?` with 9-byte entries and 2,120 words for
//!    *Trinity*, whose last byte lands exactly on its own high-memory mark.
//! 3. **A second source.** *Trinity*'s Commodore press is release 12 serial
//!    860926 checksum `$16AB` — the same build as `stories/trinity-r12-s860926.z4`
//!    — and what this reader assembles off the two floppies is **byte-identical
//!    to that file from `$40` to the end**, all 262,000 bytes of it.
//!
//! # The three header bytes that differ, and why they are allowed to
//!
//! Below `$40` the Commodore *Trinity* is not byte-identical, and this is worth
//! recording rather than smoothing over. It carries `$01 = 04` where the
//! reference has `00` — Flags 1 bit 2, "boldface available", a capability bit an
//! interpreter writes — and, more surprisingly, `$04 = 57FF` where the reference
//! has `F7BF`: the base of high memory, lowered from 63,423 to 22,527.
//!
//! That is a press written for a machine with 64 KB of RAM and a 256 KB story:
//! nearly everything has to be pageable, so the resident region is a third of
//! what the reference build declares. The header checksum is defined over
//! `$40..length` precisely so that the interpreter-facing head of the header can
//! differ, and all three bytes are inside that exempt region.
//!
//! It does mean the story's high-memory mark sits **below** its static-memory
//! base, which no other release in the corpus does; see [`crate::adf::looks_like_story`]
//! for the one clause that had to stop assuming otherwise.

use crate::infocom_pics::InfocomPics;
use std::borrow::Cow;

/// Bytes in a 1541 sector — and therefore the only alignment a story can start
/// on, since the loader reads whole sectors.
///
/// `pub(crate)` for [`crate::g64`], which decodes a GCR bitstream into sectors
/// of this size and must not keep its own copy of the number.
pub(crate) const SECTOR: usize = 256;

/// Tracks on a standard 1541 disk. The 40-track extensions exist; nothing in the
/// corpus is one, and a row that claimed a geometry no medium here has would be
/// the guess this crate declines everywhere else.
pub(crate) const TRACKS: usize = 35;

/// A 35-track 1541 image: 683 sectors of 256 bytes.
pub const D64_LEN: usize = 174_848;

/// Commodore DOS keeps its BAM and its directory chain here.
const DIRECTORY_TRACK: usize = 18;

/// A data block as the 1541's `FORMAT` command leaves it: `$4B`, then 255 ×
/// `$01`. The end of a press's data is where these begin.
const FORMATTED_BLANK: (u8, u8) = (0x4b, 0x01);

/// How much of a disk must lie outside its own filesystem before a side with no
/// story on it is taken for part of an Infocom release — 256 sectors, 64 KB.
/// See [`D64::looks_like_d64`] for why that arm exists at all.
const RAW_SECTOR_FLOOR: usize = 256;

/// Errors that can arise while mounting a Commodore 1541 image.
#[derive(Debug, PartialEq, Eq)]
pub enum D64Error {
    /// Not a 35-track 1541 image of an Infocom Commodore release.
    NotAD64,
}

/// How a press spends the sectors of a track.
///
/// **Two plans because the corpus has two presses**, not because a spec says so
/// — see the module header for the measurement. A mount tries each and keeps the
/// one whose reassembly verifies, so neither is ever assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plan {
    /// Every sector of every track, skipping only the BAM. *Trinity*, 1986.
    Dense,
    /// Sectors `0..16` of each track, skipping the loader and directory tracks
    /// 17 and 18 whole. *Hitchhiker's*, 1984.
    Sixteen,
    /// Seventeen sectors of each track, skipping the loader track 17 whole and
    /// starting at sector 2 on the directory track — so the BAM and the one
    /// directory sector behind it are stepped over and the seventeen taken from
    /// there. *Plundered Hearts*, 1987 (SQ-1095).
    ///
    /// Measured, not guessed, and the measurement is worth keeping because it is
    /// the third layout in three presses. Every 256-byte block of
    /// `stories/plunderedhearts-r26-s870730.z3` occurs exactly once on
    /// `plundered_hearts[infocom_1987](r26)(!).g64`, and mapping each to the
    /// sector it came off gives an unbroken run: `T5/S0`..`T16/S16`, then
    /// `T18/S2`..`T18/S18`, then `T19/S0` onward at seventeen a track to
    /// `T35/S9`. Track 17 is absent from that run, and track 18's first two
    /// sectors are the filesystem's.
    Seventeen,
}

/// The plans a mount tries, in order. Order is not precedence — a plan is kept
/// only when the story it produces verifies — but `Dense` first means the disk
/// that spends every sector is answered without building the other candidate.
const PLANS: [Plan; 3] = [Plan::Dense, Plan::Sixteen, Plan::Seventeen];

/// What Commodore DOS says a directory entry is — the low nibble of its type
/// byte, for the two kinds this reader hands over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CbmFileType {
    /// `SEQ`, a sequential file.
    Seq,
    /// `PRG`, a program file — which is every file on every Commodore Scott
    /// Adams disk in the corpus.
    Prg,
}

/// One closed file the CBM directory names (SQ-1458).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CbmFile {
    /// The 16-byte name with its `$A0` padding stripped, mapped out of PETSCII
    /// by this module's PETSCII rule — see `cbm_name`.
    pub name: String,
    /// `PRG` or `SEQ`.
    pub file_type: CbmFileType,
    /// The block count the directory carries. Reported because `LOAD"$"` does;
    /// never used to size a read, which follows the chain.
    pub blocks: usize,
    /// The file's bytes, chain order.
    ///
    /// **A `PRG`'s first two bytes are its load address**, little-endian, and
    /// they are kept: that is what the file *is*, it is how every extracted
    /// `.prg` beside these images is stored, and there is no fixed base address
    /// on this machine to recover it from later
    /// (`scott-dialects-spec.md` §7.5). A caller wanting the memory image drops
    /// them itself and reads byte 2 as living at that address.
    bytes: Vec<u8>,
}

/// A mounted Commodore 1541 disk.
#[derive(Debug)]
pub struct D64 {
    /// The name in the BAM, when it reads as one (`TRINITY`, `SIDE 2`).
    name: Option<String>,
    /// The closed `PRG` and `SEQ` files the directory names, in directory order.
    /// Empty on every Infocom disk in the corpus but one, and the whole of what
    /// an ordinary Commodore disk has.
    files: Vec<CbmFile>,
    /// Where the story starts, as a (track, sector). `None` on a side that
    /// carries no header — a continuation disk, which is a whole third of this
    /// corpus.
    at: Option<(usize, usize)>,
    /// The story, when this one disk holds all of it. A side of a two-disk
    /// release holds none by itself; that is [`story_across`]'s business.
    ///
    /// The image itself is deliberately NOT kept. A side of a two-disk release
    /// is reassembled from the sides [`crate::medium::MountedDisk`] gathered, so
    /// nothing asks a mounted volume for its own sectors and a mount costs one
    /// story rather than one story and a floppy.
    story: Option<Vec<u8>>,
}

impl D64 {
    /// Cheap sniff: is this a Commodore 1541 image of an Infocom release?
    ///
    /// "Cheap" is relative — a header-bearing disk is reassembled and
    /// checksummed — but a file of the wrong length leaves at the first line,
    /// which is every file in a library but these three.
    ///
    /// **Two arms, and the second one is a concession the first cannot avoid.**
    /// A disk that carries a whole verified story identifies itself completely,
    /// which is the standard this crate holds every format to. A *continuation*
    /// side carries no header, no checksum and no name that means anything —
    /// `SIDE 2` is not evidence — so there is nothing on it to verify, and the
    /// strongest true statement available is that its data lies **outside its own
    /// filesystem**: its directory chain does not read, and hundreds of sectors
    /// its BAM calls free are written. An ordinary Commodore disk is the exact
    /// opposite of that on both counts, because a disk whose directory does not
    /// read is a disk nobody could load a program from.
    ///
    /// What that concession cannot do is produce a wrong story, and that is the
    /// property worth having: a side is only ever *joined* to a release by
    /// [`story_across`], which verifies the join against the story's own header
    /// checksum. A `.d64` that is not an Infocom release either fails here or
    /// mounts and reports no story; it is never misread as one.
    ///
    /// **A third arm since SQ-1458: a disk whose directory names files.** That
    /// is the ordinary Commodore disk this sniff used to refuse outright, and it
    /// has to be accepted now because the Scott Adams Commodore releases —
    /// `MYSTADV1.D64`, `MYSTADV2.D64`, the two Questprobe disks — are exactly
    /// that and nothing else: a plain CBM DOS directory of uncompressed `PRG`
    /// files. Refusing them meant a whole family of games could not be opened
    /// through this crate's one disk seam at all.
    ///
    /// It costs the arms above nothing, because it is a strictly different
    /// question. An entry has to be a **closed** `PRG` or `SEQ` with a non-zero
    /// block count, a start sector on the disk and a name; the decorative
    /// entries the Infocom presses carry — *Zork I*'s `>GOLDEN EDITION<` banner
    /// and its zero-block `ZORK I`, *Hitchhiker's* two `GUIDE TO THE` /
    /// `GALAXY.` lines — are `DEL` or claim no blocks and are not files. Nothing
    /// in `stories/` changes hands: the same three images are Infocom releases
    /// and the same two *Zork I* sides are not.
    pub fn looks_like_d64(raw: &[u8]) -> bool {
        if raw.len() != D64_LEN || !bam_is_sane(raw) {
            return false;
        }
        (!directory_reads(raw) && raw_sectors(raw) >= RAW_SECTOR_FLOOR)
            || !directory(raw).is_empty()
            || story_on(&[raw]).is_some()
    }

    /// Open the disk, or `Err` when it is not one of these.
    ///
    /// The sniff's own work, kept — recognising and opening are the same question
    /// here, so [`Self::looks_like_d64`] cannot claim bytes this would refuse.
    pub fn mount(raw: Vec<u8>) -> Result<D64, D64Error> {
        if !D64::looks_like_d64(&raw) {
            return Err(D64Error::NotAD64);
        }
        let found = story_on(&[&raw]);
        Ok(D64 {
            name: disk_name(&raw),
            files: directory(&raw),
            at: found.as_ref().map(|(at, _)| *at),
            story: found.map(|(_, story)| story),
        })
    }

    /// The closed `PRG` and `SEQ` files the directory names, in directory order.
    ///
    /// **The door a Scott Adams Commodore loader comes through**: each file's
    /// bytes are the program file as CBM DOS stores it, load address and all
    /// (see `CbmFile`'s own docs). Empty on a disk that keeps nothing in its
    /// filesystem, which is every Infocom press here but *Hitchhiker's*.
    pub fn files(&self) -> &[CbmFile] {
        &self.files
    }

    /// The name Commodore DOS keeps at `$90` of the BAM, `$A0`-padded and here
    /// trimmed — `TRINITY`, `SIDE 2`, `HITCHHIKER GUIDE`.
    ///
    /// A label and never an identifier, like every other name in this crate: two
    /// sides of one release are called `TRINITY` and `SIDE 2`, which says nothing
    /// about either being a Commodore disk and nothing about which game.
    pub fn volume_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// How the story is named in a listing: `T5/S0`, the track and sector its
    /// header sits at.
    ///
    /// The same convention [`crate::infocom_boot`] uses, for the same reason — a
    /// disk with no filesystem has no filename to report, and where the story is
    /// is the only thing this medium knows about it.
    pub fn entry_name(&self) -> Option<String> {
        self.at.map(|(track, sector)| format!("T{track}/S{sector}"))
    }

    /// The story on this one disk, when it holds all of it.
    ///
    /// `None` on either side of *Trinity*: SIDE 1 carries the header and 344 of
    /// the 1,024 sectors, SIDE 2 carries the other 680 and no header, and neither
    /// is a game. See [`story_across`].
    pub fn story(&self) -> Option<(String, Vec<u8>)> {
        Some((self.entry_name()?, self.story.clone()?))
    }

    /// Everything the disk can be shown to hold: the files its directory names,
    /// then the raw-sector story if it has one — **and on a continuation side,
    /// nothing at all**.
    ///
    /// The two halves are different kinds of thing and both are real. An
    /// ordinary Commodore disk keeps its programs in CBM DOS files, which is
    /// every Scott Adams release here and *Hitchhiker's* one 552-byte BASIC
    /// loader; an Infocom press keeps its story in raw sectors outside the
    /// filesystem, which no directory can name, and it is reported under
    /// [`D64::entry_name`] — `T5/S0`, where it is.
    ///
    /// What is still **not** reported is the interpreter: it is 6502 the boot
    /// ROM or a BASIC stub jumps into, it is not a file, and inventing an entry
    /// for it would be inventing a directory these disks do not use. And
    /// *Trinity*'s two sides list nothing whatever, because neither holds a game
    /// and neither has a readable directory; that release is reassembled from
    /// the raw images by [`story_across`], which
    /// [`crate::medium::MountedDisk`] reaches with the sides themselves rather
    /// than with this listing.
    pub fn contents(&self) -> Vec<(String, Vec<u8>)> {
        let mut out: Vec<(String, Vec<u8>)> =
            self.files.iter().map(|f| (f.name.clone(), f.bytes.clone())).collect();
        out.extend(self.story());
        out
    }

    /// One entry by the name a caller was shown, case-insensitively — the
    /// property every format here has to have, because it is the `--pictures`
    /// door.
    ///
    /// Both kinds of name answer: a CBM filename, and the `T5/S0` the raw-sector
    /// story is listed under. A disk with two files of the same name — the two
    /// graphics-character entries on `QUESTPR1.D64` normalise alike — answers
    /// with the first, which is what `LOAD` does.
    pub fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        if let Some(file) = self.files.iter().find(|f| f.name.eq_ignore_ascii_case(name)) {
            return Some(file.bytes.clone());
        }
        match self.entry_name() {
            Some(entry) if name.eq_ignore_ascii_case(&entry) => self.story.clone(),
            _ => None,
        }
    }

    /// How many entries the mount found — the directory's files, plus one when
    /// this disk holds a whole game in raw sectors, and none when it is one side
    /// of a release that spans two.
    pub fn file_count(&self) -> usize {
        self.files.len() + usize::from(self.story.is_some())
    }

    /// **No artwork, and that is a limit rather than a finding.**
    ///
    /// Infocom pressed no Version 6 game for the Commodore — the YZIP never ran
    /// on one — so neither disk here has artwork to carry and there is no
    /// evidence about where a Commodore press would keep an archive. Scanning for
    /// one would be a guess with no medium behind it, which is the rule
    /// [`crate::infocom_boot`] applies to the same question.
    pub fn pictures(&self) -> Option<(String, InfocomPics)> {
        None
    }
}

// ── Geometry ──────────────────────────────────────────────────────────────────

/// Sectors on `track`, over the 1541's four speed zones. Tracks are 1-based, as
/// Commodore DOS numbers them; `0` for anything off the disk.
///
/// `pub(crate)` for [`crate::g64`]: a GCR track states which sector each block
/// is, and this says which of those a 35-track image has room for. **The one
/// statement of this geometry in the crate** — a decoder with its own table
/// would be the hand-maintained cross-file invariant the refactoring policy
/// names.
pub(crate) fn sectors_per_track(track: usize) -> usize {
    match track {
        1..=17 => 21,
        18..=24 => 19,
        25..=30 => 18,
        31..=35 => 17,
        _ => 0,
    }
}

/// Sectors on a whole 35-track disk: 683.
fn total_sectors() -> usize {
    (1..=TRACKS).map(sectors_per_track).sum()
}

/// The image-order index of `(track, sector)`. A `.d64` stores each track's
/// sectors in ascending order, so this is a plain running total.
///
/// `pub(crate)` for [`crate::g64`], which assembles an image and needs to put
/// each decoded sector where this module will look for it.
pub(crate) fn linear(track: usize, sector: usize) -> usize {
    (1..track).map(sectors_per_track).sum::<usize>() + sector
}

/// The inverse of [`linear`].
fn track_sector(mut index: usize) -> (usize, usize) {
    for track in 1..=TRACKS {
        let n = sectors_per_track(track);
        if index < n {
            return (track, index);
        }
        index -= n;
    }
    (0, 0)
}

/// The sector at image-order index `index`.
fn sector_at(raw: &[u8], index: usize) -> &[u8] {
    &raw[index * SECTOR..(index + 1) * SECTOR]
}

/// Has this block never been written since the disk was formatted? A 1541
/// `FORMAT` leaves `$4B` and then 255 × `$01`, and a press's data ends where
/// those begin.
fn never_written(block: &[u8]) -> bool {
    block[0] == FORMATTED_BLANK.0 && block[1..].iter().all(|&b| b == FORMATTED_BLANK.1)
}

// ── The filesystem, only ever as evidence about what is NOT one ───────────────

/// Is there a structurally sane BAM at track 18 sector 0? Cheap, and it is the
/// whole of what makes an image a 1541 image rather than 174,848 arbitrary bytes.
fn bam_is_sane(raw: &[u8]) -> bool {
    let bam = sector_at(raw, linear(DIRECTORY_TRACK, 0));
    // The BAM points at the first directory sector, and Commodore DOS puts it at
    // 18/1 on every disk this format has ever had.
    if bam[0] != DIRECTORY_TRACK as u8 || bam[1] != 1 || bam[2] != b'A' {
        return false;
    }
    // Every track's free-block count is within that track's capacity.
    (1..=TRACKS).all(|t| usize::from(bam[4 * t]) <= sectors_per_track(t))
}

/// The disk name at `$90` of the BAM, `$A0`-padded, or `None` when it is not
/// printable. Commodore stores it in PETSCII; the names in the corpus are all
/// within the range where PETSCII and ASCII agree, and a byte outside it is
/// grounds for reporting no name rather than for inventing a transliteration.
fn disk_name(raw: &[u8]) -> Option<String> {
    let bam = sector_at(raw, linear(DIRECTORY_TRACK, 0));
    let name: Vec<u8> = bam[0x90..0xa0].iter().copied().take_while(|&b| b != 0xa0).collect();
    if name.is_empty() || !name.iter().all(|&b| (0x20..0x7f).contains(&b)) {
        return None;
    }
    Some(String::from_utf8_lossy(&name).trim_end().to_string())
}

/// Does the CBM directory chain read as a directory at all?
///
/// Only ever asked in the negative: a disk whose chain does NOT read is a disk
/// with no filesystem in use, which is one half of what identifies a
/// continuation side. Nothing here is used to find a story — the story is never
/// a CBM file on any of these disks.
fn directory_reads(raw: &[u8]) -> bool {
    let bam = sector_at(raw, linear(DIRECTORY_TRACK, 0));
    let (mut track, mut sector) = (usize::from(bam[0]), usize::from(bam[1]));
    let mut seen: Vec<(usize, usize)> = Vec::new();
    while track != 0 {
        if track > TRACKS || sector >= sectors_per_track(track) || seen.contains(&(track, sector)) {
            return false;
        }
        seen.push((track, sector));
        if seen.len() > sectors_per_track(DIRECTORY_TRACK) {
            return false;
        }
        let block = sector_at(raw, linear(track, sector));
        // Eight 32-byte entries per sector, the first two bytes of the first one
        // being the chain link above — so a type byte sits at `slot * 32 + 2`.
        for slot in 0..8 {
            let file_type = block[slot * 32 + 2];
            // The low nibble is DEL, SEQ, PRG, USR or REL; the high bit is
            // "closed". Anything else means this is not a directory sector.
            if file_type != 0 && file_type & 0x0f > 4 {
                return false;
            }
        }
        (track, sector) = (usize::from(block[0]), usize::from(block[1]));
    }
    true
}

/// **The directory walk** — every closed `PRG` and `SEQ` file the chain names,
/// read through its sector chain, in directory order (SQ-1458).
///
/// Gated on [`directory_reads`] and deliberately so: that function is the
/// existing, conservative test for "is this a directory at all", and *Trinity*'s
/// two sides fail it because the game is written over their directory sectors.
/// Without the gate this walk would happily read four "entries" out of
/// *Trinity* SIDE 1's story data — one of them a plausible closed `PRG` — which
/// is precisely the misreading the raw-sector arm exists to avoid.
///
/// An entry has to be all four of these, and each one is the corpus's:
///
/// * **closed** (type bit 7), because an open file's chain was never finished;
/// * `PRG` or `SEQ` (low nibble 2 or 1). `DEL` is the decorative banner line
///   *Zork I*'s and *Hitchhiker's* presses spell their titles with; `USR` and
///   `REL` are absent from every image here, and `REL` is not a plain chain at
///   all — it has side sectors, and reading it as one would be wrong rather
///   than merely unsupported;
/// * **a non-zero block count**, which is what tells `ZORK I` — a closed `PRG`
///   claiming no blocks, sitting on the same track 17 sector 0 as the BASIC
///   stub beside it — from `THE HITCHHIKER'S`, which claims three and has them;
/// * a start track and sector **on the disk**, and a name.
///
/// A chain that then fails to read is dropped, not fatal: one unreadable entry
/// says nothing about its neighbours.
fn directory(raw: &[u8]) -> Vec<CbmFile> {
    if !directory_reads(raw) {
        return Vec::new();
    }
    let bam = sector_at(raw, linear(DIRECTORY_TRACK, 0));
    let (mut track, mut sector) = (usize::from(bam[0]), usize::from(bam[1]));
    let mut seen: Vec<(usize, usize)> = Vec::new();
    let mut out = Vec::new();
    while track != 0 {
        if track > TRACKS || sector >= sectors_per_track(track) || seen.contains(&(track, sector)) {
            break;
        }
        seen.push((track, sector));
        if seen.len() > sectors_per_track(DIRECTORY_TRACK) {
            break;
        }
        let block = sector_at(raw, linear(track, sector));
        for slot in 0..8 {
            let e = &block[slot * 32..slot * 32 + 32];
            let file_type = match (e[2] & 0x80 != 0).then_some(e[2] & 0x0f) {
                Some(1) => CbmFileType::Seq,
                Some(2) => CbmFileType::Prg,
                _ => continue,
            };
            let blocks = usize::from(u16::from_le_bytes([e[30], e[31]]));
            let (first_track, first_sector) = (usize::from(e[3]), usize::from(e[4]));
            if blocks == 0
                || first_track == 0
                || first_track > TRACKS
                || first_sector >= sectors_per_track(first_track)
            {
                continue;
            }
            let Some(name) = cbm_name(&e[5..21]) else { continue };
            let Some(bytes) = chain(raw, first_track, first_sector) else { continue };
            out.push(CbmFile { name, file_type, blocks, bytes });
        }
        (track, sector) = (usize::from(block[0]), usize::from(block[1]));
    }
    out
}

/// One file's bytes, following the 1541's two-byte block links.
///
/// Byte 0 of a block is the next track and byte 1 the next sector. A **non-zero**
/// next track means the block carries a full 254 bytes at offsets 2-255; a zero
/// next track means this is the last block and **byte 1 is the number of used
/// bytes plus one**, so the payload is `byte1 − 1` bytes from offset 2. A byte 1
/// of 0 there is malformed. Visited blocks are tracked, because a corrupt image
/// would otherwise loop forever.
fn chain(raw: &[u8], mut track: usize, mut sector: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut seen: Vec<(usize, usize)> = Vec::new();
    loop {
        if track == 0 || track > TRACKS || sector >= sectors_per_track(track) {
            return None;
        }
        if seen.contains(&(track, sector)) {
            return None;
        }
        seen.push((track, sector));
        let block = sector_at(raw, linear(track, sector));
        if block[0] == 0 {
            let used = usize::from(block[1]);
            if used == 0 {
                return None;
            }
            out.extend_from_slice(&block[2..2 + used - 1]);
            return Some(out);
        }
        out.extend_from_slice(&block[2..]);
        (track, sector) = (usize::from(block[0]), usize::from(block[1]));
    }
}

/// A CBM filename out of its sixteen stored bytes, `$A0`-padded.
///
/// Commodore stores names in **PETSCII**, and the mapping here is the smallest
/// one that reads every name in the corpus and cannot produce a control
/// character: `$20..=$5F` is where PETSCII and ASCII agree and passes through
/// (`ARROW II`, `PULSAR 7`, `WIZARD OF AKYRZ`, `SHULK.DB`); `$C1..=$DA` is
/// shifted-mode upper case and loses its high bit (`QUESTPR1.D64`'s `Á.Ó` is
/// `A.S`); everything else is a graphics character with no letter behind it and
/// becomes `?`. Trailing spaces are trimmed, and a name that is left empty is
/// not a name.
fn cbm_name(field: &[u8]) -> Option<String> {
    let name: String = field
        .iter()
        .take_while(|&&b| b != 0xa0)
        .map(|&b| match b {
            0x20..=0x5f => char::from(b),
            0xc1..=0xda => char::from(b & 0x7f),
            _ => '?',
        })
        .collect();
    let name = name.trim_end().to_string();
    (!name.is_empty()).then_some(name)
}

/// How many sectors this disk holds that its own BAM calls free — data outside
/// the filesystem, which is what an Infocom press is made of.
fn raw_sectors(raw: &[u8]) -> usize {
    let bam = sector_at(raw, linear(DIRECTORY_TRACK, 0));
    let mut n = 0;
    for track in 1..=TRACKS {
        if track == DIRECTORY_TRACK {
            continue;
        }
        let bits = u32::from(bam[4 * track + 1])
            | u32::from(bam[4 * track + 2]) << 8
            | u32::from(bam[4 * track + 3]) << 16;
        for sector in 0..sectors_per_track(track) {
            let free = bits >> sector & 1 == 1;
            if free && !never_written(sector_at(raw, linear(track, sector))) {
                n += 1;
            }
        }
    }
    n
}

// ── Finding and reading a story ───────────────────────────────────────────────

/// Every sector boundary on `raw` that opens a plausible Z-machine header.
///
/// **Not [`crate::adf::looks_like_story`]**, and the difference is the whole
/// reason this exists: that function asks "are these bytes a whole story", and
/// the answer here is *no* by construction — *Trinity* declares 262,064 bytes
/// and the rest of its first disk is 164,096. This asks the different question
/// "does this sector start a story whose body is somewhere else", so every
/// pointer is checked against the story's own DECLARED length rather than
/// against the bytes in hand.
///
/// The checksum rule is not restated here or anywhere else in this module; a
/// candidate is only ever confirmed by [`crate::infocom_packed::verified`].
fn header_candidates(raw: &[u8]) -> Vec<usize> {
    let mut found = Vec::new();
    for index in 0..total_sectors() {
        let b = sector_at(raw, index);
        let word = |o: usize| usize::from(u16::from_be_bytes([b[o], b[o + 1]]));
        // Packed-address scale, which is also the file-length unit (ZMSD §11.1.6).
        let scale = match b[0] {
            3 => 2,
            4 | 5 => 4,
            6..=8 => 8,
            _ => continue,
        };
        let declared = word(0x1a) * scale;
        let (high, dict, objects, globals, static_base) =
            (word(0x04), word(0x08), word(0x0a), word(0x0c), word(0x0e));
        // A story big enough to have a header, a checksum to verify against, and
        // an initial program counter.
        if declared <= SECTOR || word(0x1c) == 0 || word(0x06) == 0 {
            continue;
        }
        // Static memory starts after the header and inside the story; the
        // writable tables are below it and the dictionary at or above it.
        if !(64..=declared).contains(&static_base)
            || !(64..static_base).contains(&objects)
            || !(64..static_base).contains(&globals)
            || !(static_base..declared).contains(&dict)
            || !(64..=declared).contains(&high)
        {
            continue;
        }
        // A printable serial, in ASCII or the high ASCII some presses write.
        if !b[0x12..0x18].iter().all(|c| (0x20..0x7f).contains(&(c & 0x7f))) {
            continue;
        }
        found.push(index);
    }
    found
}

/// The sectors `plan` spends on `raw`, starting at image index `from` and
/// stopping at the first never-written block.
///
/// The stop is what makes a two-disk release assemble without a table of
/// per-side extents: a press's data ends where the formatted-blank blocks begin,
/// and *Trinity* SIDE 1's begin at track 19 sector 11.
fn payload(raw: &[u8], plan: Plan, from: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let bam = linear(DIRECTORY_TRACK, 0);
    let indices: Vec<usize> = match plan {
        Plan::Dense => (from..total_sectors()).filter(|&i| i != bam).collect(),
        Plan::Sixteen => {
            let (first_track, first_sector) = track_sector(from);
            (first_track..=TRACKS)
                .filter(|&t| t != 17 && t != DIRECTORY_TRACK)
                .flat_map(|t| {
                    let start = if t == first_track { first_sector } else { 0 };
                    (start..16).map(move |s| linear(t, s))
                })
                .collect()
        }
        Plan::Seventeen => {
            let (first_track, first_sector) = track_sector(from);
            (first_track..=TRACKS)
                .filter(|&t| t != 17)
                .flat_map(|t| {
                    let base = if t == DIRECTORY_TRACK { 2 } else { 0 };
                    let start =
                        if t == first_track { first_sector.max(base) } else { base };
                    (start..base + 17).map(move |s| linear(t, s))
                })
                .collect()
        }
    };
    for index in indices {
        let block = sector_at(raw, index);
        if never_written(block) {
            break;
        }
        out.extend_from_slice(block);
    }
    out
}

/// The verified story `images` carry, with the (track, sector) its header sits
/// at — or `None`.
///
/// `images[0]` is the side that must carry the header; the rest continue it in
/// order, each from its own track 1 sector 0. Every candidate header is tried
/// under every [`Plan`], and a result is returned only when
/// [`crate::infocom_packed::verified`] agrees the reassembly is the story its own
/// header describes.
fn story_on(images: &[&[u8]]) -> Option<((usize, usize), Vec<u8>)> {
    let head = images.first()?;
    for start in header_candidates(head) {
        for plan in PLANS {
            let mut story = payload(head, plan, start);
            for image in &images[1..] {
                story.extend_from_slice(&payload(image, plan, 0));
            }
            if let Some(story) = crate::infocom_packed::verified(story) {
                return Some((track_sector(start), story));
            }
        }
    }
    None
}

/// The story a Commodore release pages across the sides of its set.
///
/// **Which side leads is decided here and not by the caller**, because the caller
/// legitimately does not know: someone who opens `TRINITY2.D64` gets SIDE 2
/// first and SIDE 1 as its companion. Only one side of a release carries a
/// header, so the head is the segment that has one and the rest follow in the
/// order they arrived. The join is verified against the story's own header
/// checksum, so a pair of disks that do not belong together is refused rather
/// than handed over as plausible-looking Z-code.
pub(crate) fn story_across(segments: &[Vec<u8>]) -> Option<(String, Vec<u8>)> {
    // A side arrives in whichever container it was dumped in, and the two are
    // the same disk: a `.d64` is already a sector image, a `.g64` is the raw
    // bitstream and becomes one here (SQ-1095). Everything below this line reads
    // sectors, so neither container is named again — and a `.d64` is borrowed
    // rather than copied, because this runs on images and 175 KB a side is a
    // price the ordinary case should not pay.
    let images: Vec<Cow<[u8]>> = segments
        .iter()
        .filter_map(|raw| match raw.len() {
            D64_LEN => Some(Cow::Borrowed(raw.as_slice())),
            _ => crate::g64::sector_image(raw).map(Cow::Owned),
        })
        .collect();
    let sides: Vec<&[u8]> = images.iter().map(Cow::as_ref).collect();
    if sides.len() < 2 {
        return None;
    }
    for (i, side) in sides.iter().enumerate() {
        if header_candidates(side).is_empty() {
            continue;
        }
        let mut ordered = vec![*side];
        ordered.extend(sides.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, s)| *s));
        if let Some(((track, sector), story)) = story_on(&ordered) {
            return Some((format!("T{track}/S{sector}"), story));
        }
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A synthetic 1541 image carrying `story` from track 3 sector 0 under the
    /// dense plan — the place and the plan *Trinity* uses.
    ///
    /// [`crate::medium`]'s census uses it, which is why it is `pub(crate)`: every
    /// format there must produce a sample it can then detect, mount and read
    /// back. **No filename argument**, like [`crate::infocom_boot`]'s and for the
    /// same reason — this medium has no directory to put a name in.
    pub(crate) fn sample_disk(story: &[u8]) -> Vec<u8> {
        let mut image = blank_disk();
        let mut index = linear(3, 0);
        for chunk in story.chunks(SECTOR) {
            if index == linear(DIRECTORY_TRACK, 0) {
                index += 1;
            }
            let at = index * SECTOR;
            image[at..at + chunk.len()].copy_from_slice(chunk);
            // A short final chunk must not leave `$4B 01 01 …` behind it, or the
            // reader would stop one sector early — zero is not blank.
            for b in image.iter_mut().skip(at + chunk.len()).take(SECTOR - chunk.len()) {
                *b = 0;
            }
            index += 1;
        }
        image
    }

    /// A formatted, empty 1541 disk: every block `$4B` then 255 × `$01`, with a
    /// BAM that says so.
    fn blank_disk() -> Vec<u8> {
        let mut image = Vec::with_capacity(D64_LEN);
        for _ in 0..total_sectors() {
            image.push(FORMATTED_BLANK.0);
            image.extend(std::iter::repeat_n(FORMATTED_BLANK.1, SECTOR - 1));
        }
        let bam = linear(DIRECTORY_TRACK, 0) * SECTOR;
        image[bam..bam + SECTOR].fill(0);
        image[bam] = DIRECTORY_TRACK as u8;
        image[bam + 1] = 1;
        image[bam + 2] = b'A';
        for track in 1..=TRACKS {
            image[bam + 4 * track] = sectors_per_track(track) as u8;
            image[bam + 4 * track + 1] = 0xff;
            image[bam + 4 * track + 2] = 0xff;
            image[bam + 4 * track + 3] = 0xff;
        }
        image[bam + 0x90..bam + 0xa0].fill(0xa0);
        image[bam + 0x90..bam + 0x97].copy_from_slice(b"SAMPLE ");
        // An empty directory sector, so the chain reads and the sample is only
        // ever claimed by the verified-story arm of the sniff.
        let dir = linear(DIRECTORY_TRACK, 1) * SECTOR;
        image[dir..dir + SECTOR].fill(0);
        assert_eq!(image.len(), D64_LEN);
        image
    }

    /// A Version 3 story whose header checksum is correct for its own bytes.
    fn fake_story(len: usize) -> Vec<u8> {
        let mut story = vec![0u8; len];
        story[0] = 3;
        let mut word = |o: usize, v: u16| story[o..o + 2].copy_from_slice(&v.to_be_bytes());
        word(0x04, 0x0400); // high memory
        word(0x06, 0x0500); // initial program counter
        word(0x08, 0x0300); // dictionary
        word(0x0a, 0x0100); // objects
        word(0x0c, 0x0200); // globals
        word(0x0e, 0x0280); // static memory base
        word(0x1a, (len / 2) as u16); // file length, the Version 3 unit
        story[0x12..0x18].copy_from_slice(b"840914");
        // A body, so the checksum below is a number rather than zero — which
        // `verified` reads as "not recorded" and skips, and which this format
        // must never accept, having nothing else to identify a run of sectors by.
        for (i, byte) in story.iter_mut().enumerate().skip(64) {
            *byte = (i % 251) as u8;
        }
        let sum = story[64..].iter().fold(0u16, |a, &b| a.wrapping_add(u16::from(b)));
        story[0x1c..0x1e].copy_from_slice(&sum.to_be_bytes());
        story
    }

    // ── Geometry, without a fixture ──────────────────────────────────────────

    /// The 1541's four speed zones, and the 683 sectors they add up to. Pinned
    /// because every offset in this module is derived from them.
    #[test]
    fn the_geometry_is_the_thirty_five_track_fifteen_forty_ones() {
        assert_eq!(sectors_per_track(1), 21);
        assert_eq!(sectors_per_track(17), 21);
        assert_eq!(sectors_per_track(18), 19);
        assert_eq!(sectors_per_track(24), 19);
        assert_eq!(sectors_per_track(25), 18);
        assert_eq!(sectors_per_track(30), 18);
        assert_eq!(sectors_per_track(31), 17);
        assert_eq!(sectors_per_track(35), 17);
        assert_eq!(sectors_per_track(36), 0, "off the disk");
        assert_eq!(total_sectors(), 683);
        assert_eq!(total_sectors() * SECTOR, D64_LEN, "174,848 bytes");
        // The two places this module names outright.
        assert_eq!(linear(18, 0), 357, "the BAM");
        assert_eq!(linear(3, 0), 42, "Trinity's header");
        assert_eq!(linear(5, 0), 84, "Hitchhiker's header");
        for index in 0..total_sectors() {
            let (t, s) = track_sector(index);
            assert_eq!(linear(t, s), index, "linear and track_sector are inverses");
        }
    }

    /// A synthetic disk round-trips, and finding its story costs the reader the
    /// same search the real ones cost — so a broken table fails here with no
    /// fixture on disk at all, which is what CI runs.
    #[test]
    fn a_synthetic_commodore_disk_round_trips_without_a_fixture() {
        let story = fake_story(4096);
        let raw = sample_disk(&story);
        assert_eq!(raw.len(), D64_LEN);
        assert!(D64::looks_like_d64(&raw));
        let disk = D64::mount(raw).expect("it mounts");
        assert_eq!(disk.entry_name().as_deref(), Some("T3/S0"));
        assert_eq!(disk.volume_name(), Some("SAMPLE"));
        assert_eq!(disk.story().expect("a story").1, story, "byte-exact off the disk");

        // FALSIFICATION, and one CI can run: declare a checksum one greater than
        // the truth — a byte below `$40`, and therefore outside the sum it
        // describes — and the disk stops being one of these.
        let mut wrong = story.clone();
        wrong[0x1d] = wrong[0x1d].wrapping_add(1);
        assert!(!D64::looks_like_d64(&sample_disk(&wrong)), "a wrong checksum is not a story");
    }

    /// A formatted but empty disk is a 1541 image and is not an Infocom release
    /// — the sniff needs a story or a disk full of data outside its filesystem,
    /// and a blank one is neither.
    /// The 1987 press's layout, synthetically — so the third [`Plan`] is
    /// covered by something CI can run (SQ-1095).
    ///
    /// The real specimen it was measured off is a `.g64` in gitignored
    /// `stories/`, and a plan whose only witness skips vacuously is a plan that
    /// quietly stops working. This lays a story out the way *Plundered Hearts*
    /// does — seventeen sectors a track from track 5, track 17 skipped, track 18
    /// starting after the BAM and the directory sector — and asks the ordinary
    /// mount for it back. The two plans ahead of it in [`PLANS`] both reach for
    /// a different set of sectors and neither sums to the story's own checksum,
    /// so this passes only because the third one exists.
    #[test]
    fn a_disk_laid_out_seventeen_sectors_a_track_round_trips_without_a_fixture() {
        let story = fake_story(17 * SECTOR * 3);
        let mut image = blank_disk();
        let mut indices = Vec::new();
        for track in 5..=TRACKS {
            if track == 17 {
                continue;
            }
            let base = if track == DIRECTORY_TRACK { 2 } else { 0 };
            indices.extend((base..base + 17).map(|sector| linear(track, sector)));
        }
        for (chunk, &index) in story.chunks(SECTOR).zip(&indices) {
            let at = index * SECTOR;
            image[at..at + SECTOR].fill(0);
            image[at..at + chunk.len()].copy_from_slice(chunk);
        }
        assert!(D64::looks_like_d64(&image));
        let disk = D64::mount(image).expect("it mounts");
        assert_eq!(disk.entry_name().as_deref(), Some("T5/S0"));
        assert_eq!(disk.story().expect("a story").1, story, "byte-exact off the disk");
    }

    #[test]
    fn an_empty_formatted_disk_is_not_an_infocom_release() {
        let blank = blank_disk();
        assert!(bam_is_sane(&blank), "it really is a 1541 image");
        assert!(directory_reads(&blank), "with a readable, empty directory");
        assert_eq!(raw_sectors(&blank), 0, "and nothing outside the filesystem");
        assert!(!D64::looks_like_d64(&blank));
        assert_eq!(D64::mount(blank).err(), Some(D64Error::NotAD64));
    }

    /// **An ordinary Commodore disk mounts and lists its files, and reports no
    /// Z-code story** (SQ-1458, replacing SQ-0869's refusal).
    ///
    /// It used to be refused outright, on the reading that a disk whose
    /// directory reads and whose data its BAM accounts for is somebody's BASIC
    /// program collection rather than an Infocom press. Both halves of that are
    /// still true and the conclusion was still wrong: the Scott Adams Commodore
    /// releases are exactly such disks, and refusing them put a whole family of
    /// games outside this crate's one disk seam. What has not changed is the
    /// second half — an ordinary disk is never *misread* as a game, and this
    /// pins that as well as the listing.
    #[test]
    fn an_ordinary_commodore_disk_mounts_and_lists_its_files() {
        let mut image = blank_disk();
        // One PRG at track 1 sector 0, marked used in the BAM and listed in the
        // directory — which is what an ordinary disk looks like and what none of
        // the three Infocom disks does.
        let at = linear(1, 0) * SECTOR;
        image[at..at + SECTOR].fill(0x20);
        image[at] = 0;
        image[at + 1] = 0xff;
        let bam = linear(DIRECTORY_TRACK, 0) * SECTOR;
        image[bam + 4] = 20;
        image[bam + 5] = 0xfe;
        let dir = linear(DIRECTORY_TRACK, 1) * SECTOR;
        image[dir + 2] = 0x82; // closed PRG
        image[dir + 3] = 1; // first sector: track 1
        image[dir + 4] = 0; // sector 0
        image[dir + 5..dir + 21].copy_from_slice(b"HELLO\xa0\xa0\xa0\xa0\xa0\xa0\xa0\xa0\xa0\xa0\xa0");
        image[dir + 30] = 1; // one block
        assert!(directory_reads(&image), "its directory reads");
        assert!(raw_sectors(&image) < RAW_SECTOR_FLOOR, "and its BAM accounts for its data");

        assert!(D64::looks_like_d64(&image), "it is a Commodore disk");
        let disk = D64::mount(image).expect("and it mounts");
        assert_eq!(disk.file_count(), 1);
        assert_eq!(disk.files().len(), 1);
        assert_eq!(disk.files()[0].name, "HELLO");
        assert_eq!(disk.files()[0].file_type, CbmFileType::Prg);
        assert_eq!(disk.files()[0].blocks, 1);
        // The last block declares 0xFF used bytes plus one, so 254 of them.
        assert_eq!(disk.read_named("hello").map(|b| b.len()), Some(254), "case-insensitive");
        assert_eq!(disk.contents().len(), 1);
        // …and there is no Z-code on it, which is the half that has not changed.
        assert_eq!(disk.story(), None);
        assert_eq!(disk.entry_name(), None);
    }

    /// The four filters that keep the decorative entries an Infocom press
    /// carries out of the listing — each one said on its own, because each one
    /// is a real entry on a real disk in `stories/`.
    #[test]
    fn a_decorative_directory_entry_is_not_a_file() {
        // (type byte, blocks, what it is on the shelf)
        let cases: [(u8, u16, &str); 4] = [
            (0x80, 3, "a closed DEL — Zork I's `>GOLDEN EDITION<` banner"),
            (0x82, 0, "a closed PRG claiming no blocks — Zork I's own `ZORK I`"),
            (0x02, 1, "an open PRG, whose chain was never finished"),
            (0x84, 1, "a closed REL, which is not a plain chain at all"),
        ];
        for (kind, blocks, what) in cases {
            let mut image = blank_disk();
            let at = linear(1, 0) * SECTOR;
            image[at] = 0;
            image[at + 1] = 0xff;
            let dir = linear(DIRECTORY_TRACK, 1) * SECTOR;
            image[dir + 2] = kind;
            image[dir + 3] = 1;
            image[dir + 4] = 0;
            image[dir + 5..dir + 21].copy_from_slice(b"HELLO\xa0\xa0\xa0\xa0\xa0\xa0\xa0\xa0\xa0\xa0\xa0");
            image[dir + 30..dir + 32].copy_from_slice(&blocks.to_le_bytes());
            assert!(directory(&image).is_empty(), "{what}");
            assert!(!D64::looks_like_d64(&image), "{what}");
        }
    }

    /// PETSCII, at the two edges the corpus actually has.
    #[test]
    fn a_cbm_name_reads_the_petscii_the_shelf_uses() {
        let pad = |s: &[u8]| {
            let mut v = s.to_vec();
            v.resize(16, 0xa0);
            v
        };
        assert_eq!(cbm_name(&pad(b"WIZARD OF AKYRZ")).as_deref(), Some("WIZARD OF AKYRZ"));
        assert_eq!(cbm_name(&pad(b"BOOT")).as_deref(), Some("BOOT"));
        // Shifted-mode upper case: `QUESTPR1.D64`'s `C1 2E D3`.
        assert_eq!(cbm_name(&pad(&[0xc1, 0x2e, 0xd3])).as_deref(), Some("A.S"));
        // A graphics character has no letter behind it.
        assert_eq!(cbm_name(&pad(&[0xa3; 4])).as_deref(), Some("????"));
        assert_eq!(cbm_name(&[0xa0; 16]), None, "an all-padding name is not a name");
        assert_eq!(cbm_name(&pad(b"   ")), None, "nor is one that trims to nothing");
    }

    /// Nothing of another size is claimed, however story-like. 174,848 bytes is
    /// the 35-track 1541 geometry and this reader claims exactly it.
    #[test]
    fn only_the_thirty_five_track_geometry_is_claimed() {
        assert!(!D64::looks_like_d64(&[]));
        assert!(!D64::looks_like_d64(&vec![0u8; D64_LEN - 1]));
        assert!(!D64::looks_like_d64(&vec![0u8; D64_LEN + 1]));
        assert!(!D64::looks_like_d64(&vec![0u8; 174_848 + 5 * 683]), "the 40-track extension");
        // Right size, no BAM.
        assert!(!D64::looks_like_d64(&vec![0u8; D64_LEN]));
    }

    /// The stop rule, stated on its own: a press's data ends at the first block
    /// the `FORMAT` command left behind.
    #[test]
    fn a_read_stops_at_the_first_never_written_block() {
        let mut blank = [FORMATTED_BLANK.1; SECTOR];
        blank[0] = FORMATTED_BLANK.0;
        assert!(never_written(&blank));
        assert!(!never_written(&[0u8; SECTOR]), "zeroed is not formatted-blank");
        let mut nearly = blank;
        nearly[255] = 0;
        assert!(!never_written(&nearly));

        let raw = sample_disk(&fake_story(4096));
        // Sixteen sectors of story from track 3 sector 0, then the format filler.
        assert_eq!(payload(&raw, Plan::Dense, linear(3, 0)).len(), 4096);
    }

    // ── The fixtures ─────────────────────────────────────────────────────────

    fn stories_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories")
    }

    fn fixture(name: &str) -> Option<Vec<u8>> {
        std::fs::read(stories_dir().join(name)).ok()
    }

    const HITCHHIKERS: &str = "Hitchhikers_Guide_to_the_Galaxy_The_1984_Infocom.d64";
    const TRINITY_1: &str = "TRINITY1.D64";
    const TRINITY_2: &str = "TRINITY2.D64";

    /// **The single-disk fixture, end to end.** *Hitchhiker's* release 47 serial
    /// 840914, off the 1984 Commodore press, whose story is 16 sectors per track
    /// from track 5.
    #[test]
    fn the_hitchhikers_disk_yields_release_forty_seven() {
        let Some(raw) = fixture(HITCHHIKERS) else {
            eprintln!("SKIP: no {HITCHHIKERS}");
            return;
        };
        assert_eq!(raw.len(), D64_LEN, "35-track 1541");
        // The DOS version bytes are `TG`, not the standard `2A` — a mastering
        // marker, and evidence that the sniff must not look for `2A`.
        assert_eq!(&sector_at(&raw, linear(18, 0))[0xa5..0xa7], b"TG");
        assert!(D64::looks_like_d64(&raw));
        let disk = D64::mount(raw).expect("it mounts");
        assert_eq!(disk.volume_name(), Some("HITCHHIKER GUIDE"));
        assert_eq!(disk.entry_name().as_deref(), Some("T5/S0"), "track 5, sector 0");
        let (name, story) = disk.story().expect("a story");
        assert_eq!(name, "T5/S0");
        assert_eq!(story.len(), 112_622, "56,311 length units × 2 (ZMSD §11.1.6)");
        assert_eq!(story[0], 3, "Version 3");
        assert_eq!(u16::from_be_bytes([story[2], story[3]]), 47, "release 47");
        assert_eq!(&story[0x12..0x18], b"840914", "serial 840914");
        assert_eq!(u16::from_be_bytes([story[0x1c], story[0x1d]]), 0x2235, "checksum $2235");
    }

    /// **Neither side of *Trinity* is a game**, and that is the finding, not a
    /// failure: SIDE 1 carries the header and 344 of 1,024 sectors, SIDE 2
    /// carries the rest and no header at all.
    #[test]
    fn neither_side_of_trinity_holds_a_story_on_its_own() {
        for (name, side) in [(TRINITY_1, 1), (TRINITY_2, 2)] {
            let Some(raw) = fixture(name) else {
                eprintln!("SKIP: no {name}");
                continue;
            };
            assert_eq!(raw.len(), D64_LEN, "{name}");
            assert!(D64::looks_like_d64(&raw), "{name}: still a Commodore Infocom disk");
            assert!(!directory_reads(&raw), "{name}: its directory does not read");
            let disk = D64::mount(raw).expect("it mounts");
            assert!(disk.story().is_none(), "{name}: no whole story on one side");
            if side == 1 {
                assert_eq!(disk.volume_name(), Some("TRINITY"));
            } else {
                assert_eq!(disk.volume_name(), Some("SIDE 2"), "and it says so");
                assert!(disk.entry_name().is_none(), "a continuation side has no header");
            }
        }
    }

    /// **The two-disk fixture, end to end** — and the arithmetic that makes it
    /// two disks. Version 4 counts its length in units of four (ZMSD §11.1.6), so
    /// *Trinity* is 262,064 bytes and one 174,848-byte floppy cannot hold it.
    #[test]
    fn trinity_assembles_across_both_sides() {
        let (Some(one), Some(two)) = (fixture(TRINITY_1), fixture(TRINITY_2)) else {
            eprintln!("SKIP: no {TRINITY_1} / {TRINITY_2}");
            return;
        };
        let (name, story) =
            story_across(&[one.clone(), two.clone()]).expect("a story across the set");
        assert_eq!(name, "T3/S0", "the header is on SIDE 1, track 3 sector 0");
        assert_eq!(story.len(), 262_064, "65,516 length units × 4");
        assert!(story.len() > D64_LEN, "and that is more than one 1541 disk holds");
        assert_eq!(story[0], 4, "Version 4");
        assert_eq!(u16::from_be_bytes([story[2], story[3]]), 12, "release 12");
        assert_eq!(&story[0x12..0x18], b"860926", "serial 860926");
        assert_eq!(u16::from_be_bytes([story[0x1c], story[0x1d]]), 0x16ab, "checksum $16AB");

        // …and the caller's ordering does not matter, because only one side
        // carries a header. Opening SIDE 2 must give the same game.
        assert_eq!(story_across(&[two, one]).expect("the same story").1, story);
    }

    /// A side on its own is not a set, and a set of one is not a story. The
    /// mechanism must not manufacture a game out of half of one.
    #[test]
    fn one_side_is_never_a_set() {
        let Some(one) = fixture(TRINITY_1) else {
            eprintln!("SKIP: no {TRINITY_1}");
            return;
        };
        assert!(story_across(std::slice::from_ref(&one)).is_none());
        assert!(story_across(&[]).is_none());
        // Two copies of the SAME side do not complete each other either.
        assert!(story_across(&[one.clone(), one]).is_none());
    }

    /// **Where the sectors are**, pinned as measured — the layout this whole
    /// module is about, in numbers a later change cannot move quietly.
    #[test]
    fn the_two_presses_spend_their_sectors_differently() {
        if let Some(raw) = fixture(HITCHHIKERS) {
            // 16 of each track's 21 sectors: `s16`..`s20` were never written.
            for track in [5, 9, 16] {
                for sector in 0..16 {
                    assert!(
                        !never_written(sector_at(&raw, linear(track, sector))),
                        "t{track}s{sector} carries story"
                    );
                }
                for sector in 16..21 {
                    assert!(
                        never_written(sector_at(&raw, linear(track, sector))),
                        "t{track}s{sector} was never written"
                    );
                }
            }
            assert_eq!(payload(&raw, Plan::Sixteen, linear(5, 0)).len(), 440 * SECTOR);
        } else {
            eprintln!("SKIP: no {HITCHHIKERS}");
        }
        if let Some(raw) = fixture(TRINITY_1) {
            // Every sector, and the story stops where the disk stops being written.
            for sector in 16..21 {
                assert!(!never_written(sector_at(&raw, linear(5, sector))), "t5s{sector}");
            }
            assert!(never_written(sector_at(&raw, linear(19, 11))), "the data ends at t19s11");
            assert_eq!(payload(&raw, Plan::Dense, linear(3, 0)).len(), 344 * SECTOR);
        } else {
            eprintln!("SKIP: no {TRINITY_1}");
        }
        if let Some(raw) = fixture(TRINITY_2) {
            assert!(never_written(sector_at(&raw, linear(35, 15))), "SIDE 2 ends at t35s14");
            assert_eq!(payload(&raw, Plan::Dense, 0).len(), 680 * SECTOR);
        } else {
            eprintln!("SKIP: no {TRINITY_2}");
        }
    }

    /// **How the two plans are actually told apart**, said with the numbers
    /// rather than assumed — because they are *not* told apart the way the
    /// module header's third oracle suggests, and that is worth being exact
    /// about.
    ///
    /// On both fixtures the wrong plan is refused on **length**, long before any
    /// structure is reachable: a plan that skips five sectors of every track
    /// walks off the end of a disk that uses them, and a plan that uses them all
    /// stops dead at the first of the gaps the other press leaves.
    ///
    /// ```text
    ///   Hitchhiker's   sixteen  112,640 bytes ✓ (needs 112,622)   dense    4,096 ✗
    ///   Trinity        dense    262,144 bytes ✓ (needs 262,064)   sixteen 195,072 ✗
    /// ```
    ///
    /// *Hitchhiker's* under the dense plan is the sharper of the two: it stops
    /// after **sixteen sectors**, at track 5 sector 16, which is the very gap
    /// that identifies the 1984 press.
    #[test]
    fn the_wrong_plan_runs_out_before_it_can_be_mistaken_for_a_story() {
        if let Some(raw) = fixture(HITCHHIKERS) {
            assert_eq!(payload(&raw, Plan::Sixteen, linear(5, 0)).len(), 112_640, "the right plan");
            assert_eq!(
                payload(&raw, Plan::Dense, linear(5, 0)).len(),
                16 * SECTOR,
                "the dense plan stops at t5s16, the gap that identifies this press"
            );
        } else {
            eprintln!("SKIP: no {HITCHHIKERS}");
        }
        if let (Some(one), Some(two)) = (fixture(TRINITY_1), fixture(TRINITY_2)) {
            let across = |plan| {
                let mut v = payload(&one, plan, linear(3, 0));
                v.extend_from_slice(&payload(&two, plan, 0));
                v.len()
            };
            assert_eq!(across(Plan::Dense), 262_144, "the right plan, and 262,064 is needed");
            assert!(across(Plan::Sixteen) < 262_064, "the sixteen plan runs out: {}", across(Plan::Sixteen));
            assert_eq!(across(Plan::Sixteen), 195_072);
        } else {
            eprintln!("SKIP: no Trinity sides");
        }
    }

    /// **The structural oracle the checksum cannot be** (SQ-0868's correction).
    /// A byte sum is blind to order, so the dictionary is decoded too: under the
    /// plan that verifies, the header's own pointer lands on a textbook
    /// dictionary, whose internal consistency no wrongly-ordered run of sectors
    /// would reproduce.
    #[test]
    fn the_dictionary_decodes_where_the_header_points() {
        // (separator count, separators, entry length, entry count, end address).
        let dictionary = |story: &[u8]| {
            let at = usize::from(u16::from_be_bytes([story[8], story[9]]));
            let n = usize::from(story[at]);
            let seps = story[at + 1..at + 1 + n].to_vec();
            let entry = usize::from(story[at + 1 + n]);
            let count =
                usize::from(u16::from_be_bytes([story[at + 2 + n], story[at + 3 + n]]));
            (n, seps, entry, count, at + 4 + n + entry * count)
        };

        if let Some(raw) = fixture(HITCHHIKERS) {
            let disk = D64::mount(raw).expect("it mounts");
            let (_, story) = disk.story().expect("a story");
            let (n, seps, entry, count, end) = dictionary(&story);
            assert_eq!(n, 3, "three word separators");
            assert_eq!(seps, b",.\"", "Infocom's Version 3 separators");
            assert_eq!(entry, 7, "seven-byte entries, the Version 3 size");
            assert_eq!(count, 969, "969 words");
            assert_eq!(end, 20_394, "ending exactly at the high-memory mark");
            assert_eq!(end, usize::from(u16::from_be_bytes([story[4], story[5]])));

            // …and the same pointer read out of a wrongly-ordered story is
            // nonsense, which is the property that makes this an oracle at all.
            // Reversing the sector order preserves the byte SUM exactly and
            // destroys the structure.
            let mut reversed: Vec<u8> = Vec::with_capacity(story.len());
            for chunk in story.chunks(SECTOR).rev() {
                reversed.extend_from_slice(chunk);
            }
            let (n, _, entry, _, _) = dictionary(&reversed);
            assert_ne!((n, entry), (3, 7), "a reordering must not decode as a dictionary");
        } else {
            eprintln!("SKIP: no {HITCHHIKERS}");
        }

        if let (Some(one), Some(two)) = (fixture(TRINITY_1), fixture(TRINITY_2)) {
            let (_, story) = story_across(&[one.clone(), two]).expect("a story");
            let (n, seps, entry, count, end) = dictionary(&story);
            assert_eq!(n, 5, "five word separators");
            assert_eq!(seps, b".,\"!?", "Trinity's Version 4 separators");
            assert_eq!(entry, 9, "nine-byte entries, the Version 4 size");
            assert_eq!(count, 2120, "2,120 words");
            // **63,423 — which is the mark the REFERENCE build declares at `$04`
            // and this press does not.** The dictionary is the last thing below
            // high memory, so where it ends is where high memory begins, and it
            // ends where `stories/trinity-r12-s860926.z4` says it should. That
            // is independent confirmation that the body assembled here is the
            // standard one and that only the header's own `$04` was lowered, for
            // a machine that had to page nearly all of it (see the module head).
            assert_eq!(end, 63_423, "ending at the reference build's high-memory mark");
            assert_eq!(
                usize::from(u16::from_be_bytes([story[4], story[5]])),
                22_527,
                "while this press declares a third of that, and is right to"
            );
        } else {
            eprintln!("SKIP: no Trinity sides");
        }
    }

    /// **The order-sensitive oracle** (SQ-0868's method, applied here). The
    /// checksum is a sum and cannot see a permutation; FNV-1a over the whole
    /// extracted story can, and does.
    ///
    /// **It is a pin and not evidence** — a fingerprint of one's own output is
    /// exactly the test that passes when the implementation is wrong. What makes
    /// these two values trustworthy is that the bytes behind them were
    /// established three other ways first: the header checksum over their
    /// multiset, the dictionary decoding out of them, and — for *Trinity* — a
    /// byte-for-byte comparison against `stories/trinity-r12-s860926.z4`, an
    /// independent dump of the same build.
    #[test]
    fn the_extracted_stories_are_the_same_sectors_in_the_same_order() {
        if let Some(raw) = fixture(HITCHHIKERS) {
            let disk = D64::mount(raw).expect("it mounts");
            let (_, story) = disk.story().expect("a story");
            assert_eq!(story.len(), 112_622);
            assert_eq!(fnv1a(&story), 0x6D75_7083_B550_A454, "440 sectors, in order");
        } else {
            eprintln!("SKIP: no {HITCHHIKERS}");
        }
        if let (Some(one), Some(two)) = (fixture(TRINITY_1), fixture(TRINITY_2)) {
            let (_, story) = story_across(&[one, two]).expect("a story");
            assert_eq!(story.len(), 262_064);
            assert_eq!(fnv1a(&story), 0x4117_87F4_7CB1_FAB6, "1,024 sectors, in order");
        } else {
            eprintln!("SKIP: no Trinity sides");
        }
    }

    /// **The falsification this quest was asked for**: perturb the layout in a
    /// way the byte SUM cannot see, and confirm the order-sensitive test does.
    ///
    /// Swapping two whole sectors of a disk leaves every byte of the story
    /// present exactly once, so the header checksum is unchanged and
    /// [`crate::infocom_packed::verified`] still accepts the reassembly — the
    /// story still says Version 3, release 47, serial 840914, and still sums to
    /// `$2235`. The fingerprint is what refuses it.
    #[test]
    fn a_sector_swap_passes_the_checksum_and_fails_the_fingerprint() {
        let Some(raw) = fixture(HITCHHIKERS) else {
            eprintln!("SKIP: no {HITCHHIKERS}");
            return;
        };
        let mut swapped = raw.clone();
        // Two sectors both well inside the story, so the multiset of bytes the
        // checksum covers is identical and only their ORDER differs.
        let (a, b) = (linear(6, 3) * SECTOR, linear(12, 9) * SECTOR);
        for i in 0..SECTOR {
            swapped.swap(a + i, b + i);
        }
        let good = D64::mount(raw).expect("it mounts").story().expect("a story").1;
        let bad = D64::mount(swapped).expect("it still mounts").story().expect("still a story").1;

        assert_ne!(good, bad, "the premise: the two differ");
        assert_eq!(bad.len(), good.len(), "same length");
        assert_eq!(bad[0], 3, "still Version 3");
        assert_eq!(u16::from_be_bytes([bad[2], bad[3]]), 47, "still release 47");
        assert_eq!(&bad[0x12..0x18], b"840914", "still serial 840914");
        let sum = |s: &[u8]| s[64..].iter().fold(0u16, |a, &b| a.wrapping_add(u16::from(b)));
        assert_eq!(sum(&bad), 0x2235, "AND STILL SUMS TO ITS OWN DECLARED CHECKSUM");
        assert_eq!(sum(&bad), sum(&good), "a byte sum cannot see a reordering");
        assert_ne!(fnv1a(&bad), fnv1a(&good), "and the fingerprint must");
        assert_ne!(fnv1a(&bad), 0x6D75_7083_B550_A454, "which is what the pin above catches");
    }

    /// FNV-1a 64, hand-rolled — `blorb` takes no dependencies, and the property
    /// wanted here is only that reordering the input changes the output.
    fn fnv1a(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    /// The name a caller is shown is a name it can ask back for — the property
    /// every format in this crate has to have, because it is the `--pictures`
    /// door.
    #[test]
    fn what_the_disk_lists_it_will_also_hand_over() {
        let Some(raw) = fixture(HITCHHIKERS) else {
            eprintln!("SKIP: no {HITCHHIKERS}");
            return;
        };
        let disk = D64::mount(raw).expect("it mounts");
        let contents = disk.contents();
        // Two entries, of two different kinds, and both real (SQ-1458): the
        // 552-byte BASIC loader the press keeps in a CBM file — the `SYS(2063)`
        // stub this module's header quotes — and the story, which is raw sectors
        // no directory can name.
        assert_eq!(contents.len(), 2);
        assert_eq!(disk.file_count(), 2);
        assert_eq!(contents[0].0, "THE HITCHHIKER'S");
        assert_eq!(contents[0].1.len(), 552);
        assert_eq!(&contents[0].1[..2], &[0x01, 0x08], "loads at $0801, like every C64 PRG");
        assert_eq!(contents[1].0, "T5/S0", "where the story is: this medium's only other name");
        for (name, bytes) in &contents {
            assert_eq!(disk.read_named(name).as_ref(), Some(bytes), "{name}");
            assert_eq!(
                disk.read_named(&name.to_ascii_lowercase()).as_ref(),
                Some(bytes),
                "{name}: case-insensitive"
            );
        }
        assert_eq!(disk.read_named("HITCHHIKER GUIDE"), None, "the volume name is not a file");
        assert_eq!(disk.read_named("nothing at all"), None);
    }

    /// **The Scott Adams Commodore releases** — the disks SQ-1458's third sniff
    /// arm exists for. Ordinary CBM DOS disks of uncompressed `PRG` files, and
    /// every one of them byte-identical to the copy extracted beside the image.
    /// `stories/` is gitignored, so this skips vacuously on CI.
    #[test]
    fn the_mysterious_adventures_disks_list_their_program_files() {
        let sets: [(&str, &str, &[&str]); 2] = [
            ("MYSTADV1.D64", "HOWARTH I", &[
                "BOOT", "BATON", "TIME MACHINE", "ARROW I", "ARROW II", "PULSAR 7", "CIRCUS",
            ]),
            ("MYSTADV2.D64", "HOWARTH II", &[
                "BOOT", "EXPERIMENT", "WIZARD OF AKYRZ", "PERSEUS", "INDIANS", "WAXWORKS",
            ]),
        ];
        let dir = stories_dir().join("scott-dialects/c64");
        let mut seen = 0;
        for (image, label, names) in sets {
            let Ok(raw) = std::fs::read(dir.join(image)) else {
                eprintln!("SKIP: no {image}");
                continue;
            };
            seen += 1;
            assert_eq!(raw.len(), D64_LEN, "{image}");
            assert!(D64::looks_like_d64(&raw), "{image} is a Commodore disk");
            let disk = D64::mount(raw).expect("it mounts");
            assert_eq!(disk.volume_name(), Some(label), "{image}");
            let listed: Vec<&str> = disk.files().iter().map(|f| f.name.as_str()).collect();
            assert_eq!(listed, names, "{image}");
            assert_eq!(disk.file_count(), names.len(), "{image}: and no raw-sector story");
            assert_eq!(disk.story(), None, "{image}: nothing here is Z-code");

            for file in disk.files() {
                assert_eq!(file.file_type, CbmFileType::Prg, "{image}: {}", file.name);
                // The extraction beside the image was made by the same public
                // walk this module now does, and keeps the two load-address
                // bytes — so a difference here is a difference in the walk.
                let beside = dir.join("prg").join(image).join(format!("{}.prg", file.name));
                let Ok(reference) = std::fs::read(&beside) else {
                    eprintln!("SKIP: no {}", beside.display());
                    continue;
                };
                assert_eq!(file.bytes, reference, "{image}: {}", file.name);
                assert_eq!(disk.read_named(&file.name).as_ref(), Some(&reference), "{}", file.name);
            }
            // The eleven game files load at $4000 and the two BOOTs at $0801,
            // which is what the specimen README records and what makes the
            // kept load-address bytes worth keeping.
            for file in disk.files() {
                let load = u16::from_le_bytes([file.bytes[0], file.bytes[1]]);
                let want = if file.name == "BOOT" { 0x0801 } else { 0x4000 };
                assert_eq!(load, want, "{image}: {}", file.name);
            }
        }
        if seen == 0 {
            eprintln!("SKIP: no Commodore Scott Adams disks present");
        }
    }

    /// A Commodore press offers no artwork and says so rather than guessing —
    /// Infocom never pressed a Version 6 game for the machine.
    #[test]
    fn a_commodore_disk_offers_no_artwork() {
        for name in [HITCHHIKERS, TRINITY_1, TRINITY_2] {
            let Some(raw) = fixture(name) else { continue };
            assert!(D64::mount(raw).expect("it mounts").pictures().is_none(), "{name}");
        }
    }

    /// **The whole shelf**: exactly the three files this module names are
    /// Commodore Infocom disks, and nothing else in `stories/` is. A sniff that
    /// grew loose enough to claim a fourth file fails here.
    #[test]
    fn exactly_three_images_in_the_corpus_are_commodore_disks() {
        let Ok(dir) = std::fs::read_dir(stories_dir()) else {
            eprintln!("SKIP: no stories directory");
            return;
        };
        let mut claimed: Vec<String> = Vec::new();
        let mut seen = 0;
        for entry in dir.flatten() {
            let Ok(raw) = std::fs::read(entry.path()) else { continue };
            seen += 1;
            if D64::looks_like_d64(&raw) {
                claimed.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        if seen == 0 {
            eprintln!("SKIP: empty stories directory");
            return;
        }
        claimed.sort();
        assert_eq!(claimed, [HITCHHIKERS, TRINITY_1, TRINITY_2], "exactly three Commodore disks");
    }
}
