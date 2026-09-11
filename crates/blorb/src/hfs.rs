//! Read a Macintosh disk image — a DiskCopy 4.2 `.image` or a bare HFS volume —
//! well enough to pull a story file and its native Infocom picture archive
//! straight off the original release media, with no extraction step.
//!
//! This is the Macintosh sibling of [`crate::adf`], and it exists for the same
//! reason: Infocom shipped these games on floppies, and a floppy is a container
//! lanthorn can open rather than something a person has to unpack first.
//! Verified against `Zork Zero Disk.image` — Zork Zero, **version 6, release
//! 296, serial 881019**, a build 97 releases earlier than the r393/890714 that
//! every PC medium in the corpus carries.
//!
//! macOS itself is no help here: `hdiutil attach` refuses an HFS-standard image
//! on anything past 10.14, so the whole chain has to be ours. It is three layers
//! deep and every one of them is hand-rolled, exactly as `blorb`'s zero
//! dependencies require.
//!
//! # Layer 1 — the DiskCopy 4.2 wrapper
//!
//! An 84-byte header, then the volume, then the tags:
//!
//! | offset | field |
//! |---|---|
//! | `0x00` | 64-byte Pascal disk name |
//! | `0x40` | data size, big-endian u32 — the volume |
//! | `0x44` | tag size, big-endian u32 |
//! | `0x50` | encoding |
//! | `0x51` | format |
//! | `0x52` | `0x0100`, the format's magic |
//!
//! **The tag bytes are not part of the volume.** Apple's 800K floppies carried
//! 12 bytes of tag per 512-byte sector, so a 1600-sector disk drags 19 200 bytes
//! along behind its 819 200, and a reader that folds them into the filesystem
//! shifts every offset past the end of the data. Take the volume as exactly the
//! `data size` bytes that follow the header, and ignore the rest.
//!
//! A bare volume with no wrapper is read just the same — see
//! [`Hfs::looks_like_hfs`], which tries both placements and lets the volume's own
//! signature decide.
//!
//! # Layer 2 — HFS, which is not MFS
//!
//! The signature two logical blocks in (volume offset 1024) is `0x4244`, `BD`:
//! HFS, with a B*-tree catalog. Flat MFS would say `0xD2D7`, and nothing here
//! would read it. The Master Directory Block that carries that signature also
//! carries the geometry every offset below is computed from — the allocation
//! block size, where the allocation blocks start, and the extents of the two
//! special files:
//!
//! * the **catalog file**, whose leaf records name every file and folder;
//! * the **extents overflow file**, which holds the extents of any fork too
//!   fragmented for the three the catalog record has room for.
//!
//! Both are B*-trees of 512-byte nodes: a header node whose first record names
//! the first leaf, then a linked chain of leaves. Reading them means walking
//! that chain — no key comparisons, no tree descent, because we want *every*
//! record rather than one.
//!
//! Structure comes from Apple's *Inside Macintosh: Files*, chapter 2 ("Data
//! Organization on Volumes"), which lays out the MDB, the B*-tree node and the
//! catalog and extents records field by field.
//!
//! # Layer 3 — choosing what to run
//!
//! The scan is flat — a file in a folder is found without recursing into it —
//! but each entry now REPORTS the folder it was found in ([`HfsEntry::path`]).
//! What a story is, is still decided by CONTENT ([`crate::adf::looks_like_story`],
//! and [`crate::infocom_pics::InfocomPics::parse`] for the artwork); Infocom's
//! `Story.data` / `Pic.data` names remain a tiebreak, never a test.
//!
//! This reader used to drop the folder, on the stated grounds that "where it
//! sits is not a question anyone has to answer". On a one-game floppy that was
//! true. It is false on a compilation, and quietly so (SQ-0877): the Masterpieces
//! CD holds THREE files called `STORY.DATA` — Arthur's, Journey's and Zork
//! Zero's — so a listing showed one name three times, and `read_named` could
//! only ever reach the first. [`crate::fat12`] had already learnt this from the
//! Atari ST compilations, where four games call their story `STORY.DAT`, and
//! reports `HITCHHIK/STORY.DAT`; the two formats now spell a path alike.
//!
//! # What is on the Macintosh Zork Zero disk
//!
//! Measured, and worth writing down because it is the whole reason to look:
//!
//! | file | type/creator | data fork | what it is |
//! |---|---|---|---|
//! | `Story.data` | `INdf`/`IN0Z` | 295 936 | the story — v6, r296, s881019 |
//! | `CPic.data` | `INdf`/`IN0Z` | 218 624 | **colour** artwork, 483 records, 14-byte |
//! | `Pic.data` | `INdf`/`IN0Z` | 239 104 | **monochrome** artwork, 483 records, 12-byte |
//! | `Zork Zero` | `APPL`/`IN0Z` | 0 (38 833 rsrc) | Infocom's own 68k interpreter |
//! | `Desktop` | `FNDR`/`ERIK` | 0 (1 665 rsrc) | the Finder's desktop database |
//!
//! So the Macintosh release ships **two** picture archives, one per screen the
//! machine had, and [`InfocomPics`] reads both (SQ-0838). They hold the same 483
//! records and the same 386 of them carry pixels; what differs is the screen
//! they were drawn for — 320×200 in sixteen colours, or 480×300 in two. The
//! monochrome one declares 12-byte directory records, having no palette to
//! point at, and its header flags read `0x0e`, which is bocfel's monochrome
//! marker and is Zork Zero's ordinary `0x06` plus the `GF_MONO` bit.
//!
//! [`Hfs::pictures`] lands on the **colour** archive, now by preference rather
//! than by parse: monochrome is a thing to ask for, not a thing to be given.

use crate::adf::looks_like_story;
use crate::infocom_pics::InfocomPics;

/// A logical block: the unit the MDB and the B*-tree nodes are measured in.
pub const BLOCK: usize = 512;

/// Bytes of DiskCopy 4.2 header ahead of the volume.
///
/// Shared with [`crate::prodos`]: DiskCopy is a wrapper, not a filesystem, and
/// an Apple II 800 KB ProDOS volume arrives inside one exactly as a Macintosh
/// volume does (SQ-0889).
pub(crate) const DISKCOPY_HEADER: usize = 84;
/// `dataSize` — the volume's length in bytes.
const DISKCOPY_DATA_SIZE: usize = 0x40;
/// `tagSize` — the sector tags, which FOLLOW the volume and are not part of it.
const DISKCOPY_TAG_SIZE: usize = 0x44;
/// `private`, the format's magic number.
const DISKCOPY_MAGIC_OFF: usize = 0x52;
const DISKCOPY_MAGIC: u16 = 0x0100;

/// The Master Directory Block sits two logical blocks into the volume.
const MDB_OFFSET: usize = 2 * BLOCK;
/// `drSigWord` for HFS. (MFS, which this reader does not handle, is `0xD2D7`.)
const HFS_SIGNATURE: u16 = 0x4244;

// MDB fields, from *Inside Macintosh: Files*, "Master Directory Blocks".
const MDB_NM_AL_BLKS: usize = 18; // drNmAlBlks — allocation blocks on the volume
const MDB_AL_BLK_SIZ: usize = 20; // drAlBlkSiz — bytes per allocation block
const MDB_AL_BL_ST: usize = 28; // drAlBlSt — first allocation block, in 512-byte blocks
const MDB_VN: usize = 36; // drVN — volume name, Pascal, ≤27 bytes
const MDB_XT_FL_SIZE: usize = 130; // drXTFlSize — extents overflow file size
const MDB_XT_EXT_REC: usize = 134; // drXTExtRec — its first three extents
const MDB_CT_FL_SIZE: usize = 146; // drCTFlSize — catalog file size
const MDB_CT_EXT_REC: usize = 150; // drCTExtRec — its first three extents
/// Bytes of MDB this reader reads.
const MDB_LEN: usize = 162;

/// Every B*-tree node on an HFS volume is one logical block.
const NODE_SIZE: usize = BLOCK;
/// `ndFLink`, `ndBLink`, `ndType`, `ndNHeight`, `ndNRecs`, `ndResv2`.
const NODE_HEADER: usize = 14;
/// `ndNRecs`, the record count, within that header.
const ND_N_RECS: usize = 10;
/// `bthFNode` — the first leaf node — within a header node's first record.
const BTH_F_NODE: usize = 10;

/// `cdrDirRec`: the catalog record for a folder.
const CDR_DIR: u8 = 1;
/// `cdrFilRec`: the catalog record for a plain file.
const CDR_FILE: u8 = 2;
/// `dirDirID` within a directory record, and the record's length.
const DIR_DIR_ID: usize = 6;
const DIR_REC_LEN: usize = 70;
/// The root folder's CNID: every parent chain ends here.
const ROOT_CNID: u32 = 2;
/// The catalog file's own CNID, which is how its overflow extents are keyed.
const CATALOG_CNID: u32 = 4;

// Catalog file-record fields, offset from `cdrType`.
const FIL_TYPE: usize = 4; // filUsrWds.fdType
const FIL_CREATOR: usize = 8; // filUsrWds.fdCreator
const FIL_FL_NUM: usize = 20; // filFlNum — the file's CNID
const FIL_LG_LEN: usize = 26; // filLgLen — data fork logical length
const FIL_R_LG_LEN: usize = 36; // filRLgLen — resource fork logical length
const FIL_EXT_REC: usize = 74; // filExtRec — first three data-fork extents
const FIL_R_EXT_REC: usize = 86; // filRExtRec — first three RESOURCE-fork extents
const FILE_REC_LEN: usize = 102;

/// `xkrFkType` for a data fork.
const FORK_DATA: u8 = 0x00;
/// `xkrFkType` for a resource fork (SQ-0911). Both are kept now: a Macintosh
/// release keeps its story and artwork in data forks, which is why this reader
/// ignored resource forks for a long time — but it keeps its FONTS in the
/// resource fork, and those are worth reading.
const FORK_RSRC: u8 = 0xFF;
/// Bytes of extents-overflow key ahead of the record's own data.
const XKR_LEN: usize = 7;

/// One extent: a starting allocation block and a count.
type Extent = (u16, u16);
/// The three extents a catalog record or an overflow record carries.
type ExtentRecord = [Extent; 3];

/// Infocom's conventional names on a release disk. Never a test — only a
/// tiebreak when content identification finds more than one candidate. The Mac
/// ships both `Pic.data` and `CPic.data`, so both count as conventional.
const CONVENTIONAL_STORY: &str = "story.data";
const CONVENTIONAL_PICTURES: [&str; 2] = ["pic.data", "cpic.data"];

/// Errors that can arise while mounting a Macintosh disk image.
#[derive(Debug, PartialEq, Eq)]
pub enum HfsError {
    /// The bytes are not an HFS volume, wrapped or bare: no `BD` signature
    /// where one has to be, or a Master Directory Block whose geometry does not
    /// describe the image it sits in.
    NotHfs,
}

/// One file found on the volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HfsEntry {
    /// The Macintosh filename, as stored. See `mac_name` on the upper half.
    pub name: String,
    /// Data-fork size in bytes, from the catalog record.
    pub size: usize,
    /// Resource-fork size in bytes; a Macintosh application is *all* resource
    /// fork. Read it with [`Hfs::read_resource`].
    pub resource_size: usize,
    /// The Finder type, e.g. `INdf` for Infocom's data files, `APPL` for an
    /// application.
    pub file_type: [u8; 4],
    /// The Finder creator, e.g. `IN0Z` for Zork Zero.
    pub creator: [u8; 4],
    /// The catalog node id, unique on the volume, and the key its overflow
    /// extents are stored under.
    pub id: u32,
    /// The folder chain from the volume root, outermost first; empty at the
    /// root. **This is what names a game on a compilation** — `ARTHUR FOLDER`,
    /// `JOURNEY FOLDER` and `ZORK ZERO` are the only things telling three files
    /// called `STORY.DATA` apart.
    pub dirs: Vec<String>,
    /// The first three RESOURCE-fork extents; any more live in the overflow file.
    pub rsrc_extents: ExtentRecord,
    /// The first three data-fork extents; any more live in the overflow file.
    extents: ExtentRecord,
}

impl HfsEntry {
    /// The machine this file's Finder metadata names, or `None` when it names
    /// none this crate knows (SQ-0876).
    ///
    /// The RULE is [`crate::medium::machine_from_finder`]'s and is documented
    /// there, in one copy, because `iso9660` asks exactly the same question of
    /// the same two fields carried in a different place.
    pub fn machine(&self) -> Option<crate::medium::DiskImage> {
        crate::medium::machine_from_finder(&self.file_type, &self.creator)
    }

    /// Whether this file was copied in from a DOS volume rather than authored
    /// on the Macintosh — so a HYBRID disc's two halves can be told apart.
    pub fn is_from_dos(&self) -> bool {
        self.machine() == Some(crate::medium::DiskImage::Fat12Dos)
    }

    /// How this file is named to the outside world: `Folder/Sub/NAME` inside a
    /// folder, the bare name at the volume root.
    ///
    /// Slash-separated, which is [`crate::fat12::Fat12Entry::path`]'s spelling
    /// rather than the Finder's colon — a path here is read by the same callers
    /// for both formats, so the two agree. A Macintosh filename may legally
    /// contain `/` and never `:`; that makes this ambiguous in principle, and it
    /// stays a listing label rather than a key, exactly as
    /// [`crate::medium::DiskStory::name`] says.
    pub fn path(&self) -> String {
        if self.dirs.is_empty() {
            return self.name.clone();
        }
        format!("{}/{}", self.dirs.join("/"), self.name)
    }
}

/// A mounted Macintosh volume.
#[derive(Debug)]
pub struct Hfs {
    image: Vec<u8>,
    /// Where the volume starts in `image`: 0 bare, 84 inside a DiskCopy wrapper.
    volume: usize,
    /// Bytes per allocation block.
    alloc_size: usize,
    /// First allocation block, in 512-byte logical blocks from the volume start.
    alloc_start: usize,
    /// Allocation blocks on the volume.
    alloc_count: usize,
    name: String,
    files: Vec<HfsEntry>,
    /// Extents overflow records, as `(cnid, fork type, first block, extents)`.
    /// Both fork types are kept; the reader picks by [`FORK_DATA`]/[`FORK_RSRC`].
    overflow: Vec<(u32, u8, u16, ExtentRecord)>,
}

impl Hfs {
    /// Cheap sniff: does this look like a Macintosh disk image?
    ///
    /// By CONTENT — the `.image` extension the fixture happens to carry means
    /// nothing in particular, and this corpus has taught four separate times
    /// that a filename is not a format. The volume's own signature decides, and
    /// its geometry has to describe the image it sits in, so the two-block
    /// header a DiskCopy wrapper adds cannot be mistaken for a bare volume or
    /// the other way round. A Z-machine, Glulx, Blorb, Scott or AmigaDOS image
    /// can never collide: none of them has `BD` a kilobyte in.
    pub fn looks_like_hfs(bytes: &[u8]) -> bool {
        volume_offset(bytes).is_some() || raw_disc_volume(bytes)
    }

    /// Mount an image and enumerate its files.
    ///
    /// A raw CD dump is the one case that cannot be read in place — its user
    /// data is interrupted by a frame header every 2048 bytes — so the
    /// partition is gathered first and the disc's own bytes are dropped. Every
    /// other container, a partitioned `.iso` included, is an offset into the
    /// image as given.
    pub fn mount(image: Vec<u8>) -> Result<Hfs, HfsError> {
        let image = match volume_offset(&image) {
            Some(_) => image,
            None => crate::cd::hfs_partition(&image).ok_or(HfsError::NotHfs)?.extract(),
        };
        let volume = volume_offset(&image).ok_or(HfsError::NotHfs)?;
        let mdb = &image[volume + MDB_OFFSET..volume + MDB_OFFSET + MDB_LEN];
        let mut hfs = Hfs {
            alloc_size: be32(mdb, MDB_AL_BLK_SIZ) as usize,
            alloc_start: usize::from(be16(mdb, MDB_AL_BL_ST)),
            alloc_count: usize::from(be16(mdb, MDB_NM_AL_BLKS)),
            name: mac_name(&mdb[MDB_VN..]).unwrap_or_default(),
            volume,
            image,
            files: Vec::new(),
            overflow: Vec::new(),
        };

        // The extents overflow file can only be described by the MDB — it is
        // where every OTHER file's extra extents live, so it cannot have any of
        // its own. Read it first; the catalog may need it.
        let mdb = hfs.mdb().to_vec();
        let xt_size = be32(&mdb, MDB_XT_FL_SIZE) as usize;
        let xt = hfs.read_extents(&extent_record(&mdb, MDB_XT_EXT_REC), xt_size);
        hfs.overflow = overflow_records(&xt);
        let ct = extent_record(&mdb, MDB_CT_EXT_REC);
        let catalog = hfs
            .read_fork(CATALOG_CNID, FORK_DATA, ct, be32(&mdb, MDB_CT_FL_SIZE) as usize)
            .ok_or(HfsError::NotHfs)?;
        hfs.files = catalog_files(&catalog);
        Ok(hfs)
    }

    /// The volume's name, as the Finder showed it.
    pub fn volume_name(&self) -> &str {
        &self.name
    }

    /// Every file on the volume, in catalog order. Folders are not listed: the
    /// scan is flat, so a file in one is found without recursing into it.
    pub fn files(&self) -> &[HfsEntry] {
        &self.files
    }

    fn mdb(&self) -> &[u8] {
        &self.image[self.volume + MDB_OFFSET..self.volume + MDB_OFFSET + MDB_LEN]
    }

    /// One allocation block, or `None` if it is off the end of the volume.
    fn alloc_block(&self, n: usize) -> Option<&[u8]> {
        if n >= self.alloc_count {
            return None;
        }
        let at = self.volume + self.alloc_start * BLOCK + n * self.alloc_size;
        self.image.get(at..at + self.alloc_size)
    }

    /// Concatenate `extents`, stopping once `limit` bytes are in hand. A run
    /// that leaves the volume ends the read where it is; the caller checks the
    /// length it got.
    fn read_extents(&self, extents: &ExtentRecord, limit: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for (start, count) in extents {
            for i in 0..usize::from(*count) {
                if out.len() >= limit {
                    return out;
                }
                let Some(b) = self.alloc_block(usize::from(*start) + i) else {
                    return out;
                };
                out.extend_from_slice(b);
            }
        }
        out
    }

    /// Read one data fork whole: its first three extents, then as many overflow
    /// records as it takes. `None` when the chain runs short of `size`.
    fn read_fork(&self, cnid: u32, fork: u8, first: ExtentRecord, size: usize) -> Option<Vec<u8>> {
        let mut out = self.read_extents(&first, size);
        // Each pass consumes one extent record, and a fork cannot need more of
        // them than the overflow file holds in total.
        for _ in 0..=self.overflow.len() {
            if out.len() >= size {
                out.truncate(size);
                return Some(out);
            }
            // Overflow records are keyed by the fork-relative allocation block
            // the record starts at, which is exactly what has been read so far.
            let next = u16::try_from(out.len() / self.alloc_size).ok()?;
            let (_, _, _, extents) = self
                .overflow
                .iter()
                .find(|(id, fk, abn, _)| *id == cnid && *fk == fork && *abn == next)?;
            let more = self.read_extents(extents, size - out.len());
            if more.is_empty() {
                return None;
            }
            out.extend_from_slice(&more);
        }
        None
    }

    /// Read a file's data fork. `None` if its extents are broken or run short of
    /// the size the catalog declares.
    pub fn read(&self, entry: &HfsEntry) -> Option<Vec<u8>> {
        self.read_fork(entry.id, FORK_DATA, entry.extents, entry.size)
    }

    /// Read a file's RESOURCE fork (SQ-0911). `None` when it has none, or when
    /// its extents are broken or run short of the size the catalog declares.
    ///
    /// A Macintosh file is two forks, and until this quest only the data one was
    /// readable — a correct choice while the only things wanted were the story and
    /// the artwork, which Infocom keeps in data forks. The v6 releases keep their
    /// **bitmap fonts** in the resource fork, and `FONT` 1033 is fixed-pitch, so it
    /// is the one font in the whole corpus that fits lanthorn's cell model (the
    /// Amiga's is proportional — see `blorb::amiga_font`).
    ///
    /// The bytes come back raw. [`crate::resource_fork`] turns them into resources.
    pub fn read_resource(&self, entry: &HfsEntry) -> Option<Vec<u8>> {
        (entry.resource_size > 0)
            .then(|| self.read_fork(entry.id, FORK_RSRC, entry.rsrc_extents, entry.resource_size))
            .flatten()
    }

    /// Read a file by path or by bare name (case-insensitive), for callers that
    /// already know what they want. Prefer [`Hfs::story`] / [`Hfs::pictures`],
    /// which identify by content.
    ///
    /// The bare name still matches, so every `--pictures Pic.data` that worked
    /// on a single-game floppy works unchanged. The path is what reaches a
    /// PARTICULAR one on a compilation: `Pic.data` on the Masterpieces CD is
    /// three different archives, and only `JOURNEY FOLDER/PIC.DATA` says which.
    /// Same rule, same order, as [`crate::fat12::Fat12::read_named`].
    pub fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        let e = self
            .files
            .iter()
            .find(|e| e.path().eq_ignore_ascii_case(name) || e.name.eq_ignore_ascii_case(name))?;
        self.read(e)
    }

    /// The story image on this volume, with the path it was stored under.
    ///
    /// Every file is tested with [`looks_like_story`]; a volume with none yields
    /// `None`. When more than one passes, Infocom's `Story.data` convention
    /// breaks the tie, then the largest candidate, so the choice is
    /// deterministic rather than catalog-order luck. A compilation wants
    /// [`Hfs::files`] and a chooser, not this.
    ///
    /// The convention is matched on the file's own name, not on its path — a
    /// story is no less conventionally named for sitting in a folder.
    pub fn story(&self) -> Option<(String, Vec<u8>)> {
        let mut cands: Vec<(String, Vec<u8>)> = self
            .files
            .iter()
            .filter_map(|e| self.read(e).map(|b| (e.path(), b)))
            .filter(|(_, b)| looks_like_story(b))
            .collect();
        cands.sort_by_key(|(path, bytes)| {
            (!base_name(path).eq_ignore_ascii_case(CONVENTIONAL_STORY), std::cmp::Reverse(bytes.len()))
        });
        cands.into_iter().next()
    }

    /// The native Infocom picture archive on this volume, with its stored name.
    ///
    /// Identified by parsing, exactly as [`crate::adf::Adf::pictures`] does.
    ///
    /// **The Macintosh ships two archives, and both read** (SQ-0838) — a colour
    /// `CPic.data` and a monochrome `Pic.data` holding the same 483 pictures, one
    /// per screen Apple sold. So the choice is a real one now rather than
    /// something the parser settled by accident, and **colour wins**: it is what
    /// every other medium in this corpus supplies, it is what the automatic path
    /// has always drawn here, and choosing the two-colour art for a user whose
    /// terminal has sixteen million of them would need a reason nothing on the
    /// disk gives. bocfel makes the same call — its fallback table maps `Pic` to
    /// `kGraphicsTypeAmiga`, and monochrome is reached only when the user asks
    /// for it. Naming `Pic.data` by hand through `app`'s `PictureOverride` is
    /// how you ask for it here.
    ///
    /// After that, the conventional filenames, then picture count, so that a disk
    /// offering two archives of one depth still has a deterministic answer.
    pub fn pictures(&self) -> Option<(String, InfocomPics)> {
        let mut cands: Vec<(String, InfocomPics)> = self
            .files
            .iter()
            .filter_map(|e| self.read(e).map(|b| (e.path(), b)))
            .filter(|(_, b)| !looks_like_story(b))
            .filter_map(|(path, b)| InfocomPics::parse(b).ok().map(|p| (path, p)))
            .filter(|(_, p)| p.entries().iter().any(|e| e.has_pixels()))
            .collect();
        cands.sort_by_key(|(path, pics)| {
            let lower = base_name(path).to_ascii_lowercase();
            let conventional = CONVENTIONAL_PICTURES.contains(&lower.as_str());
            (
                crate::medium::art_preference(pics),
                !conventional,
                std::cmp::Reverse(pics.entries().len()),
            )
        });
        cands.into_iter().next()
    }
}

/// The filename at the end of a [`HfsEntry::path`] — what a naming CONVENTION is
/// tested against, since `Story.data` is no less conventional for living in a
/// folder.
fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

impl Hfs {
    /// The picture archive stored **beside** the story at `path` — same folder,
    /// same machine — or `None` when that story has no artwork of its own
    /// (SQ-0876).
    ///
    /// `None` here is an ANSWER and not a gap, which is the whole point.
    /// [`Hfs::pictures`] asks "what artwork is on this disk", and on a
    /// single-game floppy those are the same question. On a compilation they are
    /// not, and answering the easy one is how every graphical game on the
    /// Masterpieces CD came to be handed `MAC/ZORK ZERO/CPIC.DATA` — Journey
    /// and Arthur included, whose own archives sit one folder away. (Zork Zero's
    /// wins the volume-wide tiebreak on picture count, which is the point: the
    /// stranger a game gets is whichever archive happens to sort first.)
    ///
    /// Two filters, and the second is what "same machine" buys. The folder alone
    /// would already pair each game correctly on this disc; the machine test is
    /// what keeps that true on a hybrid volume that puts both pressings of one
    /// game in one folder, and it costs nothing to state now while the evidence
    /// ([`HfsEntry::is_from_dos`]) is in hand.
    ///
    /// The ranking within a folder is [`Hfs::pictures`]' own — colour over
    /// monochrome, then the conventional names, then picture count — so a
    /// Macintosh game still lands on its `CPic.data` and its `Pic.data` is still
    /// the thing you ask for by name.
    pub fn pictures_beside(&self, path: &str) -> Option<(String, InfocomPics)> {
        let story = self.files.iter().find(|e| e.path().eq_ignore_ascii_case(path))?;
        let (dirs, dos) = (story.dirs.clone(), story.is_from_dos());
        let mut cands: Vec<(String, InfocomPics)> = self
            .files
            .iter()
            .filter(|e| e.dirs == dirs && e.is_from_dos() == dos)
            .filter_map(|e| self.read(e).map(|b| (e.path(), b)))
            .filter(|(_, b)| !looks_like_story(b))
            .filter_map(|(p, b)| InfocomPics::parse(b).ok().map(|pics| (p, pics)))
            .filter(|(_, p)| p.entries().iter().any(|e| e.has_pixels()))
            .collect();
        cands.sort_by_key(|(p, pics)| {
            let lower = base_name(p).to_ascii_lowercase();
            let conventional = CONVENTIONAL_PICTURES.contains(&lower.as_str());
            (
                crate::medium::art_preference(pics),
                !conventional,
                std::cmp::Reverse(pics.entries().len()),
            )
        });
        cands.into_iter().next()
    }

    /// Whether the volume holds the story at `path` as a file imported from DOS.
    /// `None` when it holds no such file at all.
    pub fn is_from_dos(&self, path: &str) -> Option<bool> {
        self.files.iter().find(|e| e.path().eq_ignore_ascii_case(path)).map(HfsEntry::is_from_dos)
    }
}

/// Where the HFS volume starts inside `bytes` — 0 for a bare volume, 84 inside a
/// DiskCopy 4.2 wrapper, or a partition's own offset on an Apple-partitioned
/// medium — or `None` when no placement holds one.
///
/// Every arm here reads the volume **in place**. The one container that cannot
/// be — a raw CD dump, whose user data is not contiguous — is
/// [`raw_disc_volume`]'s, and [`Hfs::mount`] gathers it before asking this.
fn volume_offset(bytes: &[u8]) -> Option<usize> {
    if let Some(len) = diskcopy_volume_len(bytes) {
        if volume_is_sane(&bytes[DISKCOPY_HEADER..DISKCOPY_HEADER + len], len) {
            return Some(DISKCOPY_HEADER);
        }
    }
    // An Apple Partition Map, which a cooked `.iso` of a hybrid disc carries
    // exactly as the raw dump does — and a partitioned hard-disk image too. It
    // costs a few block reads and no copying at all (SQ-0870).
    if let Some(at) = crate::cd::hfs_partition(bytes).and_then(|p| p.contiguous_at()) {
        let present = bytes.len() - at;
        if volume_is_sane(&bytes[at..], present) {
            return Some(at);
        }
    }
    volume_is_sane(bytes, bytes.len()).then_some(0)
}

/// Does `bytes` hold a Macintosh volume inside a **raw** CD dump — the one
/// container whose bytes have to be gathered rather than indexed?
///
/// The sniff still copies nothing but a volume header, so a directory scan may
/// ask it of a 354 MB file.
fn raw_disc_volume(bytes: &[u8]) -> bool {
    crate::cd::hfs_partition(bytes)
        .filter(|p| p.contiguous_at().is_none())
        .is_some_and(|p| volume_is_sane(&p.head(MDB_OFFSET + MDB_LEN), p.len()))
}

/// The volume length a DiskCopy 4.2 header declares, when `bytes` carries one.
///
/// Nothing here is Macintosh-specific — it reads the wrapper's own declared
/// geometry and checks it against the bytes in hand — so [`crate::prodos`]
/// asks it the same question for its Apple II images (SQ-0889). What the
/// wrapper contains is the caller's to decide: each reader runs its own volume
/// sniff at [`DISKCOPY_HEADER`] and declines what is not its filesystem.
pub(crate) fn diskcopy_volume_len(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < DISKCOPY_HEADER || be16(bytes, DISKCOPY_MAGIC_OFF) != DISKCOPY_MAGIC {
        return None;
    }
    let data = be32(bytes, DISKCOPY_DATA_SIZE) as usize;
    let tag = be32(bytes, DISKCOPY_TAG_SIZE) as usize;
    // The tags follow the volume; both have to fit, and a volume is a whole
    // number of logical blocks.
    let total = DISKCOPY_HEADER.checked_add(data)?.checked_add(tag)?;
    (data >= 3 * BLOCK && data.is_multiple_of(BLOCK) && total <= bytes.len()).then_some(data)
}

/// Does `volume` open with a Master Directory Block that describes it?
///
/// `volume` is the volume's leading bytes — at least the MDB — and `available`
/// is how many of the volume's bytes are actually here, which is not always
/// `volume.len()`: the caller may be holding a header copied out of a raw disc.
///
/// The signature alone is two bytes and would fire on noise; the geometry is
/// what makes this safe. An allocation block is a whole number of logical
/// blocks, there is at least one of them, the allocation blocks start past the
/// boot blocks and the MDB and the first of them is inside the image — and then
/// **the blocks the mount is about to follow have to be here**, which is the two
/// B*-trees the MDB itself points at.
///
/// # What this checks instead of the volume's nominal size (SQ-0870)
///
/// It used to require the last allocation block the MDB *claims* to be inside
/// the image: `start * BLOCK + count * alloc_size <= volume.len()`. That is the
/// wrong quantity for any HFS volume that does not fill its container, and on a
/// **hybrid CD that is the normal case** — the Apple_HFS partition is sized for
/// the disc and shares it with the ISO9660 side. Measured on the *Masterpieces*
/// disc's `Masterpieces` volume:
///
/// ```text
///   drNmAlBlks 64,998 x drAlBlkSiz 10,240 = 665,589,248 claimed
///   drFreeBks  34,958                     = 357,969,920 free, and absent
///   the partition holds                     307,992,064
/// ```
///
/// A map claiming 634.8 MB of a disc whose whole payload is 308 MB is not a sign
/// of damage; it is a partition table describing the medium. Every allocated
/// block is present and only the free tail is missing, so the volume mounts and
/// reads — and the old bound declined it, which is what left `zvm-cli` answering
/// `Z-machine version 0 is not supported` for a perfectly good disc.
///
/// **A genuinely truncated volume is still refused, one layer down and more
/// precisely than here.** [`Hfs::alloc_block`] answers `None` for a block past
/// the end of the image and [`Hfs::read_fork`] refuses a fork whose chain runs
/// short of the length its catalog declares, so a cut-off volume yields *no*
/// story rather than half of one, and a cut-off catalog fails the mount
/// outright. That is the check this one is the cheap prefix of: what a reader
/// FOLLOWS must be present, and a missing tail nobody follows costs nothing.
fn volume_is_sane(volume: &[u8], available: usize) -> bool {
    if volume.len() < MDB_OFFSET + MDB_LEN || be16(volume, MDB_OFFSET) != HFS_SIGNATURE {
        return false;
    }
    let mdb = &volume[MDB_OFFSET..MDB_OFFSET + MDB_LEN];
    let alloc_size = be32(mdb, MDB_AL_BLK_SIZ) as usize;
    let count = usize::from(be16(mdb, MDB_NM_AL_BLKS));
    let start = usize::from(be16(mdb, MDB_AL_BL_ST));
    if alloc_size == 0 || !alloc_size.is_multiple_of(BLOCK) || count == 0 || start < 3 {
        return false;
    }
    // The allocation blocks have to START inside the image, whatever the MDB
    // says about where they end. This is the whole of what remains of a
    // length check, and it is what a volume of noise fails.
    // (`u64` throughout, because `drAlBlkSiz` is a 32-bit field and a corrupt
    // one must not overflow a 32-bit `usize` on the way to being refused.)
    if (start as u64) * (BLOCK as u64) + (alloc_size as u64) > available as u64 {
        return false;
    }
    // Then the catalog and the extents overflow file, whose extents the MDB
    // carries because nothing else could. The mount reads both immediately, so
    // a volume whose B*-trees are outside the image is not one this reader can
    // open — and a plausible pointer INTO the volume is what keeps a two-byte
    // signature from firing on noise now that the nominal size no longer does.
    let present = |extents: ExtentRecord, size: u32| -> bool {
        // A file the MDB declares empty points nowhere and is followed nowhere.
        // The overflow file is legitimately empty on a volume with no fragmented
        // fork; a catalog is not, but an all-zero MDB is refused by the geometry
        // above and a *malformed* one is the mount's business rather than the
        // sniff's — declining it here would turn "this disk holds no story" into
        // "this is not a disk", which is a worse answer to the same file.
        if size == 0 {
            return true;
        }
        let (first, run) = extents[0];
        let last = u64::from(first) + u64::from(run);
        let end = (start as u64) * (BLOCK as u64) + last * (alloc_size as u64);
        run > 0 && last <= count as u64 && end <= available as u64
    };
    present(extent_record(mdb, MDB_CT_EXT_REC), be32(mdb, MDB_CT_FL_SIZE))
        && present(extent_record(mdb, MDB_XT_EXT_REC), be32(mdb, MDB_XT_FL_SIZE))
}

/// The three extents at `off`.
fn extent_record(b: &[u8], off: usize) -> ExtentRecord {
    [
        (be16(b, off), be16(b, off + 2)),
        (be16(b, off + 4), be16(b, off + 6)),
        (be16(b, off + 8), be16(b, off + 10)),
    ]
}

/// Every data-fork record in the extents overflow file, as
/// `(cnid, first fork-relative allocation block, extents)`.
fn overflow_records(tree: &[u8]) -> Vec<(u32, u8, u16, ExtentRecord)> {
    let mut out = Vec::new();
    for rec in leaf_records(tree) {
        if usize::from(rec[0]) < XKR_LEN || rec.len() < XKR_LEN + 1 + 12 {
            continue;
        }
        if rec[1] != FORK_DATA && rec[1] != FORK_RSRC {
            continue;
        }
        // The key is `xkrKeyLen` plus that many bytes; the data follows, and
        // both halves are word-aligned by construction (7 + 1 is even).
        out.push((be32(rec, 2), rec[1], be16(rec, 6), extent_record(rec, XKR_LEN + 1)));
    }
    out
}

/// Every file the catalog names, wherever it lives on the volume.
fn catalog_files(tree: &[u8]) -> Vec<HfsEntry> {
    let mut out = Vec::new();
    let mut folders: std::collections::HashMap<u32, (String, u32)> = Default::default();
    for rec in leaf_records(tree) {
        let key_len = usize::from(rec[0]);
        if key_len < 6 {
            continue;
        }
        let Some(name) = mac_name(&rec[6..]) else { continue };
        let data = (1 + key_len).next_multiple_of(2);
        let Some(d) = rec.get(data..data + DIR_REC_LEN) else { continue };
        if d[0] == CDR_DIR {
            folders.insert(be32(d, DIR_DIR_ID), (name, be32(rec, 2)));
        }
    }
    for rec in leaf_records(tree) {
        // Key: length byte, reserved byte, parent id, then a Pascal name. The
        // record's data starts at the next even offset past it.
        let key_len = usize::from(rec[0]);
        if key_len < 6 {
            continue;
        }
        let Some(name) = mac_name(&rec[6..]) else { continue };
        let data = (1 + key_len).next_multiple_of(2);
        let Some(d) = rec.get(data..data + FILE_REC_LEN) else { continue };
        if d[0] != CDR_FILE {
            continue;
        }
        let mut dirs = Vec::new();
        let mut at = be32(rec, 2);
        while at != ROOT_CNID && dirs.len() < 32 {
            let Some((folder, up)) = folders.get(&at) else { break };
            dirs.push(folder.clone());
            at = *up;
        }
        dirs.reverse();
        out.push(HfsEntry {
            name,
            dirs,
            size: be32(d, FIL_LG_LEN) as usize,
            resource_size: be32(d, FIL_R_LG_LEN) as usize,
            file_type: [d[FIL_TYPE], d[FIL_TYPE + 1], d[FIL_TYPE + 2], d[FIL_TYPE + 3]],
            creator: [d[FIL_CREATOR], d[FIL_CREATOR + 1], d[FIL_CREATOR + 2], d[FIL_CREATOR + 3]],
            id: be32(d, FIL_FL_NUM),
            extents: extent_record(d, FIL_EXT_REC),
            rsrc_extents: extent_record(d, FIL_R_EXT_REC),
        });
    }
    out
}

/// Every record in every leaf of a B*-tree, in order.
///
/// Node 0 is the header node; the first record in it is the header record, and
/// `bthFNode` names the first leaf. From there the leaves are a linked list
/// through `ndFLink`, which is what makes reading all of them a walk rather than
/// a tree descent. The walk is bounded by the number of nodes in the file, so a
/// corrupt image that links a leaf to itself terminates.
fn leaf_records(tree: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let Some(header) = tree.get(..NODE_SIZE) else { return out };
    let Some(first) = node_records(header).first().map(|r| be32(r, BTH_F_NODE)) else {
        return out;
    };
    let mut n = first as usize;
    for _ in 0..tree.len() / NODE_SIZE {
        if n == 0 {
            break;
        }
        let Some(node) = tree.get(n * NODE_SIZE..(n + 1) * NODE_SIZE) else { break };
        out.extend(node_records(node));
        n = be32(node, 0) as usize;
    }
    out
}

/// The records in one node.
///
/// A node's records grow from the front and their offsets grow *backwards* from
/// the end, one 16-bit offset per record plus one more marking the free space —
/// so record `i` runs from `offsets[i]` to `offsets[i + 1]`. Anything that does
/// not describe a forward-running record inside the node ends the node early
/// rather than panicking.
fn node_records(node: &[u8]) -> Vec<&[u8]> {
    let count = usize::from(be16(node, ND_N_RECS));
    let mut out = Vec::new();
    if count == 0 || 2 * (count + 1) > NODE_SIZE - NODE_HEADER {
        return out;
    }
    let at = |i: usize| usize::from(be16(node, NODE_SIZE - 2 * (i + 1)));
    for i in 0..count {
        let (start, end) = (at(i), at(i + 1));
        if start < NODE_HEADER || end > NODE_SIZE || end <= start {
            break;
        }
        out.push(&node[start..end]);
    }
    out
}

/// A Pascal string, or `None` when it is empty or over-runs its buffer.
///
/// Macintosh filenames are MacRoman. Its lower half is ASCII, and this reader
/// does **not** transliterate the upper half: the 128 characters above it are a
/// table that would have to be verified against Apple's own, and nothing here
/// selects a file by name — the story and the artwork are identified by content,
/// and the conventional names are ASCII. Bytes outside printable ASCII become
/// U+FFFD, so a name is safe to display and never silently wrong.
fn mac_name(pascal: &[u8]) -> Option<String> {
    let len = usize::from(*pascal.first()?);
    if len == 0 {
        return None;
    }
    let raw = pascal.get(1..1 + len)?;
    let printable = |c: &u8| if (0x20..0x7f).contains(c) { char::from(*c) } else { '\u{fffd}' };
    Some(raw.iter().map(printable).collect())
}

/// Big-endian word at `off`.
fn be16(b: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([b[off], b[off + 1]])
}

/// Big-endian longword at `off`.
fn be32(b: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 800K floppy's worth of volume: 1600 logical blocks.
    const VOLUME_BLOCKS: usize = 1600;
    /// Where this builder puts its allocation blocks, matching a real disk.
    const ALLOC_START: usize = 4;
    /// Allocation blocks reserved for EACH of the two B*-trees. Three was the
    /// original reservation and it capped the catalog at two leaves.
    const TREE_BLOCKS: usize = 12;

    /// Builder for a synthetic HFS volume, so the tests need no fixture.
    ///
    /// One allocation block per logical block (which is what an 800K disk uses),
    /// a catalog laid out as a header node plus one leaf, and files written
    /// contiguously — with a hook to fragment one, because the extents overflow
    /// file is the part a single-extent fixture would never exercise.
    struct VolumeBuilder {
        volume: Vec<u8>,
        /// Next free allocation block for file data.
        next: usize,
        /// Catalog leaf records, in the order they were added.
        catalog: Vec<Vec<u8>>,
        /// Extents overflow records: `(cnid, first block, extents)`.
        overflow: Vec<(u32, u16, ExtentRecord)>,
        next_cnid: u32,
    }

    impl VolumeBuilder {
        fn new() -> VolumeBuilder {
            VolumeBuilder {
                volume: vec![0u8; VOLUME_BLOCKS * BLOCK],
                // The two B*-trees take the first `2 * TREE_BLOCKS`.
                next: 2 * TREE_BLOCKS,
                catalog: Vec::new(),
                overflow: Vec::new(),
                next_cnid: 16,
            }
        }

        fn put16(&mut self, at: usize, v: u16) {
            self.volume[at..at + 2].copy_from_slice(&v.to_be_bytes());
        }

        fn put32(&mut self, at: usize, v: u32) {
            self.volume[at..at + 4].copy_from_slice(&v.to_be_bytes());
        }

        /// Catalogue a folder under `parent` and hand back its CNID, so files
        /// can be put inside it. A directory record carries no data at all —
        /// the folder IS its catalog entry.
        fn add_folder(&mut self, name: &str, parent: u32) -> u32 {
            let cnid = self.next_cnid;
            self.next_cnid += 1;
            let mut rec = self.key(name, parent);
            let mut d = vec![0u8; DIR_REC_LEN];
            d[0] = CDR_DIR;
            d[DIR_DIR_ID..DIR_DIR_ID + 4].copy_from_slice(&cnid.to_be_bytes());
            rec.extend_from_slice(&d);
            self.catalog.push(rec);
            cnid
        }

        /// A catalog key: length byte, reserved byte, parent CNID, Pascal name,
        /// padded so the record's data starts on an even offset.
        fn key(&self, name: &str, parent: u32) -> Vec<u8> {
            let mut rec = vec![0u8; 1 + 1 + 4 + 1 + name.len()];
            rec[0] = (rec.len() - 1) as u8;
            rec[2..6].copy_from_slice(&parent.to_be_bytes());
            rec[6] = name.len() as u8;
            rec[7..].copy_from_slice(name.as_bytes());
            while !rec.len().is_multiple_of(2) {
                rec.push(0);
            }
            rec
        }

        /// Write `data` as a file at the volume root.
        fn add_file(&mut self, name: &str, ftype: &[u8; 4], data: &[u8], pieces: usize) {
            self.add_file_in(ROOT_CNID, name, ftype, data, pieces);
        }

        /// Write `data` as a file under `parent`, with Infocom's own creator.
        fn add_file_in(
            &mut self,
            parent: u32,
            name: &str,
            ftype: &[u8; 4],
            data: &[u8],
            pieces: usize,
        ) {
            self.add_entry(parent, name, ftype, b"IN0Z", data, pieces);
        }

        /// Write `data` as a file that Apple's PC Exchange imported from a DOS
        /// volume — the other half of a hybrid disc.
        fn add_dos_file_in(&mut self, parent: u32, name: &str, data: &[u8], pieces: usize) {
            self.add_entry(parent, name, b"TEXT", b"mdos", data, pieces);
        }

        /// Split into `pieces` extents. More than three spills into the extents
        /// overflow file, exactly as HFS does.
        fn add_entry(
            &mut self,
            parent: u32,
            name: &str,
            ftype: &[u8; 4],
            creator: &[u8; 4],
            data: &[u8],
            pieces: usize,
        ) {
            let cnid = self.next_cnid;
            self.next_cnid += 1;
            let blocks = data.len().div_ceil(BLOCK);
            let per = blocks.div_ceil(pieces.max(1));
            let mut extents: Vec<Extent> = Vec::new();
            let mut written = 0usize;
            while written < blocks {
                let run = per.min(blocks - written);
                let start = self.next;
                self.next += run;
                // A gap between runs, so a reader that assumed contiguity fails.
                self.next += 1;
                for i in 0..run {
                    let src = (written + i) * BLOCK;
                    let end = data.len().min(src + BLOCK);
                    let at = (ALLOC_START + start + i) * BLOCK;
                    self.volume[at..at + (end - src)].copy_from_slice(&data[src..end]);
                }
                extents.push((start as u16, run as u16));
                written += run;
            }
            let first = pad_extents(&extents[..extents.len().min(3)]);
            for (i, chunk) in extents[extents.len().min(3)..].chunks(3).enumerate() {
                // Keyed by the fork-relative allocation block the record starts
                // at: three extents of `per` blocks each precede it.
                self.overflow.push((cnid, ((3 + i * 3) * per) as u16, pad_extents(chunk)));
            }

            // The catalog record: key (length, reserved, parent, name) then a
            // file record.
            let mut rec = self.key(name, parent);
            let mut d = vec![0u8; FILE_REC_LEN];
            d[0] = CDR_FILE;
            d[FIL_TYPE..FIL_TYPE + 4].copy_from_slice(ftype);
            d[FIL_CREATOR..FIL_CREATOR + 4].copy_from_slice(creator);
            d[FIL_FL_NUM..FIL_FL_NUM + 4].copy_from_slice(&cnid.to_be_bytes());
            d[FIL_LG_LEN..FIL_LG_LEN + 4].copy_from_slice(&(data.len() as u32).to_be_bytes());
            for (i, (start, count)) in first.iter().enumerate() {
                let at = FIL_EXT_REC + i * 4;
                d[at..at + 2].copy_from_slice(&start.to_be_bytes());
                d[at + 2..at + 4].copy_from_slice(&count.to_be_bytes());
            }
            rec.extend_from_slice(&d);
            self.catalog.push(rec);
        }

        /// One past the last byte of volume any file uses — everything beyond
        /// it is free space, which is exactly what a hybrid disc's partition
        /// does not carry (SQ-0870). Blocks are handed out from the front, so
        /// this is where the next one would go.
        fn used_end(&self) -> usize {
            (ALLOC_START + self.next) * BLOCK
        }

        /// Lay the MDB and both B*-trees down and hand back the volume.
        fn finish(mut self) -> Vec<u8> {
            let catalog = btree(&self.catalog);
            let overflow: Vec<Vec<u8>> = self
                .overflow
                .iter()
                .map(|(cnid, abn, exts)| {
                    let mut r = vec![0u8; XKR_LEN + 1 + 12];
                    r[0] = XKR_LEN as u8;
                    r[1] = FORK_DATA;
                    r[2..6].copy_from_slice(&cnid.to_be_bytes());
                    r[6..8].copy_from_slice(&abn.to_be_bytes());
                    for (i, (start, count)) in exts.iter().enumerate() {
                        let at = XKR_LEN + 1 + i * 4;
                        r[at..at + 2].copy_from_slice(&start.to_be_bytes());
                        r[at + 2..at + 4].copy_from_slice(&count.to_be_bytes());
                    }
                    r
                })
                .collect();
            let extents = btree(&overflow);

            // Trees first, in the blocks `VolumeBuilder::new` reserved: the
            // extents file at 0, the catalog at `TREE_BLOCKS`. Each is declared
            // at its ACTUAL length rather than at the reservation — a fixed
            // `3 * BLOCK` silently truncated the catalog to its first two leaves
            // the moment a volume held more than a floppy's worth of records,
            // and the files past that point simply were not there (SQ-0876).
            assert!(
                extents.len() <= TREE_BLOCKS * BLOCK && catalog.len() <= TREE_BLOCKS * BLOCK,
                "the builder's B*-tree reservation is too small for this fixture"
            );
            for (base, tree) in [(0usize, &extents), (TREE_BLOCKS, &catalog)] {
                let at = (ALLOC_START + base) * BLOCK;
                self.volume[at..at + tree.len()].copy_from_slice(tree);
            }

            let mdb = MDB_OFFSET;
            self.put16(mdb, HFS_SIGNATURE);
            self.put16(mdb + MDB_NM_AL_BLKS, (VOLUME_BLOCKS - ALLOC_START) as u16);
            self.put32(mdb + MDB_AL_BLK_SIZ, BLOCK as u32);
            self.put16(mdb + MDB_AL_BL_ST, ALLOC_START as u16);
            let name = b"Test Disk";
            self.volume[mdb + MDB_VN] = name.len() as u8;
            self.volume[mdb + MDB_VN + 1..mdb + MDB_VN + 1 + name.len()].copy_from_slice(name);
            self.put32(mdb + MDB_XT_FL_SIZE, extents.len() as u32);
            self.put16(mdb + MDB_XT_EXT_REC, 0);
            self.put16(mdb + MDB_XT_EXT_REC + 2, TREE_BLOCKS as u16);
            self.put32(mdb + MDB_CT_FL_SIZE, catalog.len() as u32);
            self.put16(mdb + MDB_CT_EXT_REC, TREE_BLOCKS as u16);
            self.put16(mdb + MDB_CT_EXT_REC + 2, TREE_BLOCKS as u16);
            self.volume
        }
    }

    /// One synthetic Macintosh floppy carrying `files`, DiskCopy wrapper and
    /// all, for the mount-seam tests in [`crate::medium`]. They need a real
    /// volume of every format and cannot reach a builder that is private to
    /// this module.
    pub(crate) fn sample_disk(files: &[(&str, &[u8])]) -> Vec<u8> {
        diskcopy(&sample_volume(files))
    }

    /// The same volume with no wrapper at all — what a partition holds, and
    /// therefore what [`crate::cd`]'s tests lay out inside a disc.
    pub(crate) fn sample_volume(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = VolumeBuilder::new();
        for (name, data) in files {
            b.add_file(name, b"INdf", data, 1);
        }
        b.finish()
    }


    fn pad_extents(exts: &[Extent]) -> ExtentRecord {
        let mut out: ExtentRecord = [(0, 0); 3];
        for (i, e) in exts.iter().take(3).enumerate() {
            out[i] = *e;
        }
        out
    }

    /// A header node naming one leaf, then that leaf. Three blocks, which is
    /// what [`VolumeBuilder`] reserves for each tree.
    fn btree(records: &[Vec<u8>]) -> Vec<u8> {
        let mut tree = vec![0u8; 2 * NODE_SIZE];
        // Header node: one record, whose `bthFNode` is node 1.
        tree[8] = 1; // ndType: header
        tree[ND_N_RECS..ND_N_RECS + 2].copy_from_slice(&1u16.to_be_bytes());
        tree[NODE_SIZE - 2..NODE_SIZE].copy_from_slice(&(NODE_HEADER as u16).to_be_bytes());
        let header_end = NODE_HEADER as u16 + 106;
        tree[NODE_SIZE - 4..NODE_SIZE - 2].copy_from_slice(&header_end.to_be_bytes());
        tree[NODE_HEADER + BTH_F_NODE..NODE_HEADER + BTH_F_NODE + 4]
            .copy_from_slice(&1u32.to_be_bytes());

        // Pack the records into leaves, greedily, the way a real catalog does —
        // a node holds what fits between its header and its offset table, and
        // the rest go in the next leaf. A single-leaf builder cannot exercise
        // the `ndFLink` chain [`leaf_records`] walks, and a folder record per
        // game overflows one node the moment a volume holds more than a floppy's
        // worth (SQ-0877).
        let mut leaves: Vec<Vec<&Vec<u8>>> = vec![Vec::new()];
        let mut used = NODE_HEADER;
        for r in records {
            // Each record costs its own bytes plus its offset, and one more
            // offset marks the free space after the last.
            let n = leaves.last().expect("one leaf always").len();
            if !leaves.last().expect("one leaf always").is_empty()
                && used + r.len() + 2 * (n + 2) > NODE_SIZE
            {
                leaves.push(Vec::new());
                used = NODE_HEADER;
            }
            used += r.len();
            leaves.last_mut().expect("one leaf always").push(r);
        }

        // `ndType` is -1 for a leaf; `ndFLink` names the next, and 0 ends the
        // chain.
        tree.resize((1 + leaves.len()) * NODE_SIZE, 0);
        for (l, recs) in leaves.iter().enumerate() {
            let leaf = (1 + l) * NODE_SIZE;
            tree[leaf + 8] = 0xFF;
            let next = if l + 1 < leaves.len() { (l + 2) as u32 } else { 0 };
            tree[leaf..leaf + 4].copy_from_slice(&next.to_be_bytes());
            tree[leaf + ND_N_RECS..leaf + ND_N_RECS + 2]
                .copy_from_slice(&(recs.len() as u16).to_be_bytes());
            let mut at = NODE_HEADER;
            for (i, r) in recs.iter().enumerate() {
                tree[leaf + NODE_SIZE - 2 * (i + 1)..leaf + NODE_SIZE - 2 * i]
                    .copy_from_slice(&(at as u16).to_be_bytes());
                tree[leaf + at..leaf + at + r.len()].copy_from_slice(r);
                at += r.len();
            }
            let n = recs.len();
            tree[leaf + NODE_SIZE - 2 * (n + 1)..leaf + NODE_SIZE - 2 * n]
                .copy_from_slice(&(at as u16).to_be_bytes());
        }
        tree
    }

    /// Wrap a volume the way DiskCopy 4.2 does, tags and all.
    fn diskcopy(volume: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; DISKCOPY_HEADER];
        let name = b"Test Disk";
        out[0] = name.len() as u8;
        out[1..1 + name.len()].copy_from_slice(name);
        let tags = 12 * (volume.len() / BLOCK);
        out[DISKCOPY_DATA_SIZE..DISKCOPY_DATA_SIZE + 4]
            .copy_from_slice(&(volume.len() as u32).to_be_bytes());
        out[DISKCOPY_TAG_SIZE..DISKCOPY_TAG_SIZE + 4].copy_from_slice(&(tags as u32).to_be_bytes());
        out[0x50] = 0x01;
        out[0x51] = 0x22;
        let magic = DISKCOPY_MAGIC.to_be_bytes();
        out[DISKCOPY_MAGIC_OFF..DISKCOPY_MAGIC_OFF + 2].copy_from_slice(&magic);
        out.extend_from_slice(volume);
        out.extend(std::iter::repeat_n(0xA5u8, tags));
        out
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
        b[0x12..0x18].copy_from_slice(b"881019");
        b
    }

    #[test]
    fn rejects_images_that_are_not_macintosh_volumes() {
        assert!(!Hfs::looks_like_hfs(b"not a disk"));
        assert!(!Hfs::looks_like_hfs(&vec![0u8; VOLUME_BLOCKS * BLOCK]), "no BD signature");
        // An MFS volume — the flat filesystem HFS replaced — says so at the same
        // offset, and nothing here would read its directory.
        let mut mfs = vec![0u8; VOLUME_BLOCKS * BLOCK];
        mfs[MDB_OFFSET..MDB_OFFSET + 2].copy_from_slice(&0xD2D7u16.to_be_bytes());
        assert!(!Hfs::looks_like_hfs(&mfs));
        assert_eq!(Hfs::mount(vec![0u8; 16]).unwrap_err(), HfsError::NotHfs);
    }

    /// The tags are 12 bytes per sector and they follow the volume. A reader
    /// that treats the whole file as the volume, or that misplaces the 84-byte
    /// header, gets a volume whose geometry does not fit — which is exactly what
    /// [`volume_is_sane`] tests, so both mistakes are caught here.
    #[test]
    fn reads_a_volume_bare_and_inside_a_diskcopy_wrapper() {
        let mut b = VolumeBuilder::new();
        b.add_file("Story.data", b"INdf", &fake_story(4096), 1);
        let volume = b.finish();
        let wrapped = diskcopy(&volume);
        assert!(wrapped.len() > volume.len() + DISKCOPY_HEADER, "the tags are there too");

        for (what, image) in [("bare", volume.clone()), ("wrapped", wrapped)] {
            assert!(Hfs::looks_like_hfs(&image), "{what}");
            let hfs = Hfs::mount(image).unwrap_or_else(|e| panic!("{what}: {e:?}"));
            assert_eq!(hfs.volume_name(), "Test Disk", "{what}");
            assert_eq!(hfs.files().len(), 1, "{what}");
            assert_eq!(hfs.story().expect("a story").0, "Story.data", "{what}");
        }
    }

    /// A fork with more than three extents keeps the rest in the extents
    /// overflow file. Nothing on the Zork Zero disk needs it — every file there
    /// is one extent — so this is the case only a synthetic volume can pin.
    #[test]
    fn reads_a_fork_that_spills_into_the_extents_overflow_file() {
        let mut b = VolumeBuilder::new();
        let contiguous: Vec<u8> = (0..40 * BLOCK).map(|i| (i % 251) as u8).collect();
        let fragmented: Vec<u8> = (0..60 * BLOCK).map(|i| (i % 241) as u8).collect();
        b.add_file("Whole", b"INdf", &contiguous, 1);
        b.add_file("Shattered", b"INdf", &fragmented, 6);
        let hfs = Hfs::mount(b.finish()).expect("mounts");
        assert_eq!(hfs.read_named("whole").as_deref(), Some(&contiguous[..]));
        assert_eq!(
            hfs.read_named("Shattered").as_deref(),
            Some(&fragmented[..]),
            "six extents: three in the catalog record, three in the overflow file"
        );
        assert_eq!(hfs.read_named("absent"), None);
    }

    #[test]
    fn finds_a_story_by_content_not_by_name() {
        let mut b = VolumeBuilder::new();
        b.add_file("Zork Zero", b"APPL", b"Joy!peffpwpc application", 1);
        b.add_file("Bootleg", b"INdf", &fake_story(4096), 1);
        let hfs = Hfs::mount(b.finish()).expect("mounts");
        let (name, bytes) = hfs.story().expect("the story is found under any name");
        assert_eq!(name, "Bootleg");
        assert_eq!(bytes.len(), 4096);
    }

    #[test]
    fn a_disk_with_no_story_says_so() {
        let mut b = VolumeBuilder::new();
        b.add_file("Desktop", b"FNDR", &vec![0x11u8; 1665], 1);
        b.add_file("System", b"ZSYS", &vec![0x22u8; 8000], 1);
        let hfs = Hfs::mount(b.finish()).expect("mounts");
        assert_eq!(hfs.story(), None);
        assert!(hfs.pictures().is_none());
    }

    /// A one-picture Infocom archive, colour or monochrome — the smallest thing
    /// [`InfocomPics::parse`] accepts, so that a synthetic volume can carry the
    /// pair a Macintosh release ships. 4x2, rows `2222` then `3333`.
    fn fake_pics(mono: bool) -> Vec<u8> {
        let entry = if mono { 12 } else { 14 };
        let mut f = vec![0u8; 16];
        f[0] = 1; // part
        f[1] = if mono { 0x0e } else { 0x06 };
        let huff = 16 + entry;
        f[2..4].copy_from_slice(&((huff / 2) as u16).to_be_bytes());
        f[5] = 1; // one picture
        f[8] = entry as u8;
        let data = huff + 256;
        f.extend_from_slice(&[0, 1, 0, 4, 0, 2]); // id 1, 4x2
        f.extend_from_slice(&[0, if mono { 0x0a } else { 0x02 }]); // eFlags
        f.extend_from_slice(&[(data >> 16) as u8, (data >> 8) as u8, data as u8]);
        f.resize(16 + entry, 0); // pad byte, or a zero palette offset
        let mut tree = vec![0u8; 256];
        tree[0] = 128 + 2; // `0`  -> colour 2
        tree[1] = 1; // `1`  -> node 1
        tree[2] = 128 + 1; // `10` -> colour 1
        tree[3] = 128 + 18; // `11` -> repeat 3 more
        f.extend_from_slice(&tree);
        f.extend_from_slice(&[0, 0, 1, 0, 0, 4, 0b0111_0110]);
        f
    }

    /// SQ-0838: a Macintosh disk carries two archives and **colour wins**, even
    /// when the monochrome one holds every other advantage.
    ///
    /// Stacked deliberately against the rule: the monochrome archive here is the
    /// one wearing a conventional Infocom name, and the colour one is called
    /// something no tiebreak favours and sorts last in the catalog besides. Depth
    /// is asked first, so it still loses. Reading two-colour art to a terminal
    /// that has sixteen million of them is a preference, and it is the user's to
    /// state — by naming the archive — not the disk's to imply.
    #[test]
    fn a_disk_with_both_archives_offers_the_colour_one() {
        let mut b = VolumeBuilder::new();
        b.add_file("Pic.data", b"INdf", &fake_pics(true), 1);
        b.add_file("Story.data", b"INdf", &fake_story(4096), 1);
        b.add_file("ZArt", b"INdf", &fake_pics(false), 1);
        let hfs = Hfs::mount(b.finish()).expect("mounts");

        let (name, pics) = hfs.pictures().expect("an archive is found");
        assert_eq!(name, "ZArt", "depth beats both the conventional name and catalog order");
        assert!(!pics.is_monochrome());

        // Both are readable; the other one is reached by name.
        let mono = InfocomPics::parse(hfs.read_named("Pic.data").expect("present")).expect("parses");
        assert!(mono.is_monochrome());
        assert_eq!(mono.decode(1).unwrap().indices, vec![2, 2, 2, 2, 3, 3, 3, 3]);

        // With only the monochrome archive on the disk it is of course the art —
        // preferring colour is not refusing monochrome.
        let mut b = VolumeBuilder::new();
        b.add_file("Pic.data", b"INdf", &fake_pics(true), 1);
        b.add_file("Story.data", b"INdf", &fake_story(4096), 1);
        let hfs = Hfs::mount(b.finish()).expect("mounts");
        let (name, pics) = hfs.pictures().expect("the only archive is found");
        assert_eq!(name, "Pic.data");
        assert!(pics.is_monochrome());
    }

    #[test]
    fn the_conventional_name_only_breaks_a_tie() {
        let mut b = VolumeBuilder::new();
        b.add_file("Backup.data", b"INdf", &fake_story(8192), 1);
        b.add_file("Story.data", b"INdf", &fake_story(4096), 1);
        let hfs = Hfs::mount(b.finish()).expect("mounts");
        assert_eq!(hfs.story().expect("a story").0, "Story.data", "convention beats size");

        let mut b = VolumeBuilder::new();
        b.add_file("Alpha", b"INdf", &fake_story(4096), 1);
        b.add_file("Beta", b"INdf", &fake_story(8192), 1);
        let hfs = Hfs::mount(b.finish()).expect("mounts");
        assert_eq!(hfs.story().expect("a story").0, "Beta", "no convention → the largest");
    }

    /// The catalog names types and sizes, which is what lets a listing say what
    /// a Macintosh file *is* — an application has no data fork at all.
    #[test]
    fn a_listing_reports_type_creator_and_both_forks() {
        let mut b = VolumeBuilder::new();
        b.add_file("Story.data", b"INdf", &fake_story(4096), 1);
        b.add_file("Zork Zero", b"APPL", b"", 1);
        let hfs = Hfs::mount(b.finish()).expect("mounts");
        let story = &hfs.files()[0];
        assert_eq!(&story.file_type, b"INdf");
        assert_eq!(&story.creator, b"IN0Z");
        assert_eq!(story.size, 4096);
        assert_eq!(hfs.files()[1].size, 0, "an application is all resource fork");
    }

    /// **A volume that does not fill its container mounts** (SQ-0870).
    ///
    /// The MDB describes a 800 KB disk; what is here stops just past the last
    /// block any file uses, which is the shape of every Apple_HFS partition on a
    /// hybrid CD — the map sizes the partition for the medium and the free tail
    /// was never written. Nothing a reader follows is missing, so nothing is
    /// wrong.
    ///
    /// FALSIFICATION: restore the old bound in [`volume_is_sane`] —
    /// `start * BLOCK + count * alloc_size <= volume.len()` — and this fails at
    /// the `looks_like_hfs` assertion, with the volume declined and its story
    /// unreachable, which is the reported symptom exactly.
    #[test]
    fn a_volume_whose_free_tail_is_absent_still_mounts() {
        let story = fake_story(4096);
        let mut b = VolumeBuilder::new();
        b.add_file("Story.data", b"INdf", &story, 1);
        b.add_file("Pic.data", b"INdf", &fake_pics(false), 1);
        let used = b.used_end();
        let whole = b.finish();
        let trimmed = whole[..used].to_vec();
        assert!(trimmed.len() < whole.len() / 4, "most of the volume is free space and absent");

        assert!(Hfs::looks_like_hfs(&trimmed), "a volume shorter than its own geometry claims");
        let hfs = Hfs::mount(trimmed).expect("it mounts");
        assert_eq!(hfs.volume_name(), "Test Disk");
        assert_eq!(hfs.files().len(), 2);
        assert_eq!(hfs.story().expect("and the story reads whole").1, story);
        assert!(hfs.pictures().is_some(), "and so does the artwork");
    }

    /// **…and a volume that is genuinely truncated is still refused** — the
    /// guard the fix above must not break (SQ-0870).
    ///
    /// The distinction is what is MISSING. Free space nobody follows costs
    /// nothing; a file's own extents running past the end of the image is
    /// damage, and the answer to it is no story rather than the front half of
    /// one. Cut the catalog itself off and the volume does not open at all.
    #[test]
    fn a_truncated_volume_yields_no_story_rather_than_half_of_one() {
        let story = fake_story(8 * BLOCK);
        let mut b = VolumeBuilder::new();
        b.add_file("Story.data", b"INdf", &story, 1);
        let used = b.used_end();
        let whole = b.finish();

        // Cut through the story's data: its catalog record still promises 4096
        // bytes and the blocks holding the tail of them are gone.
        let cut = whole[..used - 3 * BLOCK].to_vec();
        let hfs = Hfs::mount(cut).expect("the catalog is intact, so the volume opens");
        assert_eq!(hfs.files().len(), 1, "the file is still catalogued");
        assert_eq!(hfs.read_named("Story.data"), None, "but it does not read short");
        assert_eq!(hfs.story(), None, "so the disk offers no story at all");
        assert_eq!(crate::medium::Volume::stories(&hfs).len(), 0);

        // Cut the catalog off instead and there is nothing to mount.
        let gutted = whole[..(ALLOC_START + 4) * BLOCK].to_vec();
        assert!(!Hfs::looks_like_hfs(&gutted), "a volume with no catalogue is not one");
        assert_eq!(Hfs::mount(gutted).unwrap_err(), HfsError::NotHfs);
    }

    /// **A folder is reported, and is what tells two identical names apart**
    /// (SQ-0877).
    ///
    /// Synthetic, so it runs on CI, and shaped like the case that motivated it:
    /// two games each shipping a `Story.data`, plus one at the root. Before this
    /// quest all three answered to the name `Story.data` and `read_named` could
    /// only ever reach the first.
    ///
    /// FALSIFICATION: report `e.name` instead of `e.path()` and the three paths
    /// collapse to one repeated name; drop the parent walk and every path loses
    /// its folder.
    #[test]
    fn a_folder_is_reported_and_tells_two_identical_names_apart() {
        let mut b = VolumeBuilder::new();
        let arthur = b.add_folder("ARTHUR FOLDER", ROOT_CNID);
        let journey = b.add_folder("JOURNEY FOLDER", ROOT_CNID);
        let deep = b.add_folder("DATA", journey);
        b.add_file("Loose.data", b"INdf", b"at the volume root", 1);
        b.add_file_in(arthur, "Story.data", b"INdf", b"arthur's story", 1);
        b.add_file_in(journey, "Story.data", b"INdf", b"journey's story", 1);
        b.add_file_in(deep, "Story.data", b"INdf", b"journey's spare", 1);
        let hfs = Hfs::mount(b.finish()).expect("mounts");

        let mut paths: Vec<String> = hfs.files().iter().map(|e| e.path()).collect();
        paths.sort();
        assert_eq!(
            paths,
            [
                "ARTHUR FOLDER/Story.data",
                "JOURNEY FOLDER/DATA/Story.data",
                "JOURNEY FOLDER/Story.data",
                "Loose.data",
            ],
            "the folder chain, outermost first; a root file keeps its bare name"
        );

        // The path reaches a PARTICULAR one; the bare name still reaches some
        // one of them, which is what keeps `--pictures Pic.data` working.
        assert_eq!(
            hfs.read_named("ARTHUR FOLDER/Story.data").as_deref(),
            Some(&b"arthur's story"[..])
        );
        assert_eq!(
            hfs.read_named("journey folder/data/STORY.DATA").as_deref(),
            Some(&b"journey's spare"[..]),
            "case-insensitive over the whole path"
        );
        assert!(hfs.read_named("Story.data").is_some(), "a bare name still resolves");
        assert_eq!(hfs.read_named("Loose.data").as_deref(), Some(&b"at the volume root"[..]));
    }

    /// **Each game's artwork is its own, and a DOS import is not a Macintosh**
    /// (SQ-0876).
    ///
    /// Synthetic, so it runs on CI, and shaped like the disc that motivated it:
    /// two graphical games in two folders, plus the same game again on the DOS
    /// side of the hybrid, plus a game with no artwork at all.
    ///
    /// FALSIFICATION: return `Hfs::pictures(self)` from `pictures_beside` and
    /// all three stories get Arthur's archive — the reported symptom; make
    /// `is_from_dos` always false and the DOS build claims the Macintosh.
    #[test]
    fn each_folder_pairs_its_own_artwork_and_a_dos_import_is_not_a_macintosh() {
        let mut b = VolumeBuilder::new();
        let mac = b.add_folder("MAC", ROOT_CNID);
        let arthur = b.add_folder("ARTHUR FOLDER", mac);
        let journey = b.add_folder("JOURNEY FOLDER", mac);
        let pc = b.add_folder("PC", ROOT_CNID);
        let pc_arthur = b.add_folder("ARTHUR", pc);

        b.add_file_in(arthur, "Story.data", b"INdf", &fake_story(4096), 1);
        b.add_file_in(arthur, "CPic.data", b"INdf", &fake_pics(false), 1);
        b.add_file_in(journey, "Story.data", b"INdf", &fake_story(2048), 1);
        b.add_file_in(journey, "CPic.data", b"INdf", &fake_pics(false), 1);
        b.add_file_in(mac, "ZORK I", b"APPL", &fake_story(1024), 1);
        b.add_dos_file_in(pc_arthur, "ARTHUR.ZIP", &fake_story(3072), 1);
        b.add_dos_file_in(pc_arthur, "ARTHUR.MG1", &fake_pics(false), 1);
        let raw = b.finish();

        let hfs = Hfs::mount(raw.clone()).expect("mounts");
        let named = |p: &str| hfs.pictures_beside(p).map(|(n, _)| n);
        assert_eq!(
            named("MAC/ARTHUR FOLDER/Story.data").as_deref(),
            Some("MAC/ARTHUR FOLDER/CPic.data"),
            "Arthur's own archive, not the first on the platter"
        );
        assert_eq!(
            named("MAC/JOURNEY FOLDER/Story.data").as_deref(),
            Some("MAC/JOURNEY FOLDER/CPic.data"),
            "Journey's own — the case that drew Arthur's plates"
        );
        assert_eq!(named("MAC/ZORK I"), None, "a game with no artwork has none, not another's");
        assert_eq!(
            named("PC/ARTHUR/ARTHUR.ZIP").as_deref(),
            Some("PC/ARTHUR/ARTHUR.MG1"),
            "the DOS build pairs with the DOS artwork in its own folder"
        );

        // The machine, per file, through the format-neutral door.
        let disk = crate::medium::MountedDisk::mount(raw).expect("mounts as a disk");
        assert_eq!(disk.image_for("MAC/ARTHUR FOLDER/Story.data"), crate::medium::DiskImage::Hfs);
        assert_eq!(
            disk.interpreter_number_for("MAC/ARTHUR FOLDER/Story.data"),
            Some(crate::medium::MACINTOSH_INTERPRETER_NUMBER),
            "ZMSD §11.1.3: 3 = Macintosh"
        );
        assert_eq!(
            disk.image_for("PC/ARTHUR/ARTHUR.ZIP"),
            crate::medium::DiskImage::Fat12Dos,
            "a DOS import wears the DOS row, on a Macintosh filesystem"
        );
        assert_eq!(
            disk.interpreter_number_for("PC/ARTHUR/ARTHUR.ZIP"),
            None,
            "the IBM PC's answer: leave the version-dependent rule in force"
        );
        // A name this volume does not hold gets no opinion, and falls through.
        assert_eq!(disk.image_for("nothing/here"), crate::medium::DiskImage::Hfs);
        assert!(disk.pictures_for("nothing/here").is_some(), "falls through to the whole volume");
    }

    /// **The hybrid CD, read where it lies** (SQ-0870): a raw MODE1/2352 dump
    /// whose third Apple partition is the Macintosh volume.
    ///
    /// The property is that reading the disc gives *exactly* what dd'ing the
    /// partition out by hand gives — same catalogue, same stories — so the
    /// extraction step this quest removes was never adding anything.
    ///
    /// FALSIFICATION: break the raw-sector unwrap (return `Sectors::Cooked`
    /// unconditionally from `Sectors::of`) and the `.bin` stops mounting.
    ///
    /// Outside the repo, so it skips vacuously; CI has no `masterpieces/`.
    #[test]
    fn real_masterpieces_cd_reads_the_same_volume_as_its_extracted_partition() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../masterpieces");
        let media = [
            ("the raw CD", "Classic Text Adventure Masterpieces of Infocom (USA).bin"),
            ("the partition, extracted by hand", "Masterpieces-HFS.img"),
        ];
        let mut listings: Vec<(String, Vec<(String, usize)>)> = Vec::new();
        for (what, file) in media {
            let Ok(bytes) = std::fs::read(dir.join(file)) else {
                eprintln!("SKIP: {what} is absent at {}", dir.join(file).display());
                continue;
            };
            assert!(Hfs::looks_like_hfs(&bytes), "{what} is a Macintosh volume");
            let hfs = Hfs::mount(bytes).unwrap_or_else(|e| panic!("{what}: {e:?}"));
            assert_eq!(hfs.volume_name(), "Masterpieces", "{what}");
            assert_eq!(hfs.files().len(), 770, "{what}: the whole catalogue");

            // The Macintosh build of Zork I, which is not the `ZORK1.DAT` the
            // PC side of the same disc carries: same release, different file.
            let stories = crate::medium::Volume::stories(&hfs);
            assert_eq!(stories.len(), 83, "{what}");
            let zork = stories
                .iter()
                .find(|s| s.name == "MAC/ZORK I")
                .unwrap_or_else(|| panic!("{what}: the Macintosh Zork I"));
            let release = u16::from_be_bytes([zork.bytes[2], zork.bytes[3]]);
            assert_eq!(zork.bytes[0], 3, "{what}: v3");
            assert_eq!(release, 88, "{what}: release 88");
            assert_eq!(&zork.bytes[0x12..0x18], b"840726", "{what}");
            assert_eq!(zork.bytes.len(), 84_992, "{what}");
            // …and the Macintosh *Journey* the note on this quest points at:
            // release 26, serial 890316, which no other medium here carries
            // except `stories/InfocomMasterpieces.img`.
            assert!(
                stories.iter().any(|s| s.bytes[0] == 6 && &s.bytes[0x12..0x18] == b"890316"),
                "{what}: the r26 Macintosh Journey"
            );
            // SQ-0876, on the real disc: the two halves separate exactly, and
            // each graphical game pairs with its OWN archive. Before this, all
            // six resolved to `MAC/ZORK ZERO/CPIC.DATA` and all 83 stories
            // claimed the Macintosh.
            let disk = crate::medium::MountedDisk::mount(std::fs::read(dir.join(file)).unwrap())
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            let mac = stories.iter().filter(|s| s.name.starts_with("MAC/")).count();
            let pc = stories.iter().filter(|s| s.name.starts_with("PC/")).count();
            assert_eq!((mac, pc), (33, 50), "{what}: every story is on one side or the other");
            for s in &stories {
                let want = if s.name.starts_with("PC/") {
                    crate::medium::DiskImage::Fat12Dos
                } else {
                    crate::medium::DiskImage::Hfs
                };
                assert_eq!(disk.image_for(&s.name), want, "{what}: {}", s.name);
            }
            for (story, art) in [
                ("MAC/ARTHUR FOLDER/STORY.DATA", Some("MAC/ARTHUR FOLDER/CPIC.DATA")),
                ("MAC/JOURNEY FOLDER/STORY.DATA", Some("MAC/JOURNEY FOLDER/CPIC.DATA")),
                ("MAC/ZORK ZERO/STORY.DATA", Some("MAC/ZORK ZERO/CPIC.DATA")),
                ("PC/ARTHUR/ARTHUR.ZIP", Some("PC/ARTHUR/ARTHUR.MG1")),
                ("PC/JOURNEY/JOURNEY.ZIP", Some("PC/JOURNEY/JOURNEY.MG1")),
                ("PC/ZORK0/ZORK0.ZIP", Some("PC/ZORK0/ZORK0.EG1")),
                // A text game shipped with no artwork gets none, not another
                // game's — the failure mode the whole-volume answer had.
                ("MAC/ZORK I", None),
                ("PC/ZORK1/DATA/ZORK1.DAT", None),
            ] {
                assert_eq!(
                    disk.pictures_for(story).map(|a| a.name).as_deref(),
                    art,
                    "{what}: the artwork paired with {story}"
                );
            }

            listings.push((
                what.to_string(),
                hfs.files().iter().map(|e| (e.path(), e.size)).collect(),
            ));
        }
        if let [(a, one), (b, two)] = &listings[..] {
            assert_eq!(one, two, "{a} and {b} are the same volume");
        }
        // Each medium that is PRESENT must read; a medium that is absent is a
        // skip. The equality above is the interesting property and it needs
        // both, but requiring both to exist would make this test hostage to a
        // 354 MB derived artifact — and the hand-extracted partition is exactly
        // the artifact SQ-0870 made unnecessary, so it is the one a tidy-up
        // deletes first. Deleting it should not turn this red.
        let present = media.iter().filter(|(_, f)| dir.join(f).is_file()).count();
        assert_eq!(listings.len(), present, "every medium that is here has to read");
    }

    /// **The intact control** (SQ-0870): the 12 MB Macintosh compilation in
    /// `stories/`, which fills its container and always mounted, still mounts
    /// and still offers all 33 games.
    ///
    /// This is the volume the relaxed bound must not change. It is the same
    /// collection as the CD's Macintosh partition at a twentieth of the size —
    /// including the r26/s890316 *Journey* — so a regression that made the new
    /// medium work by loosening something the old one relied on shows up here.
    #[test]
    fn the_intact_macintosh_compilation_still_offers_its_thirty_three_stories() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/InfocomMasterpieces.img");
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("SKIP: the Macintosh compilation is absent at {}", path.display());
            return;
        };
        assert!(Hfs::looks_like_hfs(&bytes));
        let hfs = Hfs::mount(bytes).expect("it mounts");
        let stories = crate::medium::Volume::stories(&hfs);
        assert_eq!(stories.len(), 33, "the whole shelf");
        assert!(
            stories.iter().any(|s| s.bytes[0] == 6 && &s.bytes[0x12..0x18] == b"890316"),
            "including the r26 Macintosh Journey"
        );
    }

    /// Real media: the user's Macintosh Zork Zero, if they have it. It lives
    /// outside the repo, so this skips vacuously.
    #[test]
    fn real_macintosh_zork_zero_disk() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/Zork Zero Disk.image");
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("SKIP: original Macintosh media absent at {}", path.display());
            return;
        };
        assert!(Hfs::looks_like_hfs(&bytes));
        let hfs = Hfs::mount(bytes).expect("the disk mounts");
        assert_eq!(hfs.volume_name(), "Zork Zero Disk");

        // The whole catalog, which is what says whether the disk carries art.
        let listing: Vec<(String, usize, usize)> = hfs
            .files()
            .iter()
            .map(|e| (e.name.clone(), e.size, e.resource_size))
            .collect();
        assert_eq!(
            listing,
            vec![
                ("CPic.data".to_string(), 218_624, 0),
                ("Desktop".to_string(), 0, 1_665),
                ("Pic.data".to_string(), 239_104, 0),
                ("Story.data".to_string(), 295_936, 0),
                ("Zork Zero".to_string(), 0, 38_833),
            ],
            "five files, and two of them are picture archives"
        );

        let (name, story) = hfs.story().expect("Story.data is found");
        assert_eq!(name, "Story.data");
        assert_eq!(story.len(), 295_936);
        assert_eq!(story[0], 6, "Zork Zero is v6");
        assert_eq!(u16::from_be_bytes([story[2], story[3]]), 296, "release 296, not the PC's 393");
        assert_eq!(&story[0x12..0x18], b"881019");

        // Both archives read (SQ-0838), and the automatic choice is the COLOUR
        // one — a preference now, not a parse failure.
        let (pname, pics) = hfs.pictures().expect("the colour archive is found");
        assert_eq!(pname, "CPic.data");
        assert_eq!(pics.entries().len(), 483);
        assert!(!pics.is_monochrome(), "the disk's default art is its colour art");
        assert!(pics.decode(1).is_ok(), "picture 1 decodes straight off the disk");

        let raw = hfs.read_named("Pic.data").expect("the monochrome archive is there");
        assert_eq!(raw[1], 0x0e, "its header flags are bocfel's monochrome-Macintosh 0x0e");
        assert_eq!(raw[8], 12, "…and its directory records are 12 bytes, not 14");
        let mono = InfocomPics::parse(raw).expect("and it parses");
        assert!(mono.is_monochrome());
        assert_eq!(mono.entries().len(), 483, "the same catalogue as the colour archive");
        assert_eq!(mono.decode(1).unwrap().indices.len(), 480 * 300);
    }
}

