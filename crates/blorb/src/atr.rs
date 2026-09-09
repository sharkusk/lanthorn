//! The **Atari 8-bit `.atr` disk image** and the Atari DOS 2 filesystem inside
//! it (SQ-1458).
//!
//! # Sources
//!
//! Public format documentation only, and named here so the numbers below can be
//! checked rather than believed:
//!
//! * The **ATR header** and its geometry — Nick Kennedy's `SIO2PC` container,
//!   documented at <https://www.atarimax.com/jindroush.atari.org/afmtatr.html>
//!   and in the ATR entry of the Atari file-format collection at
//!   <http://atariarchives.org/>. Freely published reference material; no code
//!   was consulted and none is derived here.
//! * The **Atari DOS 2.0S / 2.5 filesystem** — the *Atari DOS 2.0S Reference
//!   Manual* (Atari, 1980, part C016347) and the *Atari DOS 2.5 Manual*, both
//!   scanned at <http://atariarchives.org/>: sector 360 is the volume table of
//!   contents, sectors 361-368 the directory, and each data sector spends its
//!   last three bytes on a link trailer.
//! * This repository's own clean-room summary,
//!   [`docs/internals/scott-dialects-spec.md`](../../../docs/internals/scott-dialects-spec.md)
//!   §7.3, which states the same facts functionally and is where the mastering
//!   constants of the S.A.G.A. releases live.
//!
//! **No GPL interpreter source was read for this module**, in keeping with
//! `docs/internals/clean-room.md`.
//!
//! # The container
//!
//! Sixteen bytes of header, then the sectors back to back:
//!
//! ```text
//!   0x00  u16  signature, 0x0296 little-endian — the bytes `96 02`
//!   0x02  u16  low word of the image size, in SIXTEEN-BYTE PARAGRAPHS
//!   0x04  u16  bytes per sector (128, 256, rarely 512)
//!   0x06  u8   high byte of the paragraph count
//!   0x07  u8   disk flags; bit 0 is conventionally write-protect
//!   0x08  u16  first bad sector
//!   0x0a  6    reserved
//!   0x10       the sectors begin
//! ```
//!
//! Sectors are numbered from **1**, and for a 128-byte image the mapping is
//! uniform: sector *n* is at file offset `16 + (n − 1) × 128`. Sector 360 is at
//! 45,968 and sector 720 ends one byte short of a 92,176-byte file, which is
//! every S.A.G.A. Atari side in `stories/scott-dialects/atari/`.
//!
//! **Only 128-byte sectors are read here**, and that is a refusal rather than an
//! omission. A 256-byte image stores its first three sectors short — the
//! machine's boot loader always reads them in single density — except for the
//! minority that do not, and the only discriminator is arithmetic on the
//! declared paragraph count; and Atari DOS 2 spends only 125 data bytes in a
//! 256-byte sector anyway, which is a quirk no image in the corpus exercises.
//! Nothing here is one, so this reader claims exactly the geometry it can be
//! checked against, which is the rule the rest of this crate follows for a
//! spelling or a shape no medium justifies.
//!
//! # The filesystem, and how much of it these disks still have
//!
//! Atari DOS 2 keeps its volume table of contents at **sector 360** and its
//! directory in **sectors 361-368**, eight sixteen-byte entries each — sixty-four
//! files at most. An entry is:
//!
//! ```text
//!   +0    u8   flags: 0x80 deleted, 0x40 in use, 0x20 locked,
//!                     0x10 written past sector 720 (DOS 2.5), 0x02 DOS 2 file,
//!                     0x01 open for output. **0x00 means never used**, and DOS
//!                     itself stops scanning there.
//!   +1    u16  sectors the file spends, little-endian
//!   +3    u16  its first sector, little-endian
//!   +5    8    filename stem, space-padded
//!   +13   3    extension, space-padded
//! ```
//!
//! A data sector is **125 bytes of file** and then a three-byte trailer:
//! byte 125 carries the **file number** in its top six bits and the top two bits
//! of the next sector in its bottom two; byte 126 is the next sector's low eight
//! bits; byte 127's low seven bits are how many of the 125 are used, and a next
//! sector of 0 ends the chain. The file number is the entry's own index, so a
//! chain that wanders into somebody else's file says so on the first sector —
//! which is the check that makes this reader safe to run over a disk whose
//! directory has been written over.
//!
//! **And most of these disks have been.** The S.A.G.A. Atari sides master their
//! database as a byte range from file offset `0x04C1` to the end (§7.3), which
//! runs straight through sectors 360-368, so the filesystem underneath is
//! partly or wholly gone. Measured over the fourteen sides in `stories/`:
//!
//! ```text
//!   #1 side A   AUTORUN.SYS (25,100 bytes), DOS.SYS (4,875)
//!   #2 side A   AUTORUN.SYS (25,102 bytes), DOS.SYS (4,875)
//!   #6 side A   DOS.SYS (4,875), AUTORUN.SYS (25,100)
//!   the other eleven sides   nothing readable at all
//! ```
//!
//! Nine of the fourteen have `$FF` where their VTOC should be. That is not a
//! reason to refuse them — they are perfectly good ATR images and the game is on
//! them — so the container is what identifies the medium here and the filesystem
//! is read only if it is still there. A side with no readable directory mounts
//! and lists nothing, exactly as a side of a two-disk Commodore release does
//! (see [`crate::d64`]).
//!
//! **Which is why [`Atr::image`] exists, and why `read_named` answers to
//! [`IMAGE_ENTRY`].** The database on these releases is not a file and cannot be
//! reached through the directory; §7.3 addresses it as a **file offset**, so the
//! one thing a loader needs is the bytes it was handed, unshifted. That door is
//! the whole of what this format offers beyond its filesystem, and it is a
//! reserved name rather than a listed entry because it is not a file on the
//! disk — it *is* the disk.

use crate::infocom_pics::InfocomPics;

/// The ATR signature at offset `0x00`, little-endian — the bytes `96 02`.
pub const SIGNATURE: u16 = 0x0296;

/// Bytes of header in front of sector 1.
pub const HEADER: usize = 16;

/// The one sector size this reader claims. See the module docs for why the
/// others are refused rather than guessed at.
pub const SECTOR: usize = 128;

/// Bytes of a 128-byte sector that belong to the file. The last three are the
/// link trailer.
const DATA_BYTES: usize = 125;

/// The sector holding the volume table of contents.
const VTOC_SECTOR: usize = 360;

/// The first directory sector; there are eight, `361..=368`.
const FIRST_DIRECTORY_SECTOR: usize = 361;

/// Directory sectors, and therefore `8 × 8 = 64` entries at most.
const DIRECTORY_SECTORS: usize = 8;

/// Bytes in one directory entry.
const ENTRY: usize = 16;

/// Flag bits an Atari DOS 2 directory entry is allowed to have set. Bits 2 and 3
/// are undefined, and an entry claiming one is a directory sector that has been
/// written over — which is most of this corpus.
const KNOWN_FLAGS: u8 = 0xf3;

/// The entry is in use.
const FLAG_IN_USE: u8 = 0x40;

/// The entry names a deleted file.
const FLAG_DELETED: u8 = 0x80;

/// The reserved name [`Atr::read_named`] answers with the whole image.
///
/// **Not a file, and deliberately not listed by [`Atr::contents`]** — it is the
/// disk itself, offered because the S.A.G.A. releases keep their database
/// outside the filesystem and the dialect spec addresses it by *file* offset.
/// The bytes handed back are the bytes the mount was given, header and all, so
/// every constant in `scott-dialects-spec.md` §7.3 applies to them verbatim and
/// no caller has to know that a `.atr` has sixteen bytes in front of it.
pub const IMAGE_ENTRY: &str = "IMAGE";

/// Errors that can arise while mounting an ATR image.
#[derive(Debug, PartialEq, Eq)]
pub enum AtrError {
    /// Not an ATR image this reader recognises: no signature, or a geometry
    /// that does not agree with the file's own length.
    NotAnAtr,
}

/// One file found in the Atari DOS 2 directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtrEntry {
    /// `STEM.EXT`, or just `STEM` when the extension is blank — the stored 8.3
    /// name with its padding trimmed.
    pub name: String,
    /// Bytes the chain actually yielded, which is what a caller means by size.
    /// The directory's own count is of SECTORS and includes the trailers.
    pub size: usize,
    /// Sectors the directory says the file spends, trailers included.
    pub sectors: usize,
    /// The entry's index, `0..64` — and therefore the **file number** every one
    /// of its data sectors must carry.
    pub file_number: usize,
    /// The first sector of the chain.
    first_sector: usize,
}

/// A mounted Atari 8-bit disk image.
#[derive(Debug)]
pub struct Atr {
    image: Vec<u8>,
    sectors: usize,
    files: Vec<AtrEntry>,
}

/// Is `raw` an ATR image? The signature, plus a paragraph count and sector size
/// that agree with the file's own length.
///
/// Nothing about the filesystem is asked, because nine of the fourteen sides in
/// the corpus no longer have one — see the module docs. What makes these bytes
/// an Atari 8-bit disk is the container.
pub fn looks_like_atr(raw: &[u8]) -> bool {
    geometry(raw).is_some()
}

/// The sector count `raw` declares, when its header is an ATR header whose
/// arithmetic works out against the file it is on.
///
/// The single place the container is recognised: [`looks_like_atr`] and
/// [`Atr::mount`] both go through it, so the two cannot answer differently.
fn geometry(raw: &[u8]) -> Option<usize> {
    if raw.len() < HEADER || u16::from_le_bytes([raw[0], raw[1]]) != SIGNATURE {
        return None;
    }
    if u16::from_le_bytes([raw[4], raw[5]]) as usize != SECTOR {
        return None;
    }
    // The paragraph count is split: a low word at `0x02` and a high byte at
    // `0x06`, which is how the format reaches past 1 MiB.
    let paragraphs =
        usize::from(u16::from_le_bytes([raw[2], raw[3]])) | usize::from(raw[6]) << 16;
    let payload = paragraphs.checked_mul(HEADER)?;
    // The header must describe the file it is on. This is what keeps the
    // two-byte signature from claiming anything that happens to open `96 02`.
    if payload == 0 || !payload.is_multiple_of(SECTOR) || HEADER + payload != raw.len() {
        return None;
    }
    let sectors = payload / SECTOR;
    // A disk with no room for its own directory is not an Atari DOS 2 disk, and
    // a reader that indexed sector 368 on one would be reading past the end.
    (sectors >= FIRST_DIRECTORY_SECTOR + DIRECTORY_SECTORS - 1).then_some(sectors)
}

impl Atr {
    /// Cheap sniff — see [`looks_like_atr`].
    pub fn looks_like_atr(raw: &[u8]) -> bool {
        looks_like_atr(raw)
    }

    /// Open the image and read whatever is left of its directory.
    pub fn mount(image: Vec<u8>) -> Result<Atr, AtrError> {
        let sectors = geometry(&image).ok_or(AtrError::NotAnAtr)?;
        let mut disk = Atr { image, sectors, files: Vec::new() };
        disk.files = disk.directory();
        Ok(disk)
    }

    /// The whole image, header included — see [`IMAGE_ENTRY`].
    pub fn image(&self) -> &[u8] {
        &self.image
    }

    /// How many sectors the header declares.
    pub fn sector_count(&self) -> usize {
        self.sectors
    }

    /// One sector by its **1-based** Atari number, or `None` off the disk.
    ///
    /// The whole of the container's arithmetic, stated once: `16 + (n − 1) × 128`.
    pub fn sector(&self, n: usize) -> Option<&[u8]> {
        if n == 0 || n > self.sectors {
            return None;
        }
        let at = HEADER + (n - 1) * SECTOR;
        Some(&self.image[at..at + SECTOR])
    }

    /// Every file the directory still names, in directory order.
    pub fn files(&self) -> &[AtrEntry] {
        &self.files
    }

    /// Does the volume table of contents read as one? Never used to decide
    /// whether these bytes are a disk — nine of fourteen sides in the corpus
    /// have `$FF` here — and offered because "is this disk's filesystem intact?"
    /// is a question worth being able to ask.
    pub fn vtoc_is_sane(&self) -> bool {
        let Some(vtoc) = self.sector(VTOC_SECTOR) else { return false };
        // Byte 0 is the DOS code (2 for DOS 2.0S and 2.5), then the total and
        // free sector counts, neither of which can exceed the disk.
        let total = usize::from(u16::from_le_bytes([vtoc[1], vtoc[2]]));
        let free = usize::from(u16::from_le_bytes([vtoc[3], vtoc[4]]));
        vtoc[0] == 2 && total <= self.sectors && free <= total
    }

    /// Walk sectors 361-368 and keep the entries that are still entries.
    ///
    /// Three rules, and the difference between them is measured rather than
    /// stylistic (see the module docs):
    ///
    /// * a **zero** flag byte stops the walk, which is Atari DOS's own rule —
    ///   the entry has never been used and neither has anything after it;
    /// * an entry whose flags, name or geometry are **not an entry's** stops it
    ///   too, because at that point the directory sector has been written over
    ///   and nothing beyond is evidence about anything;
    /// * an entry that reads as an entry and whose **chain** does not verify is
    ///   dropped and the walk continues — Strange Odyssey's side A has exactly
    ///   one of those, with `DOS.SYS` and `AUTORUN.SYS` intact behind it.
    fn directory(&self) -> Vec<AtrEntry> {
        let mut out = Vec::new();
        for index in 0..DIRECTORY_SECTORS * (SECTOR / ENTRY) {
            let sector = FIRST_DIRECTORY_SECTOR + index / (SECTOR / ENTRY);
            let Some(block) = self.sector(sector) else { break };
            let at = (index % (SECTOR / ENTRY)) * ENTRY;
            let e = &block[at..at + ENTRY];
            let flags = e[0];
            if flags == 0 {
                break;
            }
            if flags & FLAG_DELETED != 0 {
                continue;
            }
            if flags & !KNOWN_FLAGS != 0 || flags & FLAG_IN_USE == 0 {
                break;
            }
            let sectors = usize::from(u16::from_le_bytes([e[1], e[2]]));
            let first = usize::from(u16::from_le_bytes([e[3], e[4]]));
            if !(1..=self.sectors).contains(&first) || !(1..=self.sectors).contains(&sectors) {
                break;
            }
            let Some(name) = filename(&e[5..16]) else { break };
            let entry =
                AtrEntry { name, size: 0, sectors, file_number: index, first_sector: first };
            // The chain is walked now rather than lazily, because its length is
            // what a listing means by "size" and because a chain that does not
            // verify must not be offered at all.
            let Some(bytes) = self.follow(&entry) else { continue };
            out.push(AtrEntry { size: bytes.len(), ..entry });
        }
        out
    }

    /// Follow one file's sector chain, or `None` when it does not verify.
    ///
    /// The file-number check is what makes this safe on an overwritten disk: a
    /// sector belonging to another file, or to no file, says so in its own
    /// trailer and the read is refused rather than returning a plausible slab of
    /// somebody else's bytes.
    fn follow(&self, entry: &AtrEntry) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let mut seen = vec![false; self.sectors + 1];
        let mut n = entry.first_sector;
        while n != 0 {
            let block = self.sector(n)?;
            if seen[n] {
                return None;
            }
            seen[n] = true;
            let trailer = &block[DATA_BYTES..];
            if usize::from(trailer[0] >> 2) != entry.file_number {
                return None;
            }
            let used = usize::from(trailer[2] & 0x7f);
            if used > DATA_BYTES {
                return None;
            }
            out.extend_from_slice(&block[..used]);
            n = usize::from(trailer[0] & 0x03) << 8 | usize::from(trailer[1]);
        }
        Some(out)
    }

    /// One file's bytes.
    pub fn read(&self, entry: &AtrEntry) -> Option<Vec<u8>> {
        self.follow(entry)
    }

    /// Every file that reads, in directory order.
    pub fn contents(&self) -> Vec<(String, Vec<u8>)> {
        self.files.iter().filter_map(|e| self.read(e).map(|b| (e.name.clone(), b))).collect()
    }

    /// One file by name, case-insensitively — **or the whole image under
    /// [`IMAGE_ENTRY`]**, which is the door the Atari releases need and the
    /// module docs explain.
    pub fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        if name.eq_ignore_ascii_case(IMAGE_ENTRY) {
            return Some(self.image.clone());
        }
        let e = self.files.iter().find(|e| e.name.eq_ignore_ascii_case(name))?;
        self.read(e)
    }

    /// The Z-machine story on this disk, if one of its files is one.
    ///
    /// **Nothing in the corpus is**, and that is the honest state of the
    /// evidence rather than a gap: Infocom's Atari 8-bit presses are not in
    /// `stories/`, and the S.A.G.A. releases here are Scott Adams games whose
    /// database is not Z-code and not a file. The question is answered because
    /// every format here answers it, by this crate's one test for what a story
    /// is, and a disk that carried one would be found.
    pub fn story(&self) -> Option<(String, Vec<u8>)> {
        self.contents().into_iter().find(|(_, b)| crate::adf::looks_like_story(b))
    }

    /// **No artwork, and that is a limit rather than a finding**, in the shape
    /// [`crate::d64`] states it: Infocom pressed no Version 6 game for the Atari
    /// 8-bit, so there is no evidence about where such a disk would keep an
    /// archive, and the S.A.G.A. picture sides are a different format entirely
    /// (`scott-dialects-spec.md` §8.3). Scanning for one would be a guess with no
    /// medium behind it.
    pub fn pictures(&self) -> Option<(String, InfocomPics)> {
        None
    }
}

/// An 8.3 name out of its eleven stored bytes, or `None` when they are not a
/// name at all.
///
/// Atari DOS writes uppercase letters and digits padded with spaces. Requiring
/// every byte to be printable ASCII is what tells a directory entry from the
/// game data written over one, and it is the check that leaves the eleven
/// overwritten sides in the corpus listing nothing instead of listing rubbish.
fn filename(field: &[u8]) -> Option<String> {
    if !field.iter().all(|c| (0x20..0x7f).contains(c)) {
        return None;
    }
    let stem = String::from_utf8_lossy(&field[..8]).trim_end().to_string();
    let ext = String::from_utf8_lossy(&field[8..]).trim_end().to_string();
    if stem.is_empty() {
        return None;
    }
    Some(if ext.is_empty() { stem } else { format!("{stem}.{ext}") })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Sectors on the one geometry every image in the corpus has: single
    /// density, 720 × 128.
    const SECTORS: usize = 720;

    /// Build an ATR image from the format documentation, so this reader's tests
    /// need no fixture.
    ///
    /// `pub(crate)` for [`crate::medium`]'s census, which must be able to
    /// produce a real, mountable volume of every format it names.
    pub(crate) struct DiskBuilder {
        image: Vec<u8>,
        /// The next free data sector. 369 is the first past the directory.
        next: usize,
        entries: usize,
    }

    impl DiskBuilder {
        pub(crate) fn new() -> DiskBuilder {
            let mut image = vec![0u8; HEADER + SECTORS * SECTOR];
            let paragraphs = (SECTORS * SECTOR) / HEADER;
            image[0..2].copy_from_slice(&SIGNATURE.to_le_bytes());
            image[2..4].copy_from_slice(&((paragraphs & 0xffff) as u16).to_le_bytes());
            image[4..6].copy_from_slice(&(SECTOR as u16).to_le_bytes());
            image[6] = (paragraphs >> 16) as u8;
            let mut disk = DiskBuilder { image, next: 369, entries: 0 };
            // A DOS 2 VTOC, so a synthetic disk is the intact case the corpus
            // mostly is not.
            let vtoc = disk.at(VTOC_SECTOR);
            disk.image[vtoc] = 2;
            disk.image[vtoc + 1..vtoc + 3]
                .copy_from_slice(&((SECTORS - 13) as u16).to_le_bytes());
            disk.image[vtoc + 3..vtoc + 5]
                .copy_from_slice(&((SECTORS - 13) as u16).to_le_bytes());
            disk
        }

        fn at(&self, sector: usize) -> usize {
            HEADER + (sector - 1) * SECTOR
        }

        /// Write `data` as a real DOS 2 chain and list it in the directory.
        pub(crate) fn add_file(&mut self, name: &str, data: &[u8]) {
            let number = self.entries;
            let first = self.next;
            let chunks: Vec<&[u8]> =
                if data.is_empty() { vec![&[]] } else { data.chunks(DATA_BYTES).collect() };
            let count = chunks.len();
            for (i, chunk) in chunks.iter().enumerate() {
                let sector = self.next;
                self.next += 1;
                let next = if i + 1 < count { self.next } else { 0 };
                let at = self.at(sector);
                self.image[at..at + chunk.len()].copy_from_slice(chunk);
                self.image[at + DATA_BYTES] = (number as u8) << 2 | (next >> 8) as u8;
                self.image[at + DATA_BYTES + 1] = (next & 0xff) as u8;
                self.image[at + DATA_BYTES + 2] = chunk.len() as u8;
            }
            let (stem, ext) = match name.split_once('.') {
                Some((s, x)) => (s, x),
                None => (name, ""),
            };
            let at = self.at(FIRST_DIRECTORY_SECTOR + number / (SECTOR / ENTRY))
                + (number % (SECTOR / ENTRY)) * ENTRY;
            self.image[at] = FLAG_IN_USE | 0x02;
            self.image[at + 1..at + 3].copy_from_slice(&(count as u16).to_le_bytes());
            self.image[at + 3..at + 5].copy_from_slice(&(first as u16).to_le_bytes());
            self.image[at + 5..at + 16].fill(b' ');
            self.image[at + 5..at + 5 + stem.len()].copy_from_slice(stem.as_bytes());
            self.image[at + 13..at + 13 + ext.len()].copy_from_slice(ext.as_bytes());
            self.entries += 1;
        }

        pub(crate) fn finish(self) -> Vec<u8> {
            self.image
        }
    }

    /// A mountable ATR carrying `files` — [`crate::medium`]'s census sample.
    pub(crate) fn sample_disk(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut disk = DiskBuilder::new();
        for (name, bytes) in files {
            disk.add_file(name, bytes);
        }
        disk.finish()
    }

    /// The header arithmetic, against the numbers §7.3 states outright: the
    /// six-byte opening every side in the corpus wears, 5,760 paragraphs, and a
    /// 92,176-byte file.
    #[test]
    fn the_header_describes_the_file_it_is_on() {
        let image = sample_disk(&[]);
        assert_eq!(image.len(), 92_176);
        assert_eq!(&image[..6], &[0x96, 0x02, 0x80, 0x16, 0x80, 0x00]);
        assert_eq!(geometry(&image), Some(720));
        // Sector 360 at 45,968 and sector 720 ending one byte short of the file,
        // which is §7.3's own worked example.
        let disk = Atr::mount(image).expect("it mounts");
        assert_eq!(HEADER + (360 - 1) * SECTOR, 45_968);
        assert_eq!(disk.sector(360), Some(&disk.image()[45_968..45_968 + SECTOR]));
        assert_eq!(HEADER + (720 - 1) * SECTOR + SECTOR, 92_176);
        assert!(disk.sector(720).is_some());
        assert!(disk.sector(721).is_none());
        assert!(disk.sector(0).is_none(), "sectors are numbered from one");
    }

    /// A header that does not agree with its own file is not a container. This
    /// is what keeps a two-byte signature from claiming arbitrary bytes.
    #[test]
    fn the_signature_alone_is_not_enough() {
        let good = sample_disk(&[]);
        assert!(looks_like_atr(&good));

        let mut truncated = good.clone();
        truncated.truncate(good.len() - 1);
        assert!(!looks_like_atr(&truncated), "the paragraph count no longer fits");

        let mut padded = good.clone();
        padded.push(0);
        assert!(!looks_like_atr(&padded));

        let mut wrong_sector = good.clone();
        wrong_sector[4] = 0x00;
        wrong_sector[5] = 0x01; // 256 bytes per sector
        assert!(!looks_like_atr(&wrong_sector), "only 128-byte sectors are claimed");

        let mut no_signature = good.clone();
        no_signature[0] = 0x95;
        assert!(!looks_like_atr(&no_signature));

        assert!(!looks_like_atr(&[]));
        assert!(!looks_like_atr(&vec![0u8; 92_176]));
        assert_eq!(Atr::mount(vec![0u8; 16]).err(), Some(AtrError::NotAnAtr));
    }

    /// A whole round trip through the filesystem: two files, real chains, read
    /// back byte for byte and by name.
    #[test]
    fn a_synthetic_disk_round_trips_through_its_own_directory() {
        let long: Vec<u8> = (0..3_000u32).map(|i| (i % 251) as u8).collect();
        let image = sample_disk(&[("README", b"hello"), ("STORY.DAT", &long)]);
        let disk = Atr::mount(image).expect("it mounts");
        assert!(disk.vtoc_is_sane());

        let files = disk.files();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].name, "README");
        assert_eq!(files[0].size, 5);
        assert_eq!(files[0].file_number, 0);
        assert_eq!(files[1].name, "STORY.DAT");
        assert_eq!(files[1].size, 3_000);
        assert_eq!(files[1].file_number, 1);
        // 3,000 bytes at 125 to a sector is 24 sectors exactly.
        assert_eq!(files[1].sectors, 24);

        assert_eq!(disk.read_named("story.dat"), Some(long.clone()), "case-insensitive");
        assert_eq!(disk.read_named("README"), Some(b"hello".to_vec()));
        assert_eq!(disk.read_named("nothing"), None);
        let contents = disk.contents();
        assert_eq!(contents.len(), 2);
        assert_eq!(contents[1], ("STORY.DAT".to_string(), long));
    }

    /// The reserved door, on its own: the image comes back with its header, so
    /// §7.3's file offsets mean what they say.
    #[test]
    fn the_whole_image_is_reachable_under_one_reserved_name() {
        let image = sample_disk(&[("A", b"x")]);
        let disk = Atr::mount(image.clone()).expect("it mounts");
        assert_eq!(disk.read_named(IMAGE_ENTRY), Some(image.clone()));
        assert_eq!(disk.read_named("image"), Some(image), "case-insensitive like every name");
        assert!(
            !disk.contents().iter().any(|(n, _)| n == IMAGE_ENTRY),
            "it is the disk, not a file on it"
        );
    }

    /// **A chain that wanders is refused, not returned.** The file-number byte
    /// is the only thing standing between a reader and somebody else's sectors
    /// on a disk whose directory has been written over, which is nine of the
    /// fourteen sides in the corpus.
    #[test]
    fn a_chain_whose_file_number_is_wrong_is_refused() {
        let mut image = sample_disk(&[("A", b"x"), ("B", b"y")]);
        // Point A's entry at B's sector. Everything about the entry still reads;
        // only the trailer disagrees.
        let entry = HEADER + (FIRST_DIRECTORY_SECTOR - 1) * SECTOR;
        image[entry + 3..entry + 5].copy_from_slice(&370u16.to_le_bytes());
        let disk = Atr::mount(image).expect("it still mounts");
        assert_eq!(disk.files().len(), 1, "A is dropped and B survives");
        assert_eq!(disk.files()[0].name, "B");
    }

    /// A directory sector full of game data lists nothing rather than listing
    /// rubbish — the state eleven of the fourteen corpus sides are in.
    #[test]
    fn an_overwritten_directory_lists_nothing() {
        let mut image = sample_disk(&[("A", b"x")]);
        let dir = HEADER + (FIRST_DIRECTORY_SECTOR - 1) * SECTOR;
        image[dir..dir + SECTOR].fill(0xff);
        let disk = Atr::mount(image).expect("it is still an ATR image");
        assert!(disk.files().is_empty());
        assert!(disk.contents().is_empty());
        assert_eq!(disk.story(), None);
        assert_eq!(disk.pictures().map(|_| ()), None);
        // …and the image door still works, which is the whole point of it.
        assert_eq!(disk.read_named(IMAGE_ENTRY).map(|b| b.len()), Some(92_176));
    }

    /// A deleted entry is stepped over; a never-used one stops the walk.
    #[test]
    fn deleted_entries_are_skipped_and_a_zero_flag_ends_the_directory() {
        let mut image = sample_disk(&[("A", b"x"), ("B", b"y")]);
        let entry = HEADER + (FIRST_DIRECTORY_SECTOR - 1) * SECTOR;
        image[entry] = FLAG_DELETED;
        let disk = Atr::mount(image).expect("it mounts");
        assert_eq!(disk.files().len(), 1);
        assert_eq!(disk.files()[0].name, "B");

        // …and nothing past a zero flag is looked at, however entry-like.
        let mut image = sample_disk(&[("A", b"x"), ("B", b"y")]);
        image[entry] = 0;
        let disk = Atr::mount(image).expect("it mounts");
        assert!(disk.files().is_empty());
    }

    /// A file out of the gitignored `stories/`, or `None` with a SKIP note.
    fn fixture(name: &str) -> Option<Vec<u8>> {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/scott-dialects/atari")
            .join(name);
        match std::fs::read(&p) {
            Ok(b) => Some(b),
            Err(_) => {
                eprintln!("SKIP: gitignored fixture missing at {}", p.display());
                None
            }
        }
    }

    /// The fourteen S.A.G.A. sides, and what each one's filesystem is still
    /// worth. Every side is an ATR image and mounts; three of them still have a
    /// directory. `stories/` is gitignored, so this skips vacuously on CI.
    #[test]
    fn every_saga_side_mounts_and_lists_what_is_left_of_its_directory() {
        // (side, the files still readable on it)
        let expected: [(&str, &[(&str, usize)]); 14] = [
            ("SAGA #1 - Adventureland [side A].atr", &[
                ("AUTORUN.SYS", 25_100),
                ("DOS.SYS", 4_875),
            ]),
            ("SAGA #1 - Adventureland [side B].atr", &[]),
            ("SAGA #2 - Pirate Adventure [side A].atr", &[
                ("AUTORUN.SYS", 25_102),
                ("DOS.SYS", 4_875),
            ]),
            ("SAGA #2 - Pirate Adventure [side B].atr", &[]),
            ("SAGA #3 - Mission Impossible [side A].atr", &[]),
            ("SAGA #3 - Mission Impossible [side B].atr", &[]),
            ("SAGA #4 - Voodoo Castle [side A].atr", &[]),
            ("SAGA #4 - Voodoo Castle [side B].atr", &[]),
            ("SAGA #5 - The Count [side A].atr", &[]),
            ("SAGA #5 - The Count [side B].atr", &[]),
            ("SAGA #6 - Strange Odyssey [side A].atr", &[
                ("DOS.SYS", 4_875),
                ("AUTORUN.SYS", 25_100),
            ]),
            ("SAGA #6 - Strange Odyssey [side B].atr", &[]),
            ("SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side A.atr", &[]),
            ("SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side B.atr", &[]),
        ];
        let mut seen = 0;
        for (name, files) in expected {
            let Some(raw) = fixture(name) else { continue };
            seen += 1;
            assert_eq!(raw.len(), 92_176, "{name}");
            assert!(looks_like_atr(&raw), "{name} is an ATR image");
            let disk = Atr::mount(raw.clone()).expect("it mounts");
            assert_eq!(disk.sector_count(), 720, "{name}");
            let listed: Vec<(String, usize)> =
                disk.files().iter().map(|e| (e.name.clone(), e.size)).collect();
            let want: Vec<(String, usize)> =
                files.iter().map(|(n, s)| ((*n).to_string(), *s)).collect();
            assert_eq!(listed, want, "{name}");
            // What is listed, reads back — on every side, empty or not.
            for (entry, bytes) in disk.contents() {
                assert_eq!(disk.read_named(&entry).as_ref(), Some(&bytes), "{name}: {entry}");
            }
            // …and the image door is the same bytes that went in, which is what
            // the dialect spec's file offsets are measured against.
            assert_eq!(disk.read_named(IMAGE_ENTRY), Some(raw), "{name}");
        }
        if seen == 0 {
            eprintln!("SKIP: no Atari S.A.G.A. sides present");
        }
    }

    /// The database is not a file, and this pins the fact rather than working
    /// around it: `scott-dialects-spec.md` §7.3 masters it at **file offset
    /// `0x04F9`**, and the header there is the one the specimen README records
    /// for Adventureland. Reached through [`IMAGE_ENTRY`], which is the whole
    /// reason that door exists.
    #[test]
    fn the_saga_database_is_reached_by_file_offset_and_not_by_name() {
        let Some(raw) = fixture("SAGA #1 - Adventureland [side A].atr") else { return };
        let disk = Atr::mount(raw).expect("it mounts");
        let image = disk.read_named(IMAGE_ENTRY).expect("the image door");
        // Ten little-endian words from 0x04F9, which is what the specimen
        // README records for Adventureland: word length, then the word, action,
        // item, message and room counts, the carry limit, the starting room,
        // the treasure count and the lamp's turns.
        let word = |i: usize| i16::from_le_bytes([image[0x04f9 + 2 * i], image[0x04f9 + 2 * i + 1]]);
        let header: Vec<i16> = (0..10).map(word).collect();
        assert_eq!(header, [3, 69, 169, 65, 75, 33, 6, 11, 13, 125]);

        // …and the directory names no such file. Neither of §7.4's Apple
        // spellings, nor anything else: the only two files on this side are the
        // DOS that boots it and the loader that runs it, so no `read_named` of
        // any name could have reached the header above.
        let listed: Vec<&str> = disk.files().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(listed, ["AUTORUN.SYS", "DOS.SYS"]);
        for name in ["A1.DAT", "DATABASE", "ADVENTURELAND"] {
            assert_eq!(disk.read_named(name), None, "{name}");
        }

        // §7.3's splice, and its arithmetic said out loud: the 128 bytes at file
        // offset 0xB390 are **sector 360**, the volume table of contents, and
        // they sit inside the database's byte range. `16 + 359 × 128 = 45,968 =
        // 0xB390`, and this reader's sector door lands on exactly that.
        assert_eq!(HEADER + 359 * SECTOR, 0xb390);
        assert_eq!(disk.sector(VTOC_SECTOR), Some(&image[0xb390..0xb390 + SECTOR]));
        // On this side the VTOC is still a VTOC, which is why three of the
        // fourteen sides still have a directory at all.
        assert!(disk.vtoc_is_sane());
    }
}
