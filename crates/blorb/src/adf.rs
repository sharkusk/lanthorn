//! Read an Amiga `.adf` floppy image — enough AmigaDOS filesystem to pull a
//! story file and its native Infocom picture archive straight out of the
//! original release media, with no extraction step.
//!
//! Infocom's Amiga releases ship as 880 KB DD floppies: 1760 blocks of 512
//! bytes, an AmigaDOS bootblock, and either the Old (OFS) or Fast (FFS) File
//! System. Everything below is verified against the two original *Zork Zero*
//! disks; the layout constants come from the ADF format description (Laurent
//! Clévy's "Amiga Disk File format FAQ", the same one `adflib` implements).
//!
//! # Layout
//!
//! * **Bootblock** (block 0) — `DOS` then a flag byte whose bit 0 selects FFS
//!   over OFS.
//! * **File header block** — longword 0 is `2` (`T_HEADER`); longword 1 is the
//!   block's own number; the last longword is `-3` (`ST_FILE`). The byte size
//!   sits at `BSIZE-188`, the name (length-prefixed, ≤30 bytes) at `BSIZE-80`,
//!   and a chain pointer to a *file extension block* at `BSIZE-8`.
//! * **Data-block table** — `high_seq` pointers stored in REVERSE from
//!   `BSIZE-204` downwards, in the file header and again in each extension
//!   block (`T_LIST` = 16, secondary type `ST_FILE`).
//! * **Data block** — under OFS a 24-byte header (`T_DATA` = 8, owning header,
//!   sequence number, bytes used, next block, checksum) precedes ≤488 payload
//!   bytes; under FFS the whole 512 bytes are payload.
//!
//! Files are enumerated by **scanning every block for a file header** rather
//! than walking the root block's hash table. It is simpler, it finds files in
//! subdirectories without recursing, and the `header_key == own block number`
//! self-reference makes a false positive vanishingly unlikely.
//!
//! # Choosing what to run
//!
//! AmigaOS has no filename extensions, and while Infocom's convention is
//! `Story.data` beside `Pic.data`, nothing on the disk guarantees it — the
//! disk's own `.info` manifest even names a file that was never written. So
//! candidates are identified by **content** ([`looks_like_story`], and
//! [`InfocomPics::parse`] for the artwork); the conventional names are only a
//! tiebreak when a disk offers more than one of either.

use crate::infocom_pics::InfocomPics;

/// AmigaDOS block size for a DD floppy. Every offset below is relative to it.
pub const BSIZE: usize = 512;

/// `T_HEADER` — the primary type of a file/directory header block.
const T_HEADER: u32 = 2;
/// `T_LIST` — the primary type of a file extension block.
const T_LIST: u32 = 16;
/// `T_DATA` — the primary type of an OFS data block.
const T_DATA: u32 = 8;
/// `ST_FILE` — the secondary type marking a header as a plain file.
const ST_FILE: u32 = 0xFFFF_FFFD;
/// Secondary type of a **directory** header block. The block scan walks these to
/// give each file its parent, because a bare filename does not identify a story on
/// a disk that stores every game under Infocom's one conventional name (SQ-0908).
const ST_USERDIR: u32 = 2;
/// The parent-directory block pointer, in a file or directory header.
const PARENT: usize = BSIZE - 12;

/// Offset of the FIRST data-block pointer; the table runs DOWNWARDS from here.
const DATA_TABLE_TOP: usize = BSIZE - 204;
/// Number of data-block pointers a header or extension block can hold.
const DATA_TABLE_LEN: usize = 72;
/// OFS data blocks reserve this many bytes for their header.
const OFS_DATA_HEADER: usize = 24;
/// Longest AmigaDOS filename.
const MAX_NAME: usize = 30;

/// Infocom's conventional names on an Amiga release disk. Never a test — only
/// a tiebreak when content identification finds more than one candidate.
const CONVENTIONAL_STORY: &str = "story.data";
const CONVENTIONAL_PICTURES: &str = "pic.data";

/// Errors that can arise while mounting a disk image.
#[derive(Debug, PartialEq, Eq)]
pub enum AdfError {
    /// The bytes are not an AmigaDOS disk image (no `DOS` bootblock, or a
    /// length that is not a whole number of 512-byte blocks).
    NotAdf,
}

/// One file found on the image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdfEntry {
    /// The AmigaDOS filename, exactly as stored (no extension convention).
    pub name: String,
    /// The file's full path from the volume root, `Dir/Sub/Name`, or just the name
    /// when it sits at the root (SQ-0908).
    ///
    /// A bare filename does not identify a story on an Amiga release disk: *The Lost
    /// Treasures of Infocom* disk 1 carries THREE files called `Story.Data`, one each
    /// under `Spellbreaker`, `Sorcerer` and `Enchanter`, and a caller asking for
    /// "Story.Data" got whichever the block scan reached first. The path is what makes
    /// a row on that disk name its own game.
    pub path: String,
    /// Size in bytes, from the header block.
    pub size: usize,
    /// Block number of this file's header block.
    pub header: usize,
}

/// A mounted AmigaDOS disk image.
#[derive(Debug)]
pub struct Adf {
    image: Vec<u8>,
    ffs: bool,
    files: Vec<AdfEntry>,
}

impl Adf {
    /// Cheap sniff: does this look like an AmigaDOS disk image? Checks the
    /// `DOS` bootblock magic, its filesystem flag byte, and a whole number of
    /// blocks. A Z-machine, Glulx, Blorb or Scott image can never collide (all
    /// four disagree at byte 0), so this is safe to run ahead of story loading.
    pub fn looks_like_adf(bytes: &[u8]) -> bool {
        bytes.len() >= 2 * BSIZE
            && bytes.len().is_multiple_of(BSIZE)
            && bytes.starts_with(b"DOS")
            // Only the low three bits (FFS / international / dir-cache) are
            // defined; anything else is not a filesystem we recognise.
            && bytes[3] < 8
    }

    /// Mount an image and enumerate its files.
    pub fn mount(image: Vec<u8>) -> Result<Adf, AdfError> {
        if !Adf::looks_like_adf(&image) {
            return Err(AdfError::NotAdf);
        }
        let ffs = image[3] & 1 != 0;
        let mut adf = Adf { image, ffs, files: Vec::new() };
        adf.files = adf.scan_file_headers();
        Ok(adf)
    }

    /// True when the image uses the Fast File System (raw 512-byte data
    /// blocks); false for the Old File System (24-byte data-block headers).
    pub fn is_ffs(&self) -> bool {
        self.ffs
    }

    /// Every file on the image, in block order. Directories are not listed —
    /// the block scan finds files wherever they live, so the tree is flat here.
    pub fn files(&self) -> &[AdfEntry] {
        &self.files
    }

    /// One block, or `None` if the number is off the end of the image.
    fn block(&self, n: usize) -> Option<&[u8]> {
        self.image.get(n * BSIZE..(n + 1) * BSIZE)
    }

    /// Walk every block looking for a valid file header.
    fn scan_file_headers(&self) -> Vec<AdfEntry> {
        let mut out = Vec::new();
        for n in 0..self.image.len() / BSIZE {
            let Some(b) = self.block(n) else { continue };
            if be32(b, 0) != T_HEADER || be32(b, BSIZE - 4) != ST_FILE {
                continue;
            }
            // A file header names its own block. Nothing else on a disk does,
            // which is what makes the scan trustworthy.
            if be32(b, 4) as usize != n {
                continue;
            }
            let Some(name) = read_name(b) else { continue };
            out.push(AdfEntry {
                path: self.path_of(&name, be32(b, PARENT) as usize),
                name,
                size: be32(b, BSIZE - 188) as usize,
                header: n,
            });
        }
        out
    }

    /// `name` prefixed by every directory above it, `Dir/Sub/Name` (SQ-0908).
    ///
    /// The block scan finds files wherever they live and never learns the tree, so
    /// the parent chain is walked here instead: a header block's `BSIZE-12` longword
    /// is its containing directory's block, and the walk stops at the root
    /// (whose secondary type is 1, not `ST_USERDIR`) or at anything that is not a directory header. Bounded by the
    /// image's block count, because a corrupt disk could chain a cycle.
    fn path_of(&self, name: &str, mut parent: usize) -> String {
        let mut parts: Vec<String> = Vec::new();
        for _ in 0..=self.image.len() / BSIZE {
            let Some(b) = self.block(parent) else { break };
            if be32(b, 0) != T_HEADER || be32(b, BSIZE - 4) != ST_USERDIR {
                break; // the root block, or not a directory at all
            }
            // Same self-reference the file scan trusts: a header names its own block.
            if be32(b, 4) as usize != parent {
                break;
            }
            let Some(dir) = read_name(b) else { break };
            parts.push(dir);
            parent = be32(b, PARENT) as usize;
        }
        parts.reverse();
        parts.push(name.to_string());
        parts.join("/")
    }

    /// Read a file's bytes. `None` if its block chain is broken or runs short
    /// of the size its header declares.
    pub fn read(&self, entry: &AdfEntry) -> Option<Vec<u8>> {
        let mut out: Vec<u8> = Vec::with_capacity(entry.size);
        let mut cur = entry.header;
        // A corrupt image could chain extension blocks in a cycle; the image
        // has finitely many blocks, so bound the walk by that.
        for _ in 0..=self.image.len() / BSIZE {
            let b = self.block(cur)?;
            let high_seq = be32(b, 8) as usize;
            if high_seq > DATA_TABLE_LEN {
                return None;
            }
            for i in 0..high_seq {
                let data = self.block(be32(b, DATA_TABLE_TOP - 4 * i) as usize)?;
                if self.ffs {
                    out.extend_from_slice(data);
                } else {
                    if be32(data, 0) != T_DATA {
                        return None;
                    }
                    let used = be32(data, 12) as usize;
                    out.extend_from_slice(data.get(OFS_DATA_HEADER..OFS_DATA_HEADER + used)?);
                }
            }
            let ext = be32(b, BSIZE - 8) as usize;
            if ext == 0 {
                break;
            }
            let eb = self.block(ext)?;
            if be32(eb, 0) != T_LIST || be32(eb, BSIZE - 4) != ST_FILE {
                return None;
            }
            cur = ext;
        }
        if out.len() < entry.size {
            return None;
        }
        // FFS pads the last block; the header's byte size is authoritative.
        out.truncate(entry.size);
        Some(out)
    }

    /// Read a file by name (case-insensitive), for callers that already know
    /// what they want. Prefer [`Adf::story`] / [`Adf::pictures`], which
    /// identify by content.
    ///
    /// The full PATH is tried first and the bare name second (SQ-0908). A path is
    /// unique on the volume where a name need not be — *The Lost Treasures of
    /// Infocom* disk 1 carries three files called `Story.Data`, under `Spellbreaker`,
    /// `Sorcerer` and `Enchanter` — so a caller that was shown paths by
    /// [`crate::medium::MountedDisk::contents`] can ask for exactly the one it means.
    /// The name fallback keeps every caller that hands over a bare filename working,
    /// including the `--pictures` door, and answers as it always did: the first match
    /// in block order.
    pub fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        let e = self
            .files
            .iter()
            .find(|e| e.path.eq_ignore_ascii_case(name))
            .or_else(|| self.files.iter().find(|e| e.name.eq_ignore_ascii_case(name)))?;
        self.read(e)
    }

    /// The story image on this disk, with the name it was stored under.
    ///
    /// Every file is tested with [`looks_like_story`]; a disk with none (an
    /// AmigaDOS boot disk, say) yields `None`. When more than one file passes —
    /// which no Infocom release does — Infocom's `Story.data` convention breaks
    /// the tie, then the largest candidate, so the choice is deterministic
    /// rather than directory-order luck.
    pub fn story(&self) -> Option<(String, Vec<u8>)> {
        let mut cands: Vec<(String, Vec<u8>)> = self
            .files
            .iter()
            .filter_map(|e| self.read(e).map(|b| (e.name.clone(), b)))
            .filter(|(_, b)| looks_like_story(b))
            .collect();
        cands.sort_by_key(|(name, bytes)| {
            (!name.eq_ignore_ascii_case(CONVENTIONAL_STORY), std::cmp::Reverse(bytes.len()))
        });
        cands.into_iter().next()
    }

    /// The native Infocom picture archive on this disk, with its stored name.
    ///
    /// Identified by parsing: [`InfocomPics::parse`] validates the record size,
    /// the directory and the Huffman-tree offset structurally, and we further
    /// require at least one entry to carry real pixels. Files already claimed
    /// by [`Adf::story`] are excluded so a story can never be mistaken for art.
    /// Ties break on the `Pic.data` convention, then on picture count.
    ///
    /// The archive's [`InfocomPics::part`] number survives on the returned
    /// value: a multi-part release numbers its archives 1, 2, … and a future
    /// multi-disk mount can join them without changing anything here.
    pub fn pictures(&self) -> Option<(String, InfocomPics)> {
        let mut cands: Vec<(String, InfocomPics)> = self
            .files
            .iter()
            .filter_map(|e| self.read(e).map(|b| (e.name.clone(), b)))
            .filter(|(_, b)| !looks_like_story(b))
            .filter_map(|(name, b)| InfocomPics::parse(b).ok().map(|p| (name, p)))
            .filter(|(_, p)| p.entries().iter().any(|e| e.has_pixels()))
            .collect();
        cands.sort_by_key(|(name, pics)| {
            (
                !name.eq_ignore_ascii_case(CONVENTIONAL_PICTURES),
                std::cmp::Reverse(pics.entries().len()),
            )
        });
        cands.into_iter().next()
    }
}

/// Does `bytes` look like a story image lanthorn could run?
///
/// A Blorb or a Glulx image says so in its first four bytes. A Z-machine story
/// has no magic at all, so its header is validated instead: a version lanthorn
/// runs, a static-memory base that a header actually fits under, the object and
/// global tables inside dynamic memory, high memory and the dictionary at or
/// above the static base, a printable serial (in ASCII or high ASCII), and a
/// declared file length that does not exceed the bytes present.
///
/// The version floor of 3 is deliberate. `zvm` runs v3–v8, Infocom's Amiga
/// releases span v3–v6, and admitting v1/v2 would let files that merely begin
/// with a small byte through — the original *Zork Zero* disk carries two saved
/// games starting `03 aa`, and only the header checks below reject them.
pub fn looks_like_story(bytes: &[u8]) -> bool {
    if bytes.starts_with(b"FORM") && bytes.len() > 12 && &bytes[8..12] == b"IFRS" {
        return true;
    }
    if bytes.starts_with(b"Glul") {
        return true;
    }
    looks_like_zcode(bytes)
}

/// The Z-machine half of [`looks_like_story`], and the workspace's **only**
/// positive identity check for a Z-code image.
///
/// Public since SQ-0889, which needed it above the disk readers: `app` used to
/// treat Z-code as the untested else-branch — Blorb proves itself by magic,
/// Glulx by `Glul`, Scott by content sniff, and anything left over was assumed
/// to be a story — so the only gate a container had to pass was `parse_header`'s
/// `3..=8` on byte 0, which about 2.3% of arbitrary bytes do. What it let
/// through ran, printed nothing, and exited 0.
///
/// It is exported rather than reimplemented deliberately. Every clause below is
/// a measurement against the corpus, two of them are corrections that cost a
/// real game its visibility (SQ-0856's high-ASCII serial, SQ-0869's Commodore
/// *Trinity*), and a second sniff elsewhere in the workspace would be a second
/// place for that knowledge to go stale.
pub fn looks_like_zcode(bytes: &[u8]) -> bool {
    if bytes.len() < 64 {
        return false;
    }
    let word = |o: usize| usize::from(u16::from_be_bytes([bytes[o], bytes[o + 1]]));
    let version = bytes[0];
    // Packed-address scale, which is also the file-length unit (ZMSD §11.1.6).
    let scale = match version {
        // Versions 1 and 2 share Version 3's scale (ZMSD §11.1.6: "This constant
        // is 2 for Versions 1 to 3"). They are here because this is the gate the
        // whole app loads a story through, not only a disk sniffer — refuse them
        // and `zvm` supporting them changes nothing the player can reach.
        //
        // Byte 0 alone is a weak signal and 1 and 2 are commoner values than 3–8:
        // of 320 files in one real `stories/` directory, 37 open with a 1 or a 2
        // — Apple II `.po`/`.dsk` images, and the `.eg1`/`.cg1`/`.mg1`/`.pic`
        // graphics files beside the Version 6 games. **None of the 37 survives
        // the clauses below**, every one of them failing on a control byte in the
        // serial field; the structural checks are what does the work here, and
        // widening the version arm cost no false positive at all. That was
        // measured before the change, not assumed after it (SQ-1422).
        1..=3 => 2,
        4 | 5 => 4,
        6..=8 => 8,
        _ => return false,
    };
    let (high, dict, objects, globals, static_base) =
        (word(0x04), word(0x08), word(0x0a), word(0x0c), word(0x0e));
    // Static memory starts after the header and inside the file.
    if !(64..=bytes.len()).contains(&static_base) {
        return false;
    }
    // Object and global tables are writable, so they live in dynamic memory.
    if !(64..static_base).contains(&objects) || !(64..static_base).contains(&globals) {
        return false;
    }
    // The dictionary is in static memory, and high memory is somewhere in the
    // file.
    //
    // **High memory is NOT required to begin at or after static memory**, and it
    // was until SQ-0869 found a release where it does not. Infocom's Commodore
    // *Trinity* — release 12, serial 860926, on `TRINITY1.D64`/`TRINITY2.D64` —
    // declares a static base of 37,726 and a high-memory mark of **22,527**,
    // where the identical build in `stories/trinity-r12-s860926.z4` declares
    // 63,423. The two files are byte-identical from `$40` to the end of all
    // 262,064 of them; only `$04` and two other bytes below the checksum's own
    // floor differ.
    //
    // That is a press for a 64 KB machine carrying a 256 KB story: almost all of
    // it has to be pageable, so the resident region is a third of what the
    // reference build declares, and `$04` is inside the region ZMSD §11.1.6
    // deliberately leaves out of the header checksum so that an interpreter may
    // write it. Demanding the usual ordering made a real, checksum-verified game
    // invisible — the same shape of defect as the high-ASCII serial two clauses
    // below (SQ-0856), and fixed the same way: widen the clause that was
    // assuming, keep every clause that was checking.
    if !(64..=bytes.len()).contains(&high) || dict < static_base || dict >= bytes.len() {
        return false;
    }
    // Serial is six printable characters ("890323", or "------" on some builds)
    // — **in either ASCII or the high ASCII the Apple II wrote text in**, so
    // bit 7 is masked before the test rather than being grounds for refusal.
    //
    // That last clause is not a loophole, it is a whole game. `LEATHRGODDESSES`
    // on *The Lost Treasures of Infocom* volume `INFOCOM6` is a structurally
    // valid Version 3 story — declared length `0xfbf3 * 2` == its 128998 bytes
    // exactly — whose serial reads `C2 EC EF F7 EE A1`. Mask bit 7 off each and
    // that is `42 6C 6F 77 6E 21`, "Blown!": a joke serial typed on a machine
    // whose character set sets the high bit, not corruption. Demanding bit 7
    // clear made Leather Goddesses of Phobos invisible on that volume (SQ-0856).
    //
    // The rejection this check exists for is unaffected, because it was never
    // bit 7 that did the work: a saved game's `$12..$18` is binary, and binary
    // is control bytes. `00`–`1F`, `7F`, `80`–`9F` and `FF` all still fail.
    if !bytes[0x12..0x18].iter().all(|c| (0x20..0x7f).contains(&(c & 0x7f))) {
        return false;
    }
    // The declared length may under-run the file (release padding) but never
    // over-run it. Zero means "not recorded", which early releases leave alone.
    let declared = word(0x1a) * scale;
    declared == 0 || declared <= bytes.len()
}

/// The length-prefixed filename at `BSIZE-80`, or `None` when it is empty,
/// over-long, or contains control bytes.
fn read_name(block: &[u8]) -> Option<String> {
    let len = usize::from(block[BSIZE - 80]);
    if len == 0 || len > MAX_NAME {
        return None;
    }
    let raw = block.get(BSIZE - 79..BSIZE - 79 + len)?;
    if raw.iter().any(|c| *c < 0x20 || *c == 0x7f) {
        return None;
    }
    // AmigaDOS filenames are Latin-1, which maps to char verbatim.
    Some(raw.iter().map(|c| char::from(*c)).collect())
}

/// Big-endian longword at `off`.
fn be32(b: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Number of blocks on a real 880 KB DD floppy.
    const DD_BLOCKS: usize = 1760;

    /// A file's PATH names its directory, and that is what tells three files called
    /// `Story.Data` apart (SQ-0908).
    ///
    /// *The Lost Treasures of Infocom* is the release this is about: every game on it
    /// lives in its own directory under Infocom's one conventional filename, so disk 3
    /// carries `Suspended/Story.Data`, `Starcross/Story.Data`,
    /// `Hitchhiker/Story.Data`, `Stationfall/Story.Data`, `Planetfall/Story.Data` and
    /// `Infidel/Story.Data`. The block scan finds files wherever they live and never
    /// walked the tree, so all six were reported as `Story.Data` and a caller asking
    /// for one got whichever the scan reached first — picking Hitchhiker's off the
    /// browser launched Suspended.
    ///
    /// Both halves are asserted: the paths are distinct AND `read_named` hands back
    /// the file each one names. Nesting is two deep, because a parent chain that
    /// stops one level up would still pass a flat pair of directories.
    #[test]
    fn a_files_path_names_the_directory_it_lives_in() {
        for ffs in [false, true] {
            let mut b = DiskBuilder::new(ffs);
            let suspended = b.add_dir("Suspended", 0);
            let hitchhiker = b.add_dir("Hitchhiker", 0);
            let deep = b.add_dir("Extra", hitchhiker);
            b.add_file_in(suspended, "Story.Data", b"SUSPENDED-BYTES");
            b.add_file_in(hitchhiker, "Story.Data", b"HITCHHIKER-BYTES");
            b.add_file_in(deep, "Story.Data", b"NESTED-BYTES");
            b.add_file("Disk.info", b"root-level");
            let adf = Adf::mount(b.image).expect("a synthetic AmigaDOS image mounts");

            let mut paths: Vec<&str> = adf.files().iter().map(|e| e.path.as_str()).collect();
            paths.sort_unstable();
            assert_eq!(
                paths,
                vec![
                    "Disk.info",
                    "Hitchhiker/Extra/Story.Data",
                    "Hitchhiker/Story.Data",
                    "Suspended/Story.Data",
                ],
                "ffs={ffs}: every file is named by the directory it lives in, and a file at \
                 the root is named by itself",
            );

            // …and asking for a path gets THAT file, which is the whole point.
            assert_eq!(
                adf.read_named("Hitchhiker/Story.Data").as_deref(),
                Some(&b"HITCHHIKER-BYTES"[..]),
                "ffs={ffs}: the path resolves to its own file",
            );
            assert_eq!(
                adf.read_named("Suspended/Story.Data").as_deref(),
                Some(&b"SUSPENDED-BYTES"[..]),
                "ffs={ffs}",
            );
            assert_eq!(
                adf.read_named("Hitchhiker/Extra/Story.Data").as_deref(),
                Some(&b"NESTED-BYTES"[..]),
                "ffs={ffs}: two levels deep",
            );
            // The bare-name fallback still answers, for callers that hand over a
            // filename — the `--pictures` door — and answers as it always did.
            assert!(
                adf.read_named("Story.Data").is_some(),
                "ffs={ffs}: a bare name still resolves, to the first match in block order",
            );
        }
    }



    /// Builder for a synthetic disk image, so the codec tests need no fixture.
    struct DiskBuilder {
        image: Vec<u8>,
        next: usize,
        /// The block [`DiskBuilder::add_file`] last wrote a header into, so
        /// [`DiskBuilder::add_file_in`] can point it at a parent afterwards.
        last_header: usize,
    }

    impl DiskBuilder {
        fn new(ffs: bool) -> DiskBuilder {
            let mut image = vec![0u8; DD_BLOCKS * BSIZE];
            image[0..3].copy_from_slice(b"DOS");
            image[3] = u8::from(ffs);
            // Files start after the root block (880), like a real disk.
            DiskBuilder { image, next: 881, last_header: 0 }
        }

        fn put32(&mut self, block: usize, off: usize, v: u32) {
            let at = block * BSIZE + off;
            self.image[at..at + 4].copy_from_slice(&v.to_be_bytes());
        }

        fn ffs(&self) -> bool {
            self.image[3] & 1 != 0
        }

        /// Write a file, spilling into extension blocks the way AmigaDOS does.
        /// Write a DIRECTORY header and return its block, for the parent chain
        /// (SQ-0908). `parent` is 0 for a directory sitting at the volume root.
        fn add_dir(&mut self, name: &str, parent: usize) -> usize {
            let b = self.next;
            self.next += 1;
            self.put32(b, 0, T_HEADER);
            self.put32(b, 4, b as u32);
            self.put32(b, BSIZE - 4, ST_USERDIR);
            self.put32(b, PARENT, parent as u32);
            let at = b * BSIZE + BSIZE - 80;
            self.image[at] = name.len() as u8;
            self.image[at + 1..at + 1 + name.len()].copy_from_slice(name.as_bytes());
            b
        }

        /// [`Self::add_file`], inside a directory.
        fn add_file_in(&mut self, dir: usize, name: &str, data: &[u8]) {
            self.add_file(name, data);
            // `add_file` took the block it was standing on; point it at its parent.
            let header = self.last_header;
            self.put32(header, PARENT, dir as u32);
        }

        fn add_file(&mut self, name: &str, data: &[u8]) {
            let header = self.next;
            self.last_header = header;
            self.next += 1;
            let payload = if self.ffs() { BSIZE } else { BSIZE - OFS_DATA_HEADER };
            let chunks: Vec<&[u8]> = if data.is_empty() {
                Vec::new()
            } else {
                data.chunks(payload).collect()
            };

            self.put32(header, 0, T_HEADER);
            self.put32(header, 4, header as u32);
            self.put32(header, BSIZE - 4, ST_FILE);
            self.put32(header, BSIZE - 188, data.len() as u32);
            let at = header * BSIZE + BSIZE - 80;
            self.image[at] = name.len() as u8;
            self.image[at + 1..at + 1 + name.len()].copy_from_slice(name.as_bytes());

            let mut owner = header;
            let mut in_owner = 0usize;
            for (seq, chunk) in chunks.iter().enumerate() {
                if in_owner == DATA_TABLE_LEN {
                    // Chain a fresh extension block and continue there.
                    let ext = self.next;
                    self.next += 1;
                    self.put32(owner, 8, in_owner as u32);
                    self.put32(owner, BSIZE - 8, ext as u32);
                    self.put32(ext, 0, T_LIST);
                    self.put32(ext, 4, ext as u32);
                    self.put32(ext, BSIZE - 4, ST_FILE);
                    owner = ext;
                    in_owner = 0;
                }
                let db = self.next;
                self.next += 1;
                if self.ffs() {
                    let at = db * BSIZE;
                    self.image[at..at + chunk.len()].copy_from_slice(chunk);
                } else {
                    self.put32(db, 0, T_DATA);
                    self.put32(db, 4, header as u32);
                    self.put32(db, 8, seq as u32 + 1);
                    self.put32(db, 12, chunk.len() as u32);
                    let at = db * BSIZE + OFS_DATA_HEADER;
                    self.image[at..at + chunk.len()].copy_from_slice(chunk);
                }
                self.put32(owner, DATA_TABLE_TOP - 4 * in_owner, db as u32);
                in_owner += 1;
            }
            self.put32(owner, 8, in_owner as u32);
        }
    }

    /// One synthetic OFS floppy carrying `files`, for the mount-seam tests in
    /// [`crate::medium`]. They need a real volume of every format and cannot
    /// reach a builder that is private to this module.
    pub(crate) fn sample_disk(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut d = DiskBuilder::new(false);
        for (name, data) in files {
            d.add_file(name, data);
        }
        d.image
    }

    /// A minimal but structurally valid v6 story header, padded to `len`.
    fn fake_story(len: usize) -> Vec<u8> {
        let mut b = vec![0u8; len];
        b[0] = 6;
        let mut word = |o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_be_bytes());
        word(0x04, 0x0400); // high memory
        word(0x08, 0x0300); // dictionary
        word(0x0a, 0x0100); // objects
        word(0x0c, 0x0200); // globals
        word(0x0e, 0x0280); // static memory base
        word(0x1a, (len / 8) as u16); // file length, v6 unit
        b[0x12..0x18].copy_from_slice(b"890323");
        b
    }

    #[test]
    fn rejects_non_amiga_images() {
        assert!(!Adf::looks_like_adf(b"not a disk"));
        assert!(!Adf::looks_like_adf(&vec![0u8; DD_BLOCKS * BSIZE]), "no DOS magic");
        let mut odd = vec![0u8; BSIZE * 3 + 1];
        odd[0..3].copy_from_slice(b"DOS");
        assert!(!Adf::looks_like_adf(&odd), "not a whole number of blocks");
        assert_eq!(Adf::mount(vec![0u8; 16]).unwrap_err(), AdfError::NotAdf);
    }

    /// The OFS and FFS data-block layouts differ; both must round-trip, and a
    /// file long enough to need extension blocks must survive the chain walk.
    #[test]
    fn round_trips_files_through_both_filesystems() {
        for ffs in [false, true] {
            let mut d = DiskBuilder::new(ffs);
            let small = b"hello from 1989".to_vec();
            // 80 blocks' worth: more than one 72-entry data table.
            let big: Vec<u8> = (0..80 * BSIZE).map(|i| (i % 251) as u8).collect();
            d.add_file("small", &small);
            d.add_file("Long.data", &big);
            let adf = Adf::mount(d.image).expect("mounts");

            assert_eq!(adf.is_ffs(), ffs);
            let names: Vec<&str> = adf.files().iter().map(|e| e.name.as_str()).collect();
            assert_eq!(names, ["small", "Long.data"], "ffs={ffs}");
            assert_eq!(adf.read_named("SMALL").as_deref(), Some(&small[..]), "ffs={ffs}");
            assert_eq!(adf.read_named("long.data").as_deref(), Some(&big[..]), "ffs={ffs}");
            assert_eq!(adf.read_named("absent"), None);
        }
    }

    #[test]
    fn finds_a_story_by_content_not_by_name() {
        let mut d = DiskBuilder::new(false);
        d.add_file("Zork Zero", b"\x00\x00\x03\xf3 amiga executable");
        d.add_file("Bootleg", &fake_story(4096));
        let adf = Adf::mount(d.image).expect("mounts");
        let (name, bytes) = adf.story().expect("the story is found under any name");
        assert_eq!(name, "Bootleg");
        assert_eq!(bytes.len(), 4096);
    }

    /// A disk with no story at all (Zork Zero's Disk0 is exactly this) must say
    /// so rather than hand back the first plausible-looking file.
    #[test]
    fn a_boot_disk_offers_no_story_and_no_pictures() {
        let mut d = DiskBuilder::new(false);
        d.add_file("startup-sequence", b"LoadWB\ndelay\n");
        d.add_file("icon.library", &vec![0x11u8; 6100]);
        let adf = Adf::mount(d.image).expect("mounts");
        assert_eq!(adf.story(), None);
        assert!(adf.pictures().is_none());
    }

    /// Two candidates: Infocom's convention decides, and only then size.
    #[test]
    fn the_conventional_name_only_breaks_a_tie() {
        let mut d = DiskBuilder::new(false);
        d.add_file("Backup.data", &fake_story(8192));
        d.add_file("Story.data", &fake_story(4096));
        let adf = Adf::mount(d.image).expect("mounts");
        assert_eq!(adf.story().expect("a story").0, "Story.data", "convention beats size");

        let mut d = DiskBuilder::new(false);
        d.add_file("Alpha", &fake_story(4096));
        d.add_file("Beta", &fake_story(8192));
        let adf = Adf::mount(d.image).expect("mounts");
        assert_eq!(adf.story().expect("a story").0, "Beta", "no convention → the largest");
    }

    /// Saved games and Workbench icons sit beside the story on a real disk and
    /// must not be mistaken for one. Both patterns are taken from Disk1.
    #[test]
    fn saved_games_and_icons_are_not_stories() {
        assert!(!looks_like_story(&[0x03, 0xaa, 0, 0, 0, 0, 0, 0]), "a truncated save");
        let mut save = vec![0u8; 31232];
        save[0] = 0x03;
        save[1] = 0xaa;
        assert!(!looks_like_story(&save), "Zork Zero's `bine` saved game");
        let mut icon = vec![0u8; 413];
        icon[0] = 0xe3;
        icon[1] = 0x10;
        assert!(!looks_like_story(&icon), "a Workbench .info sidecar");
        assert!(!looks_like_story(&[0x00, 0x00, 0x03, 0xf3]), "a 68k HUNK executable");
    }

    /// **A serial written in the Apple II's high ASCII is still a serial**
    /// (SQ-0856). `LEATHRGODDESSES` on *Lost Treasures* volume `INFOCOM6` carries
    /// `C2 EC EF F7 EE A1` at `$12`, which is `42 6C 6F 77 6E 21` with bit 7 off:
    /// "Blown!", a joke serial, not corruption. The rule masks the bit, so the
    /// same six characters pass in either encoding.
    ///
    /// FALSIFICATION: drop the `& 0x7f` from the serial check and the second
    /// assertion here fails, which is the reported symptom — the game invisible
    /// on a volume that holds it.
    #[test]
    fn a_high_ascii_serial_is_still_an_ascii_serial() {
        let mut plain = fake_story(4096);
        plain[0x12..0x18].copy_from_slice(b"Blown!");
        assert!(looks_like_story(&plain), "the same six characters in plain ASCII");

        let mut high = fake_story(4096);
        high[0x12..0x18].copy_from_slice(&[0xc2, 0xec, 0xef, 0xf7, 0xee, 0xa1]);
        assert_eq!(
            high[0x12..0x18].iter().map(|c| char::from(c & 0x7f)).collect::<String>(),
            "Blown!",
            "the bytes off INFOCOM6, read the way the Apple II wrote them"
        );
        assert!(looks_like_story(&high), "Leather Goddesses of Phobos, off INFOCOM6");

        // A high-bit SPACE (`A0`) is the other thing that machine writes, and it
        // is as printable as `20` is.
        let mut padded = fake_story(4096);
        padded[0x12..0x18].copy_from_slice(&[0xb8, 0xb9, 0xb0, 0xb3, 0xb2, 0xa0]);
        assert!(looks_like_story(&padded), "high ASCII digits and a trailing space");
    }

    /// Versions 1 and 2 are story files too (SQ-1422), and this is the gate the
    /// app loads every story through — refuse them here and `zvm` supporting
    /// them reaches no player. ZMSD §11.1.6 puts their length unit at 2, the
    /// same as Version 3's.
    ///
    /// The other half is that widening byte 0 must widen nothing else, and 1 and
    /// 2 are far commoner leading bytes than 3–8: of 320 files in one real
    /// `stories/` directory, 37 open with one — Apple II `.po`/`.dsk` images and
    /// the `.eg1`/`.cg1`/`.mg1`/`.pic` graphics beside the Version 6 games — and
    /// every one of them fails on a control byte in the serial field, which is
    /// the shape asserted below.
    #[test]
    fn a_version_one_or_two_header_is_a_story_and_a_graphics_file_is_not() {
        for v in 1..=8u8 {
            let mut b = fake_story(4096);
            b[0] = v;
            let unit: u16 = match v {
                1..=3 => 2,
                4 | 5 => 4,
                _ => 8,
            };
            b[0x1a..0x1c].copy_from_slice(&(4096 / unit).to_be_bytes());
            assert!(looks_like_story(&b), "v{v} must load");
        }
        // The leading bytes of `arthur.eg1`, `journey.cg1` and `Arthur.po` — a
        // Version 1 byte 0 apiece, and a serial field full of control bytes.
        for (serial, who) in [
            ([0x48u8, 0x02, 0xc4, 0x00, 0x00, 0x00], "arthur.eg1"),
            ([0xdc, 0x00, 0x7f, 0x00, 0x08, 0x00], "journey.cg1"),
            ([0x4a, 0x09, 0xc0, 0x85, 0x49, 0xa0], "Arthur.po"),
        ] {
            let mut b = fake_story(4096);
            b[0] = 1;
            b[0x1a..0x1c].copy_from_slice(&(4096u16 / 2).to_be_bytes());
            b[0x12..0x18].copy_from_slice(&serial);
            assert!(!looks_like_story(&b), "{who} is not a Version 1 story");
        }
    }

    /// The masking above widens what a serial may be; it must widen nothing
    /// else. **Control bytes are still control bytes with bit 7 either way** —
    /// which is the whole reason a saved game does not pass, since its `$12..$18`
    /// is binary rather than text.
    #[test]
    fn a_binary_serial_is_still_rejected_with_bit_seven_masked() {
        // The exact serial fields of the real saved games in `stories/`:
        // `LURK1.SAV` and friends are all-zero there, and `Story.Save` on the
        // Zork III floppy is `24 6D 07 39 2A 65` — printable but for the `07`.
        for (serial, who) in [
            ([0x00u8; 6], "an all-zero serial, as every .SAV on the corpus has"),
            ([0x24, 0x6d, 0x07, 0x39, 0x2a, 0x65], "Zork III's `Story.Save`"),
            ([0x80, 0x80, 0x80, 0x80, 0x80, 0x80], "high-bit NULs"),
            ([0x9f, 0x9f, 0x9f, 0x9f, 0x9f, 0x9f], "high-bit C1 controls"),
            ([0xff, 0xff, 0xff, 0xff, 0xff, 0xff], "high-bit DEL"),
            ([b'8', b'9', 0x7f, b'3', b'2', b'3'], "one DEL in an otherwise fine serial"),
        ] {
            let mut b = fake_story(4096);
            b[0x12..0x18].copy_from_slice(&serial);
            assert!(!looks_like_story(&b), "{who}");
        }
    }

    #[test]
    fn blorb_and_glulx_images_count_as_stories() {
        let mut blorb = b"FORM\x00\x00\x00\x04IFRS".to_vec();
        blorb.push(0);
        assert!(looks_like_story(&blorb));
        assert!(looks_like_story(b"Glul\x00\x03\x01\x02"));
    }

    /// Real media: mount the original Zork Zero disks if the user happens to
    /// have them. They live outside the repo, so this skips vacuously.
    #[test]
    fn real_zork_zero_disks() {
        let Some(home) = std::env::var_os("HOME") else { return };
        let dir = std::path::Path::new(&home)
            .join("Downloads/Zork Zero - The Revenge of Megaboz");
        let disk1 = dir.join("Zork Zero - The Revenge of Megaboz_Disk1.adf");
        let Ok(bytes) = std::fs::read(&disk1) else {
            eprintln!("SKIP: original Amiga media absent at {}", disk1.display());
            return;
        };
        let adf = Adf::mount(bytes).expect("Disk1 mounts");
        assert!(!adf.is_ffs(), "the release disks are OFS");
        assert_eq!(adf.files().len(), 13);
        let (name, story) = adf.story().expect("Story.data is found");
        assert_eq!(name, "Story.data");
        assert_eq!(story.len(), 296448);
        assert_eq!(story[0], 6, "Zork Zero is v6");
        assert_eq!(&story[0x12..0x18], b"890323");
        let (pname, pics) = adf.pictures().expect("Pic.data is found");
        assert_eq!(pname, "Pic.data");
        assert_eq!(pics.part(), 1);
        assert_eq!(pics.entries().len(), 495);
        assert!(pics.decode(1).is_ok(), "picture 1 decodes straight off the disk");

        let disk0 = dir.join("Zork Zero - The Revenge of Megaboz_Disk0.adf");
        if let Ok(bytes) = std::fs::read(&disk0) {
            let boot = Adf::mount(bytes).expect("Disk0 mounts");
            assert_eq!(boot.files().len(), 16);
            assert_eq!(boot.story(), None, "the boot disk carries no game");
        }
    }
}
