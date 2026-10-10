//! ISO 9660, the CD-ROM filesystem — and the Apple extension that says which
//! machine each file on it was pressed for (SQ-0871).
//!
//! # Why this format, and why it is not the hybrid CD's answer
//!
//! [`crate::hfs`] already reads a hybrid disc: *Classic Text Adventure
//! Masterpieces of Infocom* is a raw dump whose third Apple partition is a
//! Macintosh volume, and both machines' builds sit inside that one volume. It
//! needs nothing here.
//!
//! *The Lost Treasures of Infocom* I and II are a different construction, and it
//! is the ordinary one for a CD: **no Apple partition map and no HFS volume at
//! all**, just ISO 9660 with Apple's extensions layered into it. Every reader
//! this crate had declined them, so two discs holding 92 and 132 files opened as
//! nothing whatsoever — `zvm-cli` reported "Z-machine version 0 is not
//! supported", which is what happens when a whole disc image is handed to the
//! Z-machine as though it were a story.
//!
//! What they hold is worth the module: between them, *Shogun* and *The
//! Hitchhiker's Guide to the Galaxy*, neither of which is on the Masterpieces
//! disc — and Shogun in Macintosh **and** DOS pressings with all their artwork,
//! so this is new Version 6 coverage rather than more text games.
//!
//! # Layer 1 — the volume descriptors
//!
//! ECMA-119 §8.1: the descriptors begin at logical sector 16 and each is one
//! 2048-byte sector opening with a type byte, the identifier `CD001` and a
//! version byte. Type 1 is the Primary Volume Descriptor, and it carries the
//! volume's name, its logical block size, and the directory record for the root.
//! Type 255 terminates the set.
//!
//! **The block size is read, not assumed.** 2048 is universal and both discs use
//! it, but it is a field and this reads the field.
//!
//! # Layer 2 — directory records
//!
//! ECMA-119 §9.1, and every offset below comes from it: the record length, the
//! extent's first logical block and its length (both stored twice, little-endian
//! then big-endian — the little half is read), the file flags, and a length-
//! prefixed identifier. A directory is walked by reading its extent as a run of
//! these records; `\x00` and `\x01` name itself and its parent and are skipped.
//!
//! Two flag bits matter, and skipping them is what makes the listing come out
//! right rather than doubled:
//!
//! * **§9.1.6 bit 1, Directory** — recurse rather than list.
//! * **§9.1.6 bit 2, Associated File** — this record is an *associated* copy of
//!   the file named beside it, which is exactly how these discs carry Macintosh
//!   **resource forks**. `/MAC/ZORK I` appears twice: 16,810 bytes with the bit
//!   set (the interpreter's resources) and 84,992 bytes without it (the story).
//!   Taking both would list every Macintosh game twice and offer a resource fork
//!   as a story; the flag is the standard's own way of saying which is which, so
//!   no heuristic is needed.
//!
//! # Layer 3 — the Apple extension, and what it is for
//!
//! Apple's *ISO 9660 Extensions* put a `AA` System Use entry after the
//! identifier: signature, length, a system-use ID, then — for ID 2, the HFS
//! form — the Finder type, the Finder creator and the Finder flags. So a file on
//! one of these discs carries the same two four-character codes an HFS catalog
//! record does, and [`crate::medium::machine_from_finder`] answers the machine
//! question off them without either format restating the rule.
//!
//! That is what tells the `MAC/` tree from the `PC/` one (disc 1) and the `DOS/`
//! one (disc 2) — by metadata the publisher wrote, not by matching those folder
//! names, which this crate does not do.
//!
//! # What is NOT here
//!
//! Joliet (a supplementary descriptor with UCS-2 names), Rock Ridge, multi-extent
//! files (§9.1.6 bit 7), and interleaved extents. Neither disc uses any of them,
//! and a reader for a case with no example is a rule that cannot be checked.
//! A raw MODE1/2352 dump carrying only ISO 9660 is likewise absent: the one raw
//! dump in the corpus is read through its HFS partition.

use crate::adf::looks_like_story;
use crate::infocom_pics::InfocomPics;
use crate::source::Source;

/// Where the volume descriptors begin — ECMA-119 §6.2.1, logical sector 16.
const DESCRIPTOR_START: usize = 16 * 2048;
/// The sector size the descriptors themselves are laid out in.
const SECTOR: usize = 2048;
/// `CD001`, the standard identifier every volume descriptor carries.
const MAGIC: &[u8; 5] = b"CD001";
/// Volume descriptor types: 1 primary, 255 terminator (§8).
const VD_PRIMARY: u8 = 1;
const VD_TERMINATOR: u8 = 255;

// Primary Volume Descriptor fields, from the descriptor's start (§8.4).
const PVD_VOLUME_ID: usize = 40; // 32 bytes, d-characters
const PVD_BLOCK_SIZE: usize = 128; // both-endian 16
const PVD_ROOT_RECORD: usize = 156; // a 34-byte directory record

// Directory record fields (§9.1).
const DR_EXTENT: usize = 2; // both-endian 32: first logical block
const DR_LENGTH: usize = 10; // both-endian 32: data length in bytes
const DR_FLAGS: usize = 25;
const DR_NAME_LEN: usize = 32;
const DR_NAME: usize = 33;
/// §9.1.6 bit 1 — this record names a directory.
const FLAG_DIRECTORY: u8 = 0x02;
/// §9.1.6 bit 2 — an associated file: a Macintosh resource fork here.
const FLAG_ASSOCIATED: u8 = 0x04;

/// Apple's System Use entry: `AA`, length, system-use ID, then the data.
const AA_SIGNATURE: &[u8; 2] = b"AA";
/// System-use ID 2 is the HFS form: type(4), creator(4), Finder flags(2).
const AA_HFS: u8 = 2;
const AA_TYPE: usize = 4;
const AA_CREATOR: usize = 8;
/// The shortest `AA` entry this reader can take anything from.
const AA_HFS_LEN: usize = 14;

/// How deep the directory walk will recurse. The two discs are three levels at
/// most; the bound exists so a self-referential extent cannot spin.
const MAX_DEPTH: usize = 8;

/// The most file records one mount will collect. Real discs hold hundreds; the
/// cap is a ceiling for a hostile image whose directories fan out, after which
/// the walk stops and keeps what it has.
const MAX_ENTRIES: usize = 100_000;

/// Why an ISO 9660 image would not open.
#[derive(Debug, PartialEq, Eq)]
pub enum IsoError {
    /// No Primary Volume Descriptor where §6.2.1 puts one.
    NotIso9660,
}

impl std::fmt::Display for IsoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not an ISO 9660 volume")
    }
}

/// One file on the disc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsoEntry {
    /// The identifier as stored, with the `;1` version suffix stripped.
    pub name: String,
    /// The folder chain from the root, outermost first; empty at the root.
    /// **This is what names a game on a compilation** — `SHOGUN FOLDER` and
    /// `ARTHUR FOLDER` are the only things telling two `STORY.DATA` apart.
    pub dirs: Vec<String>,
    /// Size in bytes, from the directory record.
    pub size: usize,
    /// The Finder type from an Apple `AA` entry, or spaces when it carries none.
    pub file_type: [u8; 4],
    /// The Finder creator from the same entry.
    pub creator: [u8; 4],
    /// First logical block of the file's extent.
    extent: usize,
}

impl IsoEntry {
    /// How this file is named to the outside world: `Folder/Sub/NAME` inside a
    /// folder, the bare name at the root. Slash-separated, the spelling
    /// [`crate::fat12`] and [`crate::hfs`] already use.
    pub fn path(&self) -> String {
        if self.dirs.is_empty() {
            return self.name.clone();
        }
        format!("{}/{}", self.dirs.join("/"), self.name)
    }

    /// The machine this file's Finder metadata names — one rule, shared with
    /// [`crate::hfs`]; see [`crate::medium::machine_from_finder`].
    pub fn machine(&self) -> Option<crate::medium::DiskImage> {
        crate::medium::machine_from_finder(&self.file_type, &self.creator)
    }

    /// Whether this file is a DOS build sitting on a disc a Macintosh reads.
    pub fn is_from_dos(&self) -> bool {
        self.machine() == Some(crate::medium::DiskImage::Fat12Dos)
    }
}

/// A mounted ISO 9660 disc.
#[derive(Debug)]
pub struct Iso9660 {
    image: Source,
    block: usize,
    name: String,
    files: Vec<IsoEntry>,
    /// [`crate::medium::Volume::has_packed_index`]'s answer, once asked.
    packed_index: std::sync::OnceLock<bool>,
}

impl Iso9660 {
    /// Cheap sniff: is there a Primary Volume Descriptor where one has to be?
    ///
    /// By CONTENT, like every other reader here — the `.iso` a disc is usually
    /// called means nothing, and a `.bin` or an extensionless dump is claimed on
    /// the same terms. No other format in this crate puts `CD001` 32 KB in, so
    /// this cannot collide with one.
    pub fn looks_like_iso9660(raw: &[u8]) -> bool {
        primary_descriptor(raw).is_some()
    }

    /// Open the disc and enumerate every file on it.
    pub fn mount(image: Vec<u8>) -> Result<Iso9660, IsoError> {
        Iso9660::mount_source(Source::Mem(std::sync::Arc::new(image)))
    }

    /// [`Iso9660::mount`] over bytes that may stay on disk (SQ-1761).
    pub(crate) fn mount_source(image: Source) -> Result<Iso9660, IsoError> {
        let pvd_at = match &image {
            Source::Mem(bytes) => primary_descriptor(bytes),
            Source::Disc { .. } => {
                // The set of descriptors is a handful of sectors at the front;
                // scanning them is a head read, not a walk of the disc.
                primary_descriptor(&image.get_clamped(0, crate::source::HEAD))
            }
        }
        .ok_or(IsoError::NotIso9660)?;
        let pvd = image.get(pvd_at..pvd_at + SECTOR).ok_or(IsoError::NotIso9660)?.into_owned();
        let block = usize::from(le16(&pvd, PVD_BLOCK_SIZE));
        if block == 0 || !block.is_multiple_of(512) {
            return Err(IsoError::NotIso9660);
        }
        let name = ascii_field(&pvd[PVD_VOLUME_ID..PVD_VOLUME_ID + 32]);
        let root = &pvd[PVD_ROOT_RECORD..PVD_ROOT_RECORD + 34];
        let (extent, len) = (le32(root, DR_EXTENT) as usize, le32(root, DR_LENGTH) as usize);

        let mut iso = Iso9660 {
            image,
            block,
            name,
            files: Vec::new(),
            packed_index: std::sync::OnceLock::new(),
        };
        let mut files = Vec::new();
        let mut seen = std::collections::HashSet::new();
        iso.walk(extent, len, &[], &mut files, 0, &mut seen);
        iso.files = files;
        Ok(iso)
    }

    /// The volume's own name, as the PVD spells it.
    pub fn volume_name(&self) -> &str {
        &self.name
    }

    /// Every file on the disc, in directory order. Folders are not listed, and
    /// neither are the associated records carrying Macintosh resource forks.
    pub fn files(&self) -> &[IsoEntry] {
        &self.files
    }

    /// Read a file's contents, or `None` when its extent runs off the image.
    pub fn read(&self, entry: &IsoEntry) -> Option<Vec<u8>> {
        let at = entry.extent.checked_mul(self.block)?;
        self.image.get(at..at.checked_add(entry.size)?).map(std::borrow::Cow::into_owned)
    }

    /// The first `n` bytes of a file — fewer if it is shorter, `None` when its
    /// extent runs off the image (SQ-1761).
    pub(crate) fn read_prefix(&self, entry: &IsoEntry, n: usize) -> Option<Vec<u8>> {
        let at = entry.extent.checked_mul(self.block)?;
        // The whole extent has to be there for `read` to answer, so the prefix
        // of one that is not is not an answer either.
        at.checked_add(entry.size).filter(|end| *end <= self.image.len())?;
        self.image.get(at..at.checked_add(n.min(entry.size))?).map(std::borrow::Cow::into_owned)
    }

    pub(crate) fn packed_index(&self) -> &std::sync::OnceLock<bool> {
        &self.packed_index
    }

    /// Whether the disc is read from a file rather than held (SQ-1761).
    pub(crate) fn is_file_backed(&self) -> bool {
        matches!(self.image, Source::Disc { .. })
    }

    /// Read a file by path or by bare name, case-insensitively — the path is
    /// what reaches a PARTICULAR one when a disc holds several games.
    pub fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        let e = self
            .files
            .iter()
            .find(|e| e.path().eq_ignore_ascii_case(name) || e.name.eq_ignore_ascii_case(name))?;
        self.read(e)
    }

    /// The story to open when the disc is asked for one, with its path.
    ///
    /// A compilation wants [`Iso9660::files`] and a chooser; this exists so the
    /// format answers the same question every other one does. Largest wins,
    /// deterministically — these discs have no naming convention in common
    /// (`ZORK I` on one side, `ZORK1.DAT` on the other).
    ///
    /// Only the winner is read whole (SQ-1761): the tiebreak is the catalogued
    /// size and the path, and "is it a story" is a question about 64 bytes.
    pub fn story(&self) -> Option<(String, Vec<u8>)> {
        let mut cands: Vec<&IsoEntry> = self
            .files
            .iter()
            .filter(|e| {
                self.read_prefix(e, crate::adf::STORY_HEAD)
                    .is_some_and(|head| crate::adf::looks_like_story_head(&head, e.size))
            })
            .collect();
        cands.sort_by_key(|e| (std::cmp::Reverse(e.size), e.path()));
        cands.into_iter().find_map(|e| {
            self.read(e).filter(|b| looks_like_story(b)).map(|b| (e.path(), b))
        })
    }

    /// The picture archive the disc offers when asked as a whole.
    pub fn pictures(&self) -> Option<(String, InfocomPics)> {
        Self::best_archive(self.archives(self.files.iter()))
    }

    /// The archive stored **beside** the story at `path` — same folder, same
    /// machine — or `None` when that story has no artwork of its own.
    ///
    /// [`crate::hfs::Hfs::pictures_beside`]'s rule, applied to the other format
    /// that has folders, and for the same reason: this disc holds three
    /// graphical games per side, and "what artwork is on this disc" is not the
    /// same question as "what artwork is this game's".
    pub fn pictures_beside(&self, path: &str) -> Option<(String, InfocomPics)> {
        let story = self.files.iter().find(|e| e.path().eq_ignore_ascii_case(path))?;
        let (dirs, dos) = (story.dirs.clone(), story.is_from_dos());
        Self::best_archive(
            self.archives(self.files.iter().filter(|e| e.dirs == dirs && e.is_from_dos() == dos)),
        )
    }

    /// Whether this disc holds the file at `path` at all.
    pub fn holds(&self, path: &str) -> bool {
        self.files.iter().any(|e| e.path().eq_ignore_ascii_case(path))
    }

    /// The machine the file at `path` was pressed for, or `None` when the disc
    /// does not hold it or says nothing about it.
    pub fn machine_of(&self, path: &str) -> Option<crate::medium::DiskImage> {
        self.files.iter().find(|e| e.path().eq_ignore_ascii_case(path))?.machine()
    }

    /// Every readable picture archive among `entries`, identified by parsing.
    fn archives<'a>(
        &self,
        entries: impl Iterator<Item = &'a IsoEntry>,
    ) -> Vec<(String, InfocomPics)> {
        entries
            .filter_map(|e| self.read(e).map(|b| (e.path(), b)))
            .filter(|(_, b)| !looks_like_story(b))
            .filter_map(|(p, b)| InfocomPics::parse(b).ok().map(|pics| (p, pics)))
            .filter(|(_, p)| p.entries().iter().any(|e| e.has_pixels()))
            .collect()
    }

    /// Colour over monochrome, then picture count, then the path — the same
    /// preference [`crate::hfs`] states and for the reason stated there.
    fn best_archive(mut cands: Vec<(String, InfocomPics)>) -> Option<(String, InfocomPics)> {
        cands.sort_by_key(|(path, pics)| {
            (
                crate::medium::art_preference(pics),
                std::cmp::Reverse(pics.entries().len()),
                path.clone(),
            )
        });
        cands.into_iter().next()
    }

    /// Read one directory's extent as a run of records, recursing into the
    /// directories it names.
    ///
    /// Each directory extent is walked once (`seen`), so a record pointing back
    /// at its own or an ancestor's extent cannot multiply the work at every
    /// level, and the walk stops at [`MAX_ENTRIES`] records.
    fn walk(
        &self,
        extent: usize,
        len: usize,
        dirs: &[String],
        out: &mut Vec<IsoEntry>,
        depth: usize,
        seen: &mut std::collections::HashSet<usize>,
    ) {
        if depth > MAX_DEPTH || out.len() >= MAX_ENTRIES || !seen.insert(extent) {
            return;
        }
        let Some(at) = extent.checked_mul(self.block) else { return };
        let Some(data) = self.image.get(at..at.saturating_add(len)) else { return };
        let data = &data[..];
        let mut o = 0usize;
        while o < data.len() {
            let rec_len = usize::from(data[o]);
            if rec_len == 0 {
                // §6.8.1.1: a record never straddles a logical block, so a zero
                // length is padding to the next one.
                o = (o / self.block + 1) * self.block;
                continue;
            }
            let Some(rec) = data.get(o..o + rec_len) else { return };
            o += rec_len;
            let Some(entry) = self.record(rec) else { continue };
            match entry {
                Record::Directory { name, extent, len } => {
                    let mut deeper = dirs.to_vec();
                    deeper.push(name);
                    self.walk(extent, len, &deeper, out, depth + 1, seen);
                }
                Record::File(mut file) => {
                    if out.len() >= MAX_ENTRIES {
                        return;
                    }
                    file.dirs = dirs.to_vec();
                    out.push(file);
                }
            }
        }
    }

    /// One directory record, or `None` for the `\x00`/`\x01` self and parent
    /// entries, an associated file (a resource fork), or a record too short to
    /// read.
    fn record(&self, rec: &[u8]) -> Option<Record> {
        let name_len = usize::from(*rec.get(DR_NAME_LEN)?);
        let raw = rec.get(DR_NAME..DR_NAME + name_len)?;
        if matches!(raw, [0] | [1]) {
            return None;
        }
        let flags = *rec.get(DR_FLAGS)?;
        if flags & FLAG_ASSOCIATED != 0 {
            return None;
        }
        let extent = le32(rec, DR_EXTENT) as usize;
        let len = le32(rec, DR_LENGTH) as usize;
        // §7.5.1: the identifier is followed by a padding byte when its length
        // is even, so the System Use area starts on an even offset.
        let name = String::from_utf8_lossy(raw).split(';').next().unwrap_or_default().to_string();
        if flags & FLAG_DIRECTORY != 0 {
            return Some(Record::Directory { name, extent, len });
        }
        let system_use = rec.get(DR_NAME + name_len + usize::from(name_len.is_multiple_of(2))..);
        let (file_type, creator) = system_use.and_then(apple_type_creator).unwrap_or_default();
        Some(Record::File(IsoEntry {
            name,
            dirs: Vec::new(),
            size: len,
            file_type,
            creator,
            extent,
        }))
    }
}

/// What one directory record turned out to be.
enum Record {
    Directory { name: String, extent: usize, len: usize },
    File(IsoEntry),
}

/// The Finder type and creator from an Apple `AA` System Use entry, or `None`
/// when the area holds no HFS-form entry.
///
/// The area is a run of entries, each `signature(2) length(1) version(1) data…`,
/// so an unknown one is stepped over by its own length rather than ending the
/// scan.
fn apple_type_creator(mut area: &[u8]) -> Option<([u8; 4], [u8; 4])> {
    while area.len() >= 4 {
        let len = usize::from(area[2]);
        if len < 4 || len > area.len() {
            return None;
        }
        if &area[..2] == AA_SIGNATURE && area[3] == AA_HFS && len >= AA_HFS_LEN {
            let t: [u8; 4] = area[AA_TYPE..AA_TYPE + 4].try_into().ok()?;
            let c: [u8; 4] = area[AA_CREATOR..AA_CREATOR + 4].try_into().ok()?;
            return Some((t, c));
        }
        area = &area[len..];
    }
    None
}

/// A cooked ISO 9660 disc image on disk, as a window onto it, or `None`
/// (SQ-1761).
///
/// Only a **cooked** image is claimed, which is all the in-memory sniff claims
/// too: its descriptor scan starts at file byte 32,768, and a raw dump has
/// frame headers there. And a disc that an earlier format in the table would
/// claim anyway — a Macintosh volume at byte 0 — is left to the in-memory path
/// that claims it today, so the two paths cannot pick different readers.
pub(crate) fn disc_volume(disc: &std::sync::Arc<crate::source::Disc>) -> Option<Source> {
    if disc.sectors() != crate::cd::Sectors::Cooked {
        return None;
    }
    let head = disc.copy(0, crate::source::HEAD);
    primary_descriptor(&head)?;
    if crate::hfs::disc_is_a_bare_volume(disc) {
        return None;
    }
    Some(Source::window(disc, 0, disc.logical_len()))
}

/// Where the Primary Volume Descriptor sits, or `None` when the image has none.
///
/// The set is walked rather than assumed to start with the primary: a disc may
/// put a boot record first, and the terminator ends the scan.
fn primary_descriptor(raw: &[u8]) -> Option<usize> {
    let mut at = DESCRIPTOR_START;
    // Bounded by the image; a malformed set cannot run past the end.
    while let Some(vd) = raw.get(at..at + SECTOR) {
        if &vd[1..6] != MAGIC {
            return None;
        }
        match vd[0] {
            VD_PRIMARY => return Some(at),
            VD_TERMINATOR => return None,
            _ => at += SECTOR,
        }
    }
    None
}

/// A `d-characters` field, trailing spaces trimmed.
fn ascii_field(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).trim_end().to_string()
}

/// The little-endian half of a both-byte-orders 32-bit field (§7.3.3).
fn le32(b: &[u8], at: usize) -> u32 {
    let mut v = [0u8; 4];
    v.copy_from_slice(&b[at..at + 4]);
    u32::from_le_bytes(v)
}

/// The little-endian half of a both-byte-orders 16-bit field (§7.2.3).
fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// One synthetic ISO 9660 disc carrying `files` at the root, for the
    /// mount-seam tests in [`crate::medium`]. They need a real volume of every
    /// format and cannot reach a builder private to this module.
    ///
    /// Every file is given Infocom's `INdf`/`IN0Z` Finder pair through an Apple
    /// `AA` entry, so the extension path is exercised rather than only the bare
    /// ISO 9660 one.
    pub(crate) fn sample_disc(files: &[(&str, &[u8])]) -> Vec<u8> {
        const ROOT: usize = 18;
        let mut records: Vec<Vec<u8>> = vec![
            dir_record(&[0], ROOT as u32, SECTOR as u32, FLAG_DIRECTORY, None),
            dir_record(&[1], ROOT as u32, SECTOR as u32, FLAG_DIRECTORY, None),
        ];
        let mut at = ROOT + 1;
        let mut data: Vec<(usize, &[u8])> = Vec::new();
        for (name, bytes) in files {
            let id = format!("{name};1");
            records.push(dir_record(
                id.as_bytes(),
                at as u32,
                bytes.len() as u32,
                0,
                Some((b"INdf", b"IN0Z")),
            ));
            data.push((at, bytes));
            at += bytes.len().div_ceil(SECTOR).max(1);
        }

        let mut image = vec![0u8; at * SECTOR];
        // The Primary Volume Descriptor, then the terminator.
        let pvd = 16 * SECTOR;
        image[pvd] = VD_PRIMARY;
        image[pvd + 1..pvd + 6].copy_from_slice(MAGIC);
        image[pvd + 6] = 1;
        image[pvd + PVD_VOLUME_ID..pvd + PVD_VOLUME_ID + 32].fill(b' ');
        image[pvd + PVD_VOLUME_ID..pvd + PVD_VOLUME_ID + 9].copy_from_slice(b"TEST DISC");
        image[pvd + PVD_BLOCK_SIZE..pvd + PVD_BLOCK_SIZE + 2]
            .copy_from_slice(&(SECTOR as u16).to_le_bytes());
        image[pvd + PVD_BLOCK_SIZE + 2..pvd + PVD_BLOCK_SIZE + 4]
            .copy_from_slice(&(SECTOR as u16).to_be_bytes());
        let root = dir_record(&[0], ROOT as u32, SECTOR as u32, FLAG_DIRECTORY, None);
        image[pvd + PVD_ROOT_RECORD..pvd + PVD_ROOT_RECORD + 34].copy_from_slice(&root[..34]);
        image[17 * SECTOR] = VD_TERMINATOR;
        image[17 * SECTOR + 1..17 * SECTOR + 6].copy_from_slice(MAGIC);

        let mut o = ROOT * SECTOR;
        for r in &records {
            image[o..o + r.len()].copy_from_slice(r);
            o += r.len();
        }
        for (block, bytes) in data {
            let at = block * SECTOR;
            image[at..at + bytes.len()].copy_from_slice(bytes);
        }
        image
    }

    /// One directory record, padded exactly as ECMA-119 §9.1.12 requires: a
    /// padding byte after an identifier of even length, so the System Use area
    /// starts on an even offset, and an even record length overall.
    fn dir_record(
        id: &[u8],
        extent: u32,
        len: u32,
        flags: u8,
        aa: Option<(&[u8; 4], &[u8; 4])>,
    ) -> Vec<u8> {
        let mut r = vec![0u8; DR_NAME];
        r[DR_EXTENT..DR_EXTENT + 4].copy_from_slice(&extent.to_le_bytes());
        r[DR_EXTENT + 4..DR_EXTENT + 8].copy_from_slice(&extent.to_be_bytes());
        r[DR_LENGTH..DR_LENGTH + 4].copy_from_slice(&len.to_le_bytes());
        r[DR_LENGTH + 4..DR_LENGTH + 8].copy_from_slice(&len.to_be_bytes());
        r[DR_FLAGS] = flags;
        r[DR_NAME_LEN] = id.len() as u8;
        r.extend_from_slice(id);
        if id.len().is_multiple_of(2) {
            r.push(0);
        }
        if let Some((file_type, creator)) = aa {
            let mut e = vec![0u8; AA_HFS_LEN];
            e[..2].copy_from_slice(AA_SIGNATURE);
            e[2] = AA_HFS_LEN as u8;
            e[3] = AA_HFS;
            e[AA_TYPE..AA_TYPE + 4].copy_from_slice(file_type);
            e[AA_CREATOR..AA_CREATOR + 4].copy_from_slice(creator);
            r.extend_from_slice(&e);
        }
        if !r.len().is_multiple_of(2) {
            r.push(0);
        }
        r[0] = r.len() as u8;
        r
    }

    /// The builder produces a disc this reader agrees with — otherwise every
    /// case below would be testing the builder's idea of ISO 9660.
    #[test]
    fn the_synthetic_disc_round_trips() {
        let disc = sample_disc(&[("READ.ME", b"a text file"), ("STORY.DAT", b"pretend story")]);
        assert!(Iso9660::looks_like_iso9660(&disc));
        let iso = Iso9660::mount(disc).expect("mounts");
        assert_eq!(iso.volume_name(), "TEST DISC");
        let names: Vec<String> = iso.files().iter().map(|e| e.path()).collect();
        assert_eq!(names, ["READ.ME", "STORY.DAT"], "the `;1` suffix is stripped");
        assert_eq!(iso.read_named("STORY.DAT").as_deref(), Some(&b"pretend story"[..]));
        assert_eq!(
            iso.files()[0].machine(),
            Some(crate::medium::DiskImage::Hfs),
            "the Apple AA entry is read"
        );
    }

    /// SQ-1727: directory records that point back at the root's own extent
    /// were re-walked at every level (k records cost ~k^9 entries).
    #[test]
    fn self_referential_directories_are_walked_once() {
        let mut disc = sample_disc(&[("A.DAT", b"x")]);
        let root = 18 * SECTOR;
        let mut o = root;
        while disc[o] != 0 {
            o += usize::from(disc[o]);
        }
        for i in 0..4u8 {
            let id = [b'D', b'0' + i];
            let r = dir_record(&id, 18, SECTOR as u32, FLAG_DIRECTORY, None);
            disc[o..o + r.len()].copy_from_slice(&r);
            o += r.len();
        }
        let iso = Iso9660::mount(disc).expect("mounts");
        assert_eq!(iso.files().len(), 1, "the root is walked once, not once per level");
    }

    /// The two discs live outside the repo (`treasures/` is gitignored), so
    /// every case here skips vacuously. CI has none of them.
    fn disc(name: &str) -> Option<Iso9660> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../treasures/ISOs")
            .join(name);
        let Ok(raw) = std::fs::read(&path) else {
            eprintln!("SKIP: {name} is absent at {}", path.display());
            return None;
        };
        assert!(Iso9660::looks_like_iso9660(&raw), "{name} is an ISO 9660 disc");
        Some(Iso9660::mount(raw).unwrap_or_else(|e| panic!("{name}: {e}")))
    }

    /// **The resource forks are skipped, and the folder is reported.**
    ///
    /// Every Macintosh game on disc 1 is catalogued twice — the story and the
    /// interpreter's resource fork under one name — so taking both would list
    /// eighteen games as thirty-six and offer a resource fork as a story. The
    /// Associated File flag (§9.1.6) is the standard's own answer.
    ///
    /// FALSIFICATION: stop skipping `FLAG_ASSOCIATED` and the file count rises
    /// by exactly the number of Macintosh files, with `MAC/ZORK I` appearing
    /// twice.
    #[test]
    fn resource_forks_are_skipped_and_the_folder_is_reported() {
        let Some(iso) = disc("LostTreasures1.iso") else { return };
        assert_eq!(iso.volume_name(), "INFOCOM");

        let paths: Vec<String> = iso.files().iter().map(|e| e.path()).collect();
        assert_eq!(paths.iter().filter(|p| *p == "MAC/ZORK I").count(), 1, "once, not twice");
        assert!(paths.contains(&"MAC/ZORK ZERO/STORY.DATA".to_string()));
        assert!(paths.contains(&"PC/DATA/ZORK1.DAT".to_string()));

        // …and the one that only exists here: Hitchhiker's, which the
        // Masterpieces disc does not carry at all.
        let hitch = iso
            .files()
            .iter()
            .find(|e| e.path() == "MAC/HITCHHIKER'S GUIDE")
            .expect("the Macintosh Hitchhiker's");
        let bytes = iso.read(hitch).expect("it reads");
        assert!(looks_like_story(&bytes), "the data fork is the story, not the resources");
        assert_eq!(bytes.len(), 113_664);
    }

    /// **The Apple extension separates the two halves**, exactly as the Finder
    /// metadata does on the hybrid HFS disc.
    ///
    /// Two things this pins that are easy to state too strongly:
    ///
    /// The rule NEVER contradicts the disc. Every file it names a machine for
    /// is in that machine's tree — so a wrong answer is impossible, whatever
    /// the coverage.
    ///
    /// The coverage is not total, and must not be asserted as though it were.
    /// Every Macintosh story on both discs is recognised, because Infocom
    /// stamped its own creator on all of them. The DOS side is recognised on
    /// disc 2 (`PCXT`) and only partly on disc 1, where nineteen files under
    /// `PC/DATA/` carry a BLANK creator and are therefore unclaimed. That is
    /// the fail-safe doing its job rather than a gap to paper over: an
    /// unrecognised file falls back to what the volume implies, and this row
    /// implies no machine at all, which leaves the IBM PC default in force —
    /// the right answer for exactly those files.
    #[test]
    fn the_apple_extension_says_which_machine_each_file_is_for() {
        for (name, dos_tree) in [("LostTreasures1.iso", "PC"), ("LostTreasures2.iso", "DOS")] {
            let Some(iso) = disc(name) else { continue };
            let (mut mac, mut dos, mut unclaimed) = (0, 0, 0);
            for e in iso.files() {
                let Some(bytes) = iso.read(e) else { continue };
                if !looks_like_story(&bytes) {
                    continue;
                }
                let top = e.dirs.first().map(String::as_str).unwrap_or_default();
                match e.machine() {
                    Some(crate::medium::DiskImage::Hfs) => {
                        mac += 1;
                        assert_eq!(top, "MAC", "{name}: {}", e.path());
                    }
                    Some(crate::medium::DiskImage::Fat12Dos) => {
                        dos += 1;
                        assert_eq!(top, dos_tree, "{name}: {}", e.path());
                    }
                    // Unclaimed, and therefore not in the DOS tree by accident:
                    // it must at least not be a Macintosh file, or the fallback
                    // would hand a Macintosh story the IBM PC.
                    None => {
                        unclaimed += 1;
                        assert_ne!(top, "MAC", "{name}: unclaimed Macintosh {}", e.path());
                    }
                    other => panic!("{name}: {} says {other:?}", e.path()),
                }
            }
            assert!(mac > 0, "{name}: the Macintosh half is recognised in full ({mac})");
            assert!(dos + unclaimed > 0, "{name}: the disc has a DOS half at all");
        }
    }

    /// **Each graphical game pairs with the archive in its own folder** — and
    /// disc 2 is the only medium in the corpus carrying *Shogun* for two
    /// machines at once.
    #[test]
    fn each_graphical_game_on_disc_two_pairs_with_its_own_artwork() {
        let Some(iso) = disc("LostTreasures2.iso") else { return };
        for (story, art) in [
            ("MAC/SHOGUN FOLDER/STORY.DATA", Some("MAC/SHOGUN FOLDER/CPIC.DATA")),
            ("MAC/ARTHUR FOLDER/STORY.DATA", Some("MAC/ARTHUR FOLDER/CPIC.DATA")),
            ("MAC/JOURNEY FOLDER/STORY.DATA", Some("MAC/JOURNEY FOLDER/CPIC.DATA")),
            // MCGA, and this disc is why the rule exists (SQ-0880). All three
            // of its DOS games press `.MG1` and `.EG1` into ONE folder, which
            // is the example `medium.rs` said the MCGA-against-EGA question
            // lacked. Ranking on picture count split them two ways — Shogun's
            // EGA holds 50 against MCGA's 48, Arthur's 171 against 171 — so
            // `art_preference` decides it outright and all three agree.
            ("DOS/SHOGUN/SHOGUN.ZIP", Some("DOS/SHOGUN/SHOGUN.MG1")),
            ("DOS/ARTHUR/ARTHUR.ZIP", Some("DOS/ARTHUR/ARTHUR.MG1")),
            ("DOS/JOURNEY/JOURNEY.ZIP", Some("DOS/JOURNEY/JOURNEY.MG1")),
            // A text game shipped with no artwork gets none, not another's.
            ("MAC/TRINITY", None),
        ] {
            assert!(iso.holds(story), "{story} is on the disc");
            assert_eq!(
                iso.pictures_beside(story).map(|(n, _)| n).as_deref(),
                art,
                "the artwork paired with {story}"
            );
        }
    }

    /// A `read_named` by path reaches a particular file where the bare name is
    /// ambiguous — three `STORY.DATA` on disc 2.
    #[test]
    fn a_path_reaches_one_of_three_story_data_files() {
        let Some(iso) = disc("LostTreasures2.iso") else { return };
        let named = |p: &str| iso.read_named(p).map(|b| b.len());
        assert_eq!(named("MAC/SHOGUN FOLDER/STORY.DATA"), Some(341_416));
        assert_eq!(named("MAC/ARTHUR FOLDER/STORY.DATA"), Some(270_848));
        assert_eq!(named("MAC/JOURNEY FOLDER/STORY.DATA"), Some(279_872));
        assert!(named("STORY.DATA").is_some(), "a bare name still resolves to one of them");
    }
}
