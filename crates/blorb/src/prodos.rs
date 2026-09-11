//! Read an Apple II ProDOS disk image — a 2IMG-wrapped or bare ProDOS volume —
//! well enough to pull a story file straight off the original release media,
//! with no extraction step (SQ-0836).
//!
//! This is the Apple sibling of [`crate::adf`], [`crate::hfs`] and
//! [`crate::fat12`], and it exists for the same reason: a floppy is a container
//! lanthorn can open rather than something a person has to unpack first.
//! Everything below is measured against the **ten** ProDOS images in the user's
//! corpus — `Arthur Quest 4 Excalibur.2mg`, `Journey.2mg`,
//! `Beyond Zork (1988)(Infocom).2mg` and the seven volumes of
//! `Lost Treasures of Infocom, The (1993)(Big Red Computer Club)`. All ten are
//! 800 KB (1600 blocks) 3.5" ProDOS volumes inside a 64-byte 2IMG header, all
//! ten name `WOOF` (CiderPress) as their creator, and all ten declare image
//! format 1, ProDOS block order.
//!
//! # Layer 1 — the 2IMG wrapper
//!
//! A 64-byte little-endian header, then the disk data, then optional comment and
//! creator chunks:
//!
//! | offset | field |
//! |---|---|
//! | `+$00` | `2IMG`, the magic |
//! | `+$04` | creator signature (`WOOF` for every image here) |
//! | `+$08` | header length, `u16` — 64 |
//! | `+$0a` | file-format version, `u16` |
//! | `+$0c` | image data format, `u32` — 0 DOS order, **1 ProDOS order**, 2 nibbles |
//! | `+$14` | 512-byte blocks, `u32` — meaningful when the format is 1 |
//! | `+$18` | offset from the start of the file to the data, `u32` |
//! | `+$1c` | length of the data in bytes, `u32` |
//!
//! **The length field reads zero on every image in this corpus**, which is a
//! known quirk of exactly the tool that wrote them: CiderPress2's format notes
//! say *"some images created by `WOOF` have a meaningful block count but a zero
//! data length"*, and prescribe the fallback this reader uses — for a ProDOS
//! image *"the data length will be equal to the number of 512-byte blocks *
//! 512"*. So `volume_at` takes the declared length when there is one, then the
//! block count, then whatever follows the header; whichever it lands on has to
//! be a whole number of blocks and has to be present in the bytes in hand.
//!
//! A **bare** ProDOS volume with no wrapper at all is read just the same, by
//! trying both placements and letting the volume directory decide — the same
//! shape [`crate::hfs`] uses for its DiskCopy wrapper. That path was pinned by a
//! synthetic volume until SQ-0863, when the corpus grew three real ones:
//! `Arthur.po`, `Journey.po` and `ZorkZero.po`, the 3.5-inch consolidated
//! pressings of the three graphical Version 6 games, each 819,200 bytes of
//! volume with no header in front of it (`ARTHUR.3.5`, `JOURNEY.3.5`,
//! `ZORK.ZERO.3.5`). They opened the day they arrived, and `.po` is claimed in
//! [`crate::medium`]'s extension census now that a medium wears it.
//!
//! # Layer 1a — the DiskCopy 4.2 wrapper (SQ-0889)
//!
//! **`Shogun.po` is a fourth spelling, and it used to be declined.** Despite the
//! extension it is not a bare volume: it is a DiskCopy 4.2 image — 838,484
//! bytes, `06 "SHOGUN"`, magic `0100` at `+$52` — and this module said for two
//! quests that not unwrapping it was the point, because the Apple artwork was
//! reachable off `shogun_s1.dsk`…`s5` anyway. That was a refusal to open a file
//! that is perfectly readable, and it is gone.
//!
//! The wrapper states its own geometry and the arithmetic closes to the byte:
//! `dataSize` at `+$40` is `$000C8000` = 819,200 — the same 800 KB as the three
//! bare `.po` volumes — `tagSize` at `+$44` is `$00004B00` = 19,200, and
//! 84 + 819,200 + 19,200 = 838,484. So an intact 800K ProDOS volume sits 84
//! bytes in, and its volume directory header at file offset 1108 (= 84 + 1024)
//! reads `0000 0300 f6 "SHOGUN     "` — storage type `$F`, name length 6 —
//! structurally identical to `Journey.po`'s `0000 0300 fb "JOURNEY.3.5"` at
//! offset 1024. The sector tags follow the volume and are not part of it, so
//! nothing here ever addresses them.
//!
//! The unwrap itself is **[`crate::hfs`]'s**, shared rather than rewritten:
//! `diskcopy_volume_len` reads the wrapper's declared geometry and knows
//! nothing about which filesystem is inside, so each reader runs its own volume
//! sniff at the offset and declines what is not its own. That is the "small
//! refactor nobody has done" this paragraph used to describe as optional, and
//! it is one placement in `volume_at` beside the `2IMG` one. A Macintosh
//! DiskCopy image is unwrapped here just as willingly and then declined, for
//! the same reason a DOS 3.3 dump is: nothing but a ProDOS volume directory is
//! allowed to make `volume_is_sane` answer `Some`.
//!
//! # Layer 1b — DOS sector order (SQ-0864)
//!
//! A **third** wrapper, and the one that is not an offset. Apple II 5.25-inch
//! media is dumped in the order the drive numbers its sectors, not the order
//! ProDOS numbers its blocks, so the volume directory of such an image is not at
//! offset 1024 and the sniff above declines it — which is what this paragraph
//! used to say was the end of the matter. It is not: those bytes are a ProDOS
//! volume, merely shuffled, and [`crate::dos_order`] unshuffles them. The
//! fourteen 5.25-inch images in the corpus — `shogun_s1.dsk`…`s5`,
//! `zork_zero_1.dsk`…`_4` and `journey_s1.dsk`…`s5` — are bare 143,360-byte
//! dumps that mount here as `SHOGUN.1`…`SHOGUN.5`, `ZORK0.1`…`ZORK0.4` and
//! `JOURNEY.1`…`JOURNEY.5` once they are.
//!
//! It stays a wrapper rather than becoming a format for the same reason `2IMG`
//! does: what comes out is ProDOS, read by this reader, answering to
//! `blorb::medium`'s ProDOS row. Nibble images (2IMG format 2) are still not
//! decoded — that is a track encoding, not a permutation, and it arrives with
//! its own decoder or not at all.
//!
//! # Layer 2 — ProDOS
//!
//! Blocks are 512 bytes; the volume directory is a chain starting at block 2.
//! Field offsets are from Apple's *ProDOS 8 Technical Reference Manual*, chapter
//! 4 ("File Organization"):
//!
//! * **Directory block** — two `u16` pointers, previous and next, then thirteen
//!   39-byte entries.
//! * **Volume directory header**, the first entry of block 2: storage type `$F`
//!   in the high nibble with the name length in the low, a 15-byte name, then
//!   `entry_length` ($27), `entries_per_block` ($0D), `file_count`,
//!   `bit_map_pointer` and `total_blocks`.
//! * **File entry** — storage type and name length, name, file type,
//!   `key_pointer`, `blocks_used`, a 24-bit `EOF`, and the dates.
//! * **Subdirectory** — storage type `$D`; its `key_pointer` is the first block
//!   of a directory whose own first entry is a `$E` subdirectory header.
//!
//! A file's data is reached by its storage type:
//!
//! * `$1` **seedling** — the key pointer IS the data block.
//! * `$2` **sapling** — the key pointer is an index block of up to 256 pointers,
//!   *"stored with the low byte in the first half of the block, and the high byte
//!   in the second half"*.
//! * `$3` **tree** — the key pointer is a master index block of up to 128
//!   pointers to index blocks, split the same way.
//! * `$5` **extended** (GS/OS forked files) — the key pointer is an extended key
//!   block holding a mini-entry per fork, *"with data at +$0000 and rsrc at
//!   +$0100"*: storage type, key block, blocks used and EOF. This reader reads
//!   the **data** fork, exactly as [`crate::hfs`] reads a Macintosh data fork,
//!   and for the same reason — an Infocom release keeps its story in one.
//!
//! A zero pointer anywhere in an index or master index block is a **sparse**
//! run: *"Zeroed entries in index and master index blocks indicate sparse
//! sections of the file, which are treated as blocks full of `$00` bytes."*
//!
//! # Choosing what to run
//!
//! By CONTENT ([`looks_like_story`]), like every other reader here. There is no
//! conventional story name to fall back on: ProDOS releases name the file after
//! the game (`BZ.DAT`, `ZORK.I`, `TRINITY`), so when a volume offers several the
//! largest wins and disk order breaks an exact tie. A compilation wants
//! [`ProDos::files`] and a chooser, not [`ProDos::story`].
//!
//! The ProDOS **file type** byte is deliberately not used, though it is
//! suggestive: the *Lost Treasures* volumes store every game as type `$F5` and
//! every saved game as `$F8`, while the standalone *Beyond Zork* disk stores its
//! `BZ.DAT` as `$AF`. Two spellings for one thing across two presses of the same
//! publisher is exactly why this crate identifies by bytes.
//!
//! # What is actually on these ten disks
//!
//! Measured through this reader, and worth writing down because two of them are
//! surprising:
//!
//! * **`Beyond Zork (1988)(Infocom).2mg`** — volume `BEYOND.ZORK`, 26 files.
//!   `BZ.DAT` is *Beyond Zork* v5 r57 s871221. Everything else is GS/OS: a
//!   `SYSTEM/` tree, tool sets, and `BZ.SYS16`, an Apple **IIgs** application.
//! * **The seven *Lost Treasures* volumes** — `INFOCOM1`…`INFOCOM7`. Volume 1 is
//!   the GS/OS launcher and carries **no game at all**; volumes 2–7 carry
//!   thirty between them, v3 through v5.
//! * **`Arthur Quest 4 Excalibur.2mg` and `Journey.2mg` carry no whole story
//!   file.** They are the ProDOS **8** Apple II presses — `INFOCOM.SYSTEM`
//!   beside `BASIC.SYSTEM` — and the game is split across `ARTHUR.D1`…`D5` /
//!   `JOURNEY.D1`…`D4`, none of which begins with a Z-machine header. (Arthur's
//!   v6 r63 s890622 header sits 18 bytes into a block partway through `.D1`, so
//!   the segments are a container of their own and not story images.) Both disks
//!   mount and offer nothing, which is what lets a caller say "this is the wrong
//!   disk" rather than "corrupt story file". Reading the segmented Apple II
//!   format is a separate piece of work.
//!
//! # One image at a time
//!
//! A mount is one disk, the same real limit [`crate::fat12`] documents: the
//! seven *Lost Treasures* volumes each mount perfectly on their own, and
//! presenting them as one collection is a **set** model this module does not
//! have.

use crate::adf::looks_like_story;
use crate::infocom_pics::InfocomPics;

/// A ProDOS block. Every offset below is relative to one.
pub const BLOCK: usize = 512;

// ── The 2IMG wrapper ─────────────────────────────────────────────────────────

/// `2IMG`, the wrapper's magic.
const TWO_IMG_MAGIC: &[u8; 4] = b"2IMG";
/// Header length, `u16` — 64 on every image in the corpus.
const TWO_IMG_HEADER_LEN: usize = 0x08;
/// Image data format, `u32`.
const TWO_IMG_FORMAT: usize = 0x0c;
/// 512-byte blocks, `u32`; meaningful when the format is ProDOS order.
const TWO_IMG_BLOCKS: usize = 0x14;
/// Offset from the start of the file to the data, `u32`.
const TWO_IMG_DATA_OFFSET: usize = 0x18;
/// Length of the data in bytes, `u32` — and see the module docs, because it is
/// zero on every image here.
const TWO_IMG_DATA_LENGTH: usize = 0x1c;
/// The shortest header the format allows, and the only one anything writes.
const TWO_IMG_MIN_HEADER: usize = 64;
/// `image data format` 1: the blocks are in ProDOS order, which is the only
/// order this reader can address.
const TWO_IMG_PRODOS_ORDER: u32 = 1;

// ── ProDOS directories ───────────────────────────────────────────────────────

/// The volume directory's key block.
const VOLUME_DIR_BLOCK: u16 = 2;
/// `previous` block pointer, `u16`.
const DIR_PREV: usize = 0;
/// `next` block pointer, `u16`.
const DIR_NEXT: usize = 2;
/// Where a directory block's entries start.
const DIR_FIRST_ENTRY: usize = 4;
/// `entry_length`: always `$27`.
const ENTRY_LEN: usize = 0x27;
/// `entries_per_block`: always `$0D`.
const ENTRIES_PER_BLOCK: usize = 0x0d;

// Fields of a file entry, offset from the entry.
/// Storage type in the high nibble, name length in the low.
const E_STORAGE_NAME: usize = 0x00;
/// The 15-byte file name.
const E_NAME: usize = 0x01;
/// ProDOS file type.
const E_FILE_TYPE: usize = 0x10;
/// `key_pointer`, `u16`.
const E_KEY: usize = 0x11;
/// `EOF`, three bytes little-endian.
const E_EOF: usize = 0x15;

// Fields of a volume-directory header, offset from the HEADER ENTRY (which
// itself begins at `DIR_FIRST_ENTRY` of the key block, so these are the
// manual's block-relative offsets less four).
/// `entry_length`.
const H_ENTRY_LEN: usize = 0x1f;
/// `entries_per_block`.
const H_ENTRIES_PER_BLOCK: usize = 0x20;
/// `bit_map_pointer`, `u16`.
const H_BIT_MAP: usize = 0x23;
/// `total_blocks`, `u16`.
const H_TOTAL_BLOCKS: usize = 0x25;

/// Seedling: the key pointer is the one data block.
const ST_SEEDLING: u8 = 1;
/// Sapling: the key pointer is an index block.
const ST_SAPLING: u8 = 2;
/// Tree: the key pointer is a master index block.
const ST_TREE: u8 = 3;
/// Extended (GS/OS forked): the key pointer is an extended key block.
const ST_EXTENDED: u8 = 5;
/// A subdirectory entry.
const ST_SUBDIR: u8 = 0x0d;
/// A subdirectory header — the first entry of a subdirectory's key block.
const ST_SUBDIR_HEADER: u8 = 0x0e;
/// A volume directory header — the first entry of block 2.
const ST_VOLUME_HEADER: u8 = 0x0f;

/// Pointers in a sapling's index block: 256 low bytes then 256 high bytes.
const INDEX_POINTERS: usize = 256;
/// Pointers in a tree's master index block, split the same way.
const MASTER_POINTERS: usize = 128;

/// The data fork's mini-entry in an extended key block.
const FORK_DATA: usize = 0x000;
/// Storage type, within a mini-entry.
const MINI_STORAGE: usize = 0;
/// `key_block`, `u16`, within a mini-entry.
const MINI_KEY: usize = 1;
/// `EOF`, three bytes little-endian, within a mini-entry.
const MINI_EOF: usize = 5;

/// The longest ProDOS file name.
const MAX_NAME: usize = 15;
/// How deep the directory walk will recurse. The corpus reaches two levels
/// (`SYSTEM/SYSTEM.SETUP/TOOL.SETUP`); this only stops a malformed image from
/// running away.
const MAX_DEPTH: usize = 8;
/// The smallest volume this reader will believe in: the loader, the volume
/// directory and one block of data.
const MIN_VOLUME_BLOCKS: usize = 3;

/// Errors that can arise while mounting a ProDOS image.
#[derive(Debug, PartialEq, Eq)]
pub enum ProDosError {
    /// The bytes are not a ProDOS volume, wrapped or bare: no volume directory
    /// where one has to be, or a header whose geometry does not describe the
    /// image it sits in.
    NotProDos,
}

/// One file found on the volume. Directories are not listed; the walk descends
/// into them and reports what is inside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProDosEntry {
    /// The ProDOS file name, exactly as stored (`BZ.DAT`, `ZORK.I`).
    pub name: String,
    /// The directory path it lives in, or `None` at the volume root. ProDOS
    /// nests, so this is the whole prefix — `SYSTEM/SYSTEM.SETUP`, not just the
    /// innermost folder.
    pub dir: Option<String>,
    /// Size in bytes: the entry's `EOF`, or the data fork's for an extended
    /// file.
    pub size: usize,
    /// The ProDOS file type byte. Reported so a listing can say what a file is;
    /// never used to decide what it is (see the module docs).
    pub file_type: u8,
    /// Storage type: seedling, sapling, tree or extended.
    storage: u8,
    /// The entry's `key_pointer`.
    key: u16,
}

impl ProDosEntry {
    /// How this file is named to the outside world: `DIR/NAME` inside a
    /// directory, the bare name at the root.
    pub fn path(&self) -> String {
        match &self.dir {
            Some(d) => format!("{d}/{}", self.name),
            None => self.name.clone(),
        }
    }
}

/// A mounted ProDOS volume.
#[derive(Debug)]
pub struct ProDos {
    image: Vec<u8>,
    /// Where block 0 starts in `image`: 0 bare, the 2IMG data offset otherwise.
    data: usize,
    /// Blocks on the volume, as its own directory header declares.
    blocks: usize,
    name: String,
    files: Vec<ProDosEntry>,
}

impl ProDos {
    /// Cheap sniff: does this look like a ProDOS disk image?
    ///
    /// By CONTENT — the `.2mg` extension the corpus happens to use means nothing
    /// in particular, and neither does the `.po` one that `Shogun.po` wears over
    /// a DiskCopy wrapper. The volume directory decides: a `$F` header entry at
    /// block 2 carrying a legal volume name, the format's fixed `$27`/`$0D`
    /// entry geometry, and a `total_blocks` that fits the bytes in hand. An
    /// AmigaDOS, HFS, FAT12, Z-machine, Glulx, Blorb or Scott image can never
    /// collide — none of them has that shape a kilobyte in, wrapped or not.
    ///
    /// A 5.25-inch dump is asked the same question after its sectors are put
    /// back in block order; see [`crate::dos_order`], and see [`ProDos::mount`]
    /// for why that is an unwrapping rather than a second format.
    pub fn looks_like_prodos(raw: &[u8]) -> bool {
        volume_at(raw).is_some() || dos_ordered(raw).is_some()
    }

    /// Mount an image and enumerate it, subdirectories and all.
    ///
    /// Four placements are tried, and the volume directory decides between
    /// them: a bare volume, one behind a `2IMG` header, one behind a DiskCopy
    /// 4.2 header (SQ-0889), and — SQ-0864 — a 5.25-inch dump whose sectors are
    /// in DOS order. The last is the only one that MOVES bytes rather than
    /// merely offsetting them, so it produces a new image and the mount goes on
    /// with that; everything past this line is the same ProDOS volume it always
    /// was.
    pub fn mount(image: Vec<u8>) -> Result<ProDos, ProDosError> {
        let image = match volume_at(&image) {
            Some(_) => image,
            None => dos_ordered(&image).ok_or(ProDosError::NotProDos)?,
        };
        let (data, blocks) = volume_at(&image).ok_or(ProDosError::NotProDos)?;
        let mut fs = ProDos { image, data, blocks, name: String::new(), files: Vec::new() };
        let key = fs.block(VOLUME_DIR_BLOCK).ok_or(ProDosError::NotProDos)?;
        fs.name = entry_name(&key[DIR_FIRST_ENTRY..]).ok_or(ProDosError::NotProDos)?;
        let mut files = Vec::new();
        fs.walk(VOLUME_DIR_BLOCK, None, 0, &mut files);
        fs.files = files;
        Ok(fs)
    }

    /// The volume's own name, as ProDOS stored it (`BEYOND.ZORK`, `INFOCOM2`).
    pub fn volume_name(&self) -> &str {
        &self.name
    }

    /// Every file on the volume, in directory order, subdirectory contents
    /// included. Directories themselves are not listed.
    pub fn files(&self) -> &[ProDosEntry] {
        &self.files
    }

    /// One block, or `None` when it is off the end of the volume.
    fn block(&self, n: u16) -> Option<&[u8]> {
        let n = usize::from(n);
        if n >= self.blocks {
            return None;
        }
        self.image.get(self.data + n * BLOCK..self.data + (n + 1) * BLOCK)
    }

    /// Walk one directory, following its block chain and recursing into
    /// subdirectories.
    ///
    /// The walk is bounded twice over — by the blocks on the volume and by a
    /// visited set — so a directory chained into a cycle terminates.
    fn walk(&self, first: u16, parent: Option<&str>, depth: usize, out: &mut Vec<ProDosEntry>) {
        let mut seen = vec![false; self.blocks];
        let mut b = first;
        while let Some(dir) = self.block(b) {
            if seen[usize::from(b)] {
                return;
            }
            seen[usize::from(b)] = true;
            for i in 0..ENTRIES_PER_BLOCK {
                let at = DIR_FIRST_ENTRY + i * ENTRY_LEN;
                let Some(e) = dir.get(at..at + ENTRY_LEN) else { break };
                let storage = e[E_STORAGE_NAME] >> 4;
                // `$0` is an inactive entry, and a deleted file leaves one in
                // the middle of a directory — so this SKIPS rather than stops.
                // The two header types are the directory describing itself.
                if matches!(storage, 0 | ST_SUBDIR_HEADER | ST_VOLUME_HEADER) {
                    continue;
                }
                let Some(name) = entry_name(e) else { continue };
                let key = le16(e, E_KEY);
                if storage == ST_SUBDIR {
                    if depth >= MAX_DEPTH {
                        continue;
                    }
                    let path = match parent {
                        Some(p) => format!("{p}/{name}"),
                        None => name,
                    };
                    self.walk(key, Some(&path), depth + 1, out);
                    continue;
                }
                let size = le24(e, E_EOF);
                // An extended file's own EOF describes its extended key block,
                // not its data; the fork's mini-entry is what knows.
                let size = if storage == ST_EXTENDED {
                    match self.data_fork(key) {
                        Some((_, _, eof)) => eof,
                        None => continue,
                    }
                } else {
                    size
                };
                out.push(ProDosEntry {
                    name,
                    dir: parent.map(str::to_string),
                    size,
                    file_type: e[E_FILE_TYPE],
                    storage,
                    key,
                });
            }
            b = le16(dir, DIR_NEXT);
            if b == 0 {
                return;
            }
        }
    }

    /// The data fork of an extended file, as `(storage type, key, EOF)`.
    fn data_fork(&self, key: u16) -> Option<(u8, u16, usize)> {
        let ext = self.block(key)?;
        let mini = ext.get(FORK_DATA..FORK_DATA + 8)?;
        Some((mini[MINI_STORAGE], le16(mini, MINI_KEY), le24(mini, MINI_EOF)))
    }

    /// Read a file's bytes, or `None` when its block chain runs short of the
    /// size its entry declares.
    pub fn read(&self, entry: &ProDosEntry) -> Option<Vec<u8>> {
        let (storage, key, size) = if entry.storage == ST_EXTENDED {
            self.data_fork(entry.key)?
        } else {
            (entry.storage, entry.key, entry.size)
        };
        let mut out = Vec::new();
        self.read_fork(storage, key, size, &mut out)?;
        (out.len() >= size).then(|| {
            out.truncate(size);
            out
        })
    }

    /// Append `size` bytes of a fork to `out`, following whichever block tree
    /// its storage type describes.
    fn read_fork(&self, storage: u8, key: u16, size: usize, out: &mut Vec<u8>) -> Option<()> {
        match storage {
            ST_SEEDLING => out.extend_from_slice(self.block(key)?),
            ST_SAPLING => self.read_index(key, size, out)?,
            ST_TREE => {
                let master = self.block(key)?;
                for i in 0..MASTER_POINTERS {
                    if out.len() >= size {
                        break;
                    }
                    match u16::from_le_bytes([master[i], master[BLOCK / 2 + i]]) {
                        // A sparse run wide enough that ProDOS did not allocate
                        // the index block either.
                        0 => sparse(out, INDEX_POINTERS * BLOCK, size),
                        index => self.read_index(index, size, out)?,
                    }
                }
            }
            // An extended file's forks are ordinary seedlings, saplings and
            // trees; nothing nests a second level.
            _ => return None,
        }
        Some(())
    }

    /// Append the blocks one index block names.
    fn read_index(&self, index: u16, size: usize, out: &mut Vec<u8>) -> Option<()> {
        let idx = self.block(index)?;
        for i in 0..INDEX_POINTERS {
            if out.len() >= size {
                break;
            }
            match u16::from_le_bytes([idx[i], idx[BLOCK / 2 + i]]) {
                0 => sparse(out, BLOCK, size),
                b => out.extend_from_slice(self.block(b)?),
            }
        }
        Some(())
    }

    /// Read a file by path or by bare name, case-insensitively.
    ///
    /// `read_named("TOOL.SETUP")` on the *Lost Treasures* launcher matches the
    /// first such file; `read_named("SYSTEM/SYSTEM.SETUP/TOOL.SETUP")` names the
    /// one you meant. Prefer [`ProDos::story`] / [`ProDos::pictures`], which
    /// identify by content.
    ///
    /// **A full path is tried before any bare name**, in two passes rather than
    /// one predicate, and the launcher volume is why: it carries three files
    /// called `FINDER.DATA` — one at the root, one under `SYSTEM/FONTS` and one
    /// under `ICONS` — so a single pass matching either spelling resolves the
    /// ROOT one to whichever namesake happens to sit earlier in the directory.
    /// That breaks the property every format here owes its callers, that a name
    /// they were shown is a name they can ask for.
    pub fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        let e = self
            .files
            .iter()
            .find(|e| e.path().eq_ignore_ascii_case(name))
            .or_else(|| self.files.iter().find(|e| e.name.eq_ignore_ascii_case(name)))?;
        self.read(e)
    }

    /// The story image on this volume, with the path it was stored under.
    ///
    /// Every file is tested with [`looks_like_story`], so the saved games sitting
    /// beside the games are never offered. **There is no conventional name to
    /// break a tie with here** — a ProDOS release names the file after the game —
    /// so the largest candidate wins and disk order settles an exact tie, which
    /// is deterministic rather than directory-order luck. A compilation wants
    /// [`ProDos::files`] and a chooser, not this.
    ///
    /// **A ProDOS release can hold a story without storing it as a file**, and
    /// two in this corpus do: *Arthur* and *Journey* page theirs out of the
    /// segmented `.D1`…`.D5` container, which no per-file test can see. So when
    /// nothing on the volume IS a story, the volume is asked whether it holds
    /// one — see [`crate::infocom_packed`], and [`ProDos::packed_story`].
    pub fn story(&self) -> Option<(String, Vec<u8>)> {
        let mut cands: Vec<(String, Vec<u8>)> = self
            .files
            .iter()
            .filter_map(|e| self.read(e).map(|b| (e.path(), b)))
            .filter(|(_, b)| looks_like_story(b))
            .collect();
        cands.sort_by_key(|(_, bytes)| std::cmp::Reverse(bytes.len()));
        cands.into_iter().next().or_else(|| self.packed_story())
    }

    /// The story held in a packed Apple volume on this disk, reassembled out of
    /// its segments — or `None` when the disk carries no such container, or
    /// carries one whose segments are not all here.
    ///
    /// The reader is [`crate::infocom_packed`]; this is only the door to it, and
    /// it is a separate method because the container is a separate thing. A
    /// ProDOS volume with a `.D1` on it is still an ordinary ProDOS volume.
    pub fn packed_story(&self) -> Option<(String, Vec<u8>)> {
        let files: Vec<(String, Vec<u8>)> =
            self.files.iter().filter_map(|e| self.read(e).map(|b| (e.path(), b))).collect();
        crate::infocom_packed::story(&files)
    }

    /// The native Infocom picture archive **file** on this volume, with its
    /// stored path. Identified by parsing, exactly as
    /// [`crate::adf::Adf::pictures`] does.
    ///
    /// No ProDOS volume in the corpus keeps its artwork as a *file*: the two
    /// graphical releases here (*Arthur* and *Journey*) keep theirs inside the
    /// same segmented `.D1`…`.D5` container as their story, so no per-file parse
    /// can reach it. That is why this falls through to
    /// [`Self::packed_pictures`], exactly as [`Self::story`] falls through to
    /// [`Self::packed_story`] — the file tier is kept because a ProDOS volume
    /// with a loose archive on it would be an ordinary volume, and the container
    /// tier is what the corpus actually answers with.
    ///
    /// **The fall-through moves the story's screen, and that is correct**
    /// (SQ-0863). An archive states a picture space and the Apple's is 140×192
    /// (`apple.equ`'s `MAXWIDTH`/`MAXHEIGHT`), which `app`'s
    /// `PictSource::native_std_window` turns into a 560×384 story window where
    /// *Arthur* release 63 used to be laid out on 640×400. The 640×400 was the
    /// ARTLESS fallback — the profile's, reached only while nothing declared a
    /// space — and an archive outranks a profile for the same reason the
    /// standard Macintosh's monochrome `Pic.data` outranks `MACINTOSH_STD_WINDOW`
    /// and lays Zork Zero out on 480×300 (SQ-0838). The Apple press is a
    /// different build on a different machine and it is entitled to its own
    /// screen.
    ///
    /// This does not reopen `InterpreterProfile::AppleIIgs::std_window`, which
    /// SQ-0857 left `None` and which stays `None`: the archive supplies the
    /// space, so the profile never has to.
    pub fn pictures(&self) -> Option<(String, InfocomPics)> {
        let mut cands: Vec<(String, InfocomPics)> = self
            .files
            .iter()
            .filter_map(|e| self.read(e).map(|b| (e.path(), b)))
            .filter(|(_, b)| !looks_like_story(b))
            .filter_map(|(path, b)| InfocomPics::parse(b).ok().map(|p| (path, p)))
            .filter(|(_, p)| p.entries().iter().any(|e| e.has_pixels()))
            .collect();
        cands.sort_by_key(|(path, pics)| (std::cmp::Reverse(pics.entries().len()), path.clone()));
        cands.into_iter().next().or_else(|| self.packed_pictures())
    }

    /// The artwork held in a packed Apple volume on this disk, merged out of the
    /// archives its segments carry — or `None` when the disk holds no such
    /// container, when no segment carries art, or when a segment that should is
    /// missing.
    ///
    /// The reader is [`crate::infocom_packed`]; this is only the door to it, and
    /// it is a separate method for the same reason [`Self::packed_story`] is.
    pub fn packed_pictures(&self) -> Option<(String, InfocomPics)> {
        let files: Vec<(String, Vec<u8>)> =
            self.files.iter().filter_map(|e| self.read(e).map(|b| (e.path(), b))).collect();
        crate::infocom_packed::pictures(&files)
    }
}

/// Extend `out` with a sparse run of at most `want` bytes, never past `size`.
fn sparse(out: &mut Vec<u8>, want: usize, size: usize) {
    let run = want.min(size.saturating_sub(out.len()));
    out.resize(out.len() + run, 0);
}

/// Where the ProDOS volume starts inside `raw` and how many blocks it has — 0
/// for a bare volume, the 2IMG data offset inside a `2IMG` wrapper, 84 inside a
/// DiskCopy 4.2 one — or `None` when no placement holds one.
fn volume_at(raw: &[u8]) -> Option<(usize, usize)> {
    if let Some((at, len)) = two_img_data(raw) {
        if let Some(blocks) = volume_is_sane(&raw[at..at + len]) {
            return Some((at, blocks));
        }
    }
    // DiskCopy 4.2, whose unwrap is [`crate::hfs`]'s and is shared rather than
    // rewritten (SQ-0889). The header declares the volume's length; the tags
    // that follow it are not part of the volume and are never addressed.
    if let Some(len) = crate::hfs::diskcopy_volume_len(raw) {
        let at = crate::hfs::DISKCOPY_HEADER;
        if let Some(blocks) = volume_is_sane(&raw[at..at + len]) {
            return Some((at, blocks));
        }
    }
    volume_is_sane(raw).map(|blocks| (0, blocks))
}

/// `raw` de-interleaved out of DOS sector order, when it is a 5.25-inch dump
/// that holds a ProDOS volume once it is — else `None` (SQ-0864).
///
/// The re-order is [`crate::dos_order`]'s and the verdict is still
/// [`volume_is_sane`]'s: a DOS 3.3 or Pascal 5.25-inch disk is re-ordered just
/// as willingly and then declined, because nothing but a ProDOS volume directory
/// is allowed to make this answer `Some`.
fn dos_ordered(raw: &[u8]) -> Option<Vec<u8>> {
    let volume = crate::dos_order::prodos_order(raw)?;
    volume_is_sane(&volume).map(|_| volume)
}

/// The disk data a 2IMG header describes, as `(offset, length)`.
///
/// The length field is zero on every image in this corpus, so the block count is
/// the fallback and the tail of the file is the last resort; see the module
/// docs. Whatever it lands on is a whole number of blocks and is present.
fn two_img_data(raw: &[u8]) -> Option<(usize, usize)> {
    if raw.len() < TWO_IMG_MIN_HEADER || !raw.starts_with(TWO_IMG_MAGIC) {
        return None;
    }
    let header = usize::from(le16(raw, TWO_IMG_HEADER_LEN));
    if header < TWO_IMG_MIN_HEADER {
        return None;
    }
    let at = le32(raw, TWO_IMG_DATA_OFFSET) as usize;
    if at < header || at >= raw.len() {
        return None;
    }
    let declared = le32(raw, TWO_IMG_DATA_LENGTH) as usize;
    let blocks = le32(raw, TWO_IMG_BLOCKS) as usize;
    let len = match (declared, le32(raw, TWO_IMG_FORMAT)) {
        (0, TWO_IMG_PRODOS_ORDER) if blocks != 0 => blocks.checked_mul(BLOCK)?,
        (0, _) => raw.len() - at,
        (n, _) => n,
    };
    (len >= MIN_VOLUME_BLOCKS * BLOCK && len.is_multiple_of(BLOCK) && at + len <= raw.len())
        .then_some((at, len))
}

/// Does `volume` open with a ProDOS volume directory that describes it? Answers
/// the block count when it does.
///
/// The name alone would fire on noise; the format's fixed geometry is what makes
/// this safe to run over arbitrary bytes ahead of story loading. The key block
/// has no predecessor, the header entry is a `$F`, the entry geometry is ProDOS's
/// own `$27`/`$0D`, the bitmap is on the volume, and the last block the header
/// claims has to be inside the bytes in hand — which a wrapper's 64- or 84-byte
/// offset breaks, so no two placements can both pass.
fn volume_is_sane(volume: &[u8]) -> Option<usize> {
    let key = volume.get(usize::from(VOLUME_DIR_BLOCK) * BLOCK..)?.get(..BLOCK)?;
    if le16(key, DIR_PREV) != 0 {
        return None;
    }
    let header = &key[DIR_FIRST_ENTRY..];
    if header[E_STORAGE_NAME] >> 4 != ST_VOLUME_HEADER {
        return None;
    }
    volume_name(header)?;
    if header[H_ENTRY_LEN] as usize != ENTRY_LEN
        || header[H_ENTRIES_PER_BLOCK] as usize != ENTRIES_PER_BLOCK
    {
        return None;
    }
    let blocks = usize::from(le16(header, H_TOTAL_BLOCKS));
    let bitmap = usize::from(le16(header, H_BIT_MAP));
    if blocks < MIN_VOLUME_BLOCKS || blocks * BLOCK > volume.len() {
        return None;
    }
    (bitmap > 0 && bitmap < blocks).then_some(blocks)
}

/// The name in an entry, when it is a legal ProDOS one: a letter, then letters,
/// digits and periods, at most fifteen.
///
/// This is the strict form, and only the SNIFF uses it — a volume name is the
/// evidence that these bytes are a ProDOS volume at all, so it may not be
/// generous. See [`entry_name`] for what a FILE is allowed to be called.
fn volume_name(entry: &[u8]) -> Option<String> {
    let name = entry_name(entry)?;
    let mut chars = name.chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.'))
    .then_some(name)
}

/// The name in an entry: the length is the low nibble of the storage byte, and
/// the bytes follow.
///
/// Deliberately more generous than [`volume_name`]. ProDOS 8 wrote uppercase
/// letters, digits and periods and nothing else, but GS/OS carries mixed case
/// and this reader does not select a file by name — the story is identified by
/// content — so anything printable is reported as it was stored. Control bytes
/// are not: a directory full of garbage is not a directory.
fn entry_name(entry: &[u8]) -> Option<String> {
    let len = usize::from(entry[E_STORAGE_NAME] & 0x0f);
    if len == 0 || len > MAX_NAME {
        return None;
    }
    let raw = entry.get(E_NAME..E_NAME + len)?;
    if raw.iter().any(|c| !(0x20..0x7f).contains(c) || *c == b'/') {
        return None;
    }
    Some(raw.iter().map(|c| char::from(*c)).collect())
}

/// Little-endian word at `off`.
fn le16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

/// Little-endian 24-bit value at `off` — ProDOS's `EOF`.
fn le24(b: &[u8], off: usize) -> usize {
    usize::from(b[off]) | usize::from(b[off + 1]) << 8 | usize::from(b[off + 2]) << 16
}

/// Little-endian longword at `off`.
fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An 800 KB 3.5" ProDOS volume, which is what all ten images in the corpus
    /// are.
    const VOLUME_BLOCKS: usize = 1600;
    /// Blocks 0–1 are the loader, 2–5 the volume directory, 6 the bitmap.
    const FIRST_FREE: u16 = 7;

    /// Builder for a synthetic ProDOS volume, so the reader's tests need no
    /// fixture. Writes real seedlings, saplings, trees and subdirectories.
    pub(crate) struct VolumeBuilder {
        volume: Vec<u8>,
        /// Next free block.
        next: u16,
        /// Entries written into the volume directory's key block so far,
        /// including the header.
        root_used: usize,
    }

    impl VolumeBuilder {
        pub(crate) fn new(name: &str) -> VolumeBuilder {
            let mut b =
                VolumeBuilder { volume: vec![0u8; VOLUME_BLOCKS * BLOCK], next: FIRST_FREE, root_used: 0 };
            let key = usize::from(VOLUME_DIR_BLOCK) * BLOCK;
            let header = key + DIR_FIRST_ENTRY;
            b.volume[header + E_STORAGE_NAME] = (ST_VOLUME_HEADER << 4) | name.len() as u8;
            b.volume[header + E_NAME..header + E_NAME + name.len()].copy_from_slice(name.as_bytes());
            b.volume[header + H_ENTRY_LEN] = ENTRY_LEN as u8;
            b.volume[header + H_ENTRIES_PER_BLOCK] = ENTRIES_PER_BLOCK as u8;
            b.put16(header + H_BIT_MAP, 6);
            b.put16(header + H_TOTAL_BLOCKS, VOLUME_BLOCKS as u16);
            b.root_used = 1;
            b
        }

        fn put16(&mut self, at: usize, v: u16) {
            self.volume[at..at + 2].copy_from_slice(&v.to_le_bytes());
        }

        fn take(&mut self) -> u16 {
            let b = self.next;
            self.next += 1;
            b
        }

        fn write_block(&mut self, block: u16, data: &[u8]) {
            let at = usize::from(block) * BLOCK;
            let n = data.len().min(BLOCK);
            self.volume[at..at + n].copy_from_slice(&data[..n]);
        }

        /// Lay `data` down as a seedling, sapling or tree, whichever its length
        /// calls for, and return `(storage type, key block)`.
        ///
        /// `sparse_every` blanks every Nth data block: ProDOS leaves a zero
        /// pointer for a run it never allocated, and a reader that treats one as
        /// an error or as end-of-file gets a short file.
        fn write_file(&mut self, data: &[u8], sparse_every: usize) -> (u8, u16) {
            let blocks = data.len().div_ceil(BLOCK).max(1);
            if blocks == 1 {
                let b = self.take();
                self.write_block(b, data);
                return (ST_SEEDLING, b);
            }
            let mut indexes = Vec::new();
            let mut written = 0usize;
            while written < blocks {
                let run = INDEX_POINTERS.min(blocks - written);
                let index = self.take();
                let mut lo = vec![0u8; BLOCK];
                for i in 0..run {
                    let src = (written + i) * BLOCK;
                    let end = data.len().min(src + BLOCK);
                    let sparse = sparse_every > 0
                        && (written + i).is_multiple_of(sparse_every)
                        && data[src..end].iter().all(|b| *b == 0);
                    if sparse {
                        continue; // a zero pointer: the hole is the point
                    }
                    let b = self.take();
                    self.write_block(b, &data[src..end]);
                    lo[i] = b as u8;
                    lo[BLOCK / 2 + i] = (b >> 8) as u8;
                }
                self.write_block(index, &lo);
                indexes.push(index);
                written += run;
            }
            if indexes.len() == 1 {
                return (ST_SAPLING, indexes[0]);
            }
            let master = self.take();
            let mut m = vec![0u8; BLOCK];
            for (i, index) in indexes.iter().enumerate() {
                m[i] = *index as u8;
                m[BLOCK / 2 + i] = (*index >> 8) as u8;
            }
            self.write_block(master, &m);
            (ST_TREE, master)
        }

        fn entry(name: &str, storage: u8, file_type: u8, key: u16, size: usize) -> Vec<u8> {
            // ProDOS names are at most fifteen characters, so a longer one is
            // shortened here exactly as FAT12's builder shortens an over-long
            // 8.3 name. The name is not what is under test.
            let name = &name[..name.len().min(MAX_NAME)];
            let mut e = vec![0u8; ENTRY_LEN];
            e[E_STORAGE_NAME] = (storage << 4) | name.len() as u8;
            e[E_NAME..E_NAME + name.len()].copy_from_slice(name.as_bytes());
            e[E_FILE_TYPE] = file_type;
            e[E_KEY..E_KEY + 2].copy_from_slice(&key.to_le_bytes());
            e[E_EOF] = size as u8;
            e[E_EOF + 1] = (size >> 8) as u8;
            e[E_EOF + 2] = (size >> 16) as u8;
            e
        }

        /// Write an entry into the volume directory, spilling into a second
        /// directory block once the first thirteen are used.
        fn push_root(&mut self, e: &[u8]) {
            let block = VOLUME_DIR_BLOCK + (self.root_used / ENTRIES_PER_BLOCK) as u16;
            let slot = self.root_used % ENTRIES_PER_BLOCK;
            let at = usize::from(block) * BLOCK + DIR_FIRST_ENTRY + slot * ENTRY_LEN;
            self.volume[at..at + ENTRY_LEN].copy_from_slice(e);
            if slot == 0 && block > VOLUME_DIR_BLOCK {
                let prev = usize::from(block - 1) * BLOCK;
                self.put16(prev + DIR_NEXT, block);
                let this = usize::from(block) * BLOCK;
                self.put16(this + DIR_PREV, block - 1);
            }
            self.root_used += 1;
        }

        /// A file in the volume directory.
        pub(crate) fn add_file(&mut self, name: &str, data: &[u8]) {
            self.add_file_typed(name, 0xf5, data, 0);
        }

        pub(crate) fn add_file_typed(
            &mut self,
            name: &str,
            file_type: u8,
            data: &[u8],
            sparse_every: usize,
        ) {
            let (storage, key) = self.write_file(data, sparse_every);
            let e = Self::entry(name, storage, file_type, key, data.len());
            self.push_root(&e);
        }

        /// A GS/OS **extended** file: an extended key block whose data-fork
        /// mini-entry points at the real bytes. Every ProDOS image in the corpus
        /// that boots GS/OS carries several.
        pub(crate) fn add_extended_file(&mut self, name: &str, data: &[u8], rsrc: &[u8]) {
            let (dst, dkey) = self.write_file(data, 0);
            let (rst, rkey) = self.write_file(rsrc, 0);
            let ext = self.take();
            let mut b = vec![0u8; BLOCK];
            for (at, (st, key, len)) in
                [(FORK_DATA, (dst, dkey, data.len())), (0x100, (rst, rkey, rsrc.len()))]
            {
                b[at + MINI_STORAGE] = st;
                b[at + MINI_KEY..at + MINI_KEY + 2].copy_from_slice(&key.to_le_bytes());
                b[at + MINI_EOF] = len as u8;
                b[at + MINI_EOF + 1] = (len >> 8) as u8;
                b[at + MINI_EOF + 2] = (len >> 16) as u8;
            }
            self.write_block(ext, &b);
            // An extended file's own EOF describes the key block, exactly as the
            // real disks store it.
            let e = Self::entry(name, ST_EXTENDED, 0xb3, ext, BLOCK);
            self.push_root(&e);
        }

        /// A subdirectory holding `files` — the shape of the GS/OS `SYSTEM/`
        /// tree, one level of it.
        pub(crate) fn add_dir(&mut self, dir: &str, files: &[(&str, &[u8])]) {
            let block = self.take();
            let mut d = vec![0u8; BLOCK];
            // A subdirectory's own header is its first entry.
            let header = DIR_FIRST_ENTRY;
            d[header + E_STORAGE_NAME] = (ST_SUBDIR_HEADER << 4) | dir.len() as u8;
            d[header + E_NAME..header + E_NAME + dir.len()].copy_from_slice(dir.as_bytes());
            d[header + H_ENTRY_LEN] = ENTRY_LEN as u8;
            d[header + H_ENTRIES_PER_BLOCK] = ENTRIES_PER_BLOCK as u8;
            let mut entries = Vec::new();
            for (name, data) in files {
                let (storage, key) = self.write_file(data, 0);
                entries.push(Self::entry(name, storage, 0xf5, key, data.len()));
            }
            for (i, e) in entries.iter().enumerate() {
                let at = DIR_FIRST_ENTRY + (i + 1) * ENTRY_LEN;
                d[at..at + ENTRY_LEN].copy_from_slice(e);
            }
            self.write_block(block, &d);
            self.push_root(&Self::entry(dir, ST_SUBDIR, 0x0f, block, 0));
        }

        /// The bare volume, with no wrapper.
        pub(crate) fn finish(self) -> Vec<u8> {
            self.volume
        }
    }

    /// Wrap a volume the way CiderPress does — **including the zero data-length
    /// field**, which is the quirk the whole corpus carries.
    pub(crate) fn two_img(volume: &[u8], declared_length: u32) -> Vec<u8> {
        let mut out = vec![0u8; TWO_IMG_MIN_HEADER];
        out[..4].copy_from_slice(TWO_IMG_MAGIC);
        out[4..8].copy_from_slice(b"WOOF");
        out[TWO_IMG_HEADER_LEN..TWO_IMG_HEADER_LEN + 2]
            .copy_from_slice(&(TWO_IMG_MIN_HEADER as u16).to_le_bytes());
        out[0x0a..0x0c].copy_from_slice(&1u16.to_le_bytes());
        out[TWO_IMG_FORMAT..TWO_IMG_FORMAT + 4]
            .copy_from_slice(&TWO_IMG_PRODOS_ORDER.to_le_bytes());
        out[TWO_IMG_BLOCKS..TWO_IMG_BLOCKS + 4]
            .copy_from_slice(&((volume.len() / BLOCK) as u32).to_le_bytes());
        out[TWO_IMG_DATA_OFFSET..TWO_IMG_DATA_OFFSET + 4]
            .copy_from_slice(&(TWO_IMG_MIN_HEADER as u32).to_le_bytes());
        out[TWO_IMG_DATA_LENGTH..TWO_IMG_DATA_LENGTH + 4]
            .copy_from_slice(&declared_length.to_le_bytes());
        out.extend_from_slice(volume);
        out
    }

    /// One synthetic ProDOS floppy carrying `files` in its volume directory,
    /// 2IMG wrapper and all, for the mount-seam tests in [`crate::medium`]. They
    /// need a real volume of every format and cannot reach a builder that is
    /// private to this module.
    pub(crate) fn sample_disk(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = VolumeBuilder::new("TEST.DISK");
        for (name, data) in files {
            b.add_file(name, data);
        }
        two_img(&b.finish(), 0)
    }

    /// A minimal but structurally valid v5 story header, padded to `len`.
    fn fake_story(len: usize) -> Vec<u8> {
        let mut b = vec![0u8; len];
        b[0] = 5;
        let mut word = |o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_be_bytes());
        word(0x04, 0x0400); // high memory
        word(0x08, 0x0300); // dictionary
        word(0x0a, 0x0100); // objects
        word(0x0c, 0x0200); // globals
        word(0x0e, 0x0280); // static memory base
        word(0x1a, (len / 4) as u16); // file length, v5 unit
        b[0x12..0x18].copy_from_slice(b"871221");
        b
    }

    #[test]
    fn rejects_images_that_are_not_prodos_volumes() {
        assert!(!ProDos::looks_like_prodos(b"not a disk"));
        assert!(!ProDos::looks_like_prodos(&[]));
        assert!(!ProDos::looks_like_prodos(&vec![0u8; VOLUME_BLOCKS * BLOCK]), "no volume header");
        // An AmigaDOS boot block, an HFS volume and a FAT12 BPB all disagree
        // with a ProDOS volume directory at block 2.
        let mut adf = vec![0u8; 1760 * BLOCK];
        adf[0..3].copy_from_slice(b"DOS");
        assert!(!ProDos::looks_like_prodos(&adf));
        let mut hfs = vec![0u8; VOLUME_BLOCKS * BLOCK];
        hfs[2 * BLOCK..2 * BLOCK + 2].copy_from_slice(&0x4244u16.to_be_bytes());
        assert!(!ProDos::looks_like_prodos(&hfs), "a Macintosh volume is not a ProDOS one");
        assert_eq!(ProDos::mount(vec![0u8; 16]).unwrap_err(), ProDosError::NotProDos);
    }

    /// The format's fixed geometry is what keeps the sniff safe over arbitrary
    /// bytes; each of these is a field a real volume cannot get wrong.
    #[test]
    fn a_volume_header_that_does_not_describe_its_image_is_declined() {
        let mut b = VolumeBuilder::new("TEST.DISK");
        b.add_file("STORY", &fake_story(4096));
        let volume = b.finish();
        assert!(ProDos::looks_like_prodos(&volume));

        let header = usize::from(VOLUME_DIR_BLOCK) * BLOCK + DIR_FIRST_ENTRY;
        for (what, at, to) in [
            ("entry length", H_ENTRY_LEN, 0x26u8),
            ("entries per block", H_ENTRIES_PER_BLOCK, 0x0c),
            ("storage type", E_STORAGE_NAME, 0xd9),
        ] {
            let mut bent = volume.clone();
            bent[header + at] = to;
            assert!(!ProDos::looks_like_prodos(&bent), "{what} is not checked");
        }
        // A volume that claims more blocks than it has is not a volume.
        let mut big = volume.clone();
        big[header + H_TOTAL_BLOCKS..header + H_TOTAL_BLOCKS + 2]
            .copy_from_slice(&9000u16.to_le_bytes());
        assert!(!ProDos::looks_like_prodos(&big));
        // …and half a volume is not one either.
        assert!(!ProDos::looks_like_prodos(&volume[..volume.len() / 2]));
    }

    /// **The zero-length field**, which is the whole of the 2IMG wrapper's
    /// difficulty here: every image in the corpus declares `data length = 0` and
    /// a reader that believed it would find no volume at all.
    ///
    /// All three resolutions are pinned — an honest length, the block count, and
    /// the tail of the file — and so is the bare volume with no wrapper, which
    /// has no fixture in `stories/` and would otherwise be untested.
    #[test]
    fn reads_a_volume_bare_and_inside_a_2img_wrapper_whatever_it_declares() {
        let mut b = VolumeBuilder::new("TEST.DISK");
        b.add_file("STORY", &fake_story(4096));
        let volume = b.finish();

        let honest = two_img(&volume, volume.len() as u32);
        let zero = two_img(&volume, 0);
        // A wrapper whose block count is ALSO zero: only the tail of the file is
        // left to go on.
        let mut no_blocks = zero.clone();
        no_blocks[TWO_IMG_BLOCKS..TWO_IMG_BLOCKS + 4].copy_from_slice(&0u32.to_le_bytes());

        for (what, image) in [
            ("bare", volume.clone()),
            ("wrapped, honest length", honest),
            ("wrapped, zero length", zero),
            ("wrapped, zero length and no block count", no_blocks),
        ] {
            assert!(ProDos::looks_like_prodos(&image), "{what}");
            let fs = ProDos::mount(image).unwrap_or_else(|e| panic!("{what}: {e:?}"));
            assert_eq!(fs.volume_name(), "TEST.DISK", "{what}");
            assert_eq!(fs.files().len(), 1, "{what}");
            assert_eq!(fs.story().expect("a story").0, "STORY", "{what}");
        }
    }

    /// A 2IMG header that lies about where its data is describes no volume, and
    /// the bare placement must not rescue it — the two are 64 bytes apart, which
    /// is exactly what the geometry check catches.
    #[test]
    fn a_2img_header_pointing_nowhere_is_not_a_volume() {
        let mut b = VolumeBuilder::new("TEST.DISK");
        b.add_file("STORY", &fake_story(4096));
        let wrapped = two_img(&b.finish(), 0);
        let mut bent = wrapped.clone();
        bent[TWO_IMG_DATA_OFFSET..TWO_IMG_DATA_OFFSET + 4]
            .copy_from_slice(&(TWO_IMG_MIN_HEADER as u32 + 4).to_le_bytes());
        assert!(!ProDos::looks_like_prodos(&bent), "the volume is not four bytes further on");
    }

    /// Seedlings, saplings and trees — one file of each, round-tripped. Nothing
    /// smaller than a tree exercises the master index block, and the corpus's
    /// biggest stories (*Beyond Zork*, *Trinity*) are trees.
    #[test]
    fn round_trips_files_of_every_storage_type() {
        let mut b = VolumeBuilder::new("TEST.DISK");
        let seedling = b"hello from 1988".to_vec();
        let sapling: Vec<u8> = (0..40 * BLOCK + 7).map(|i| (i % 251) as u8).collect();
        // Over 256 blocks, so it needs a master index block.
        let tree: Vec<u8> = (0..300 * BLOCK + 11).map(|i| (i % 241) as u8).collect();
        b.add_file("SEEDLING", &seedling);
        b.add_file("SAPLING", &sapling);
        b.add_file("TREE", &tree);
        let fs = ProDos::mount(b.finish()).expect("mounts");

        let names: Vec<String> = fs.files().iter().map(ProDosEntry::path).collect();
        assert_eq!(names, ["SEEDLING", "SAPLING", "TREE"]);
        assert_eq!(fs.read_named("seedling").as_deref(), Some(&seedling[..]), "case-insensitive");
        assert_eq!(fs.read_named("SAPLING").as_deref(), Some(&sapling[..]));
        assert_eq!(fs.read_named("TREE").as_deref(), Some(&tree[..]), "300 blocks, two index blocks");
        assert_eq!(fs.read_named("ABSENT"), None);
    }

    /// A sparse file is a hole ProDOS never allocated, not a short read — a zero
    /// pointer means 512 zero bytes.
    #[test]
    fn a_sparse_file_reads_its_holes_as_zeroes() {
        let mut b = VolumeBuilder::new("TEST.DISK");
        let mut data: Vec<u8> = (0..20 * BLOCK).map(|i| (i % 251) as u8).collect();
        // Blank every fourth block so the builder leaves a zero pointer there.
        for i in (0..20).step_by(4) {
            data[i * BLOCK..(i + 1) * BLOCK].fill(0);
        }
        b.add_file_typed("HOLEY", 0xf5, &data, 4);
        let fs = ProDos::mount(b.finish()).expect("mounts");
        assert_eq!(fs.read_named("HOLEY").as_deref(), Some(&data[..]), "the holes are zeroes");
    }

    /// GS/OS forked files: the entry's own EOF describes the extended key block,
    /// and the data fork's mini-entry is what knows the size. Reading the entry's
    /// 512 would truncate every one of them.
    #[test]
    fn an_extended_file_reports_and_reads_its_data_fork() {
        let mut b = VolumeBuilder::new("TEST.DISK");
        let data: Vec<u8> = (0..3 * BLOCK + 5).map(|i| (i % 253) as u8).collect();
        let rsrc: Vec<u8> = (0..2 * BLOCK).map(|i| (i % 239) as u8).collect();
        b.add_extended_file("FORKED", &data, &rsrc);
        let fs = ProDos::mount(b.finish()).expect("mounts");
        assert_eq!(fs.files()[0].size, data.len(), "the DATA fork's length, not the key block's");
        assert_eq!(fs.read_named("FORKED").as_deref(), Some(&data[..]));
    }

    /// Subdirectories are real and must be walked: the GS/OS disks put most of
    /// their files under `SYSTEM/`, and the header entry of a subdirectory is not
    /// a file.
    #[test]
    fn subdirectories_are_walked_and_the_path_names_the_file() {
        let mut b = VolumeBuilder::new("TEST.DISK");
        b.add_dir("SYSTEM", &[("GS.OS", b"\x00\x01\x02"), ("ERROR.MSG", b"oops")]);
        b.add_file("BZ.DAT", &fake_story(4096));
        let fs = ProDos::mount(b.finish()).expect("mounts");
        let paths: Vec<String> = fs.files().iter().map(ProDosEntry::path).collect();
        assert_eq!(
            paths,
            ["SYSTEM/GS.OS", "SYSTEM/ERROR.MSG", "BZ.DAT"],
            "the subdirectory header is not a file"
        );
        assert_eq!(fs.read_named("SYSTEM/ERROR.MSG").as_deref(), Some(&b"oops"[..]));
        assert_eq!(fs.read_named("gs.os").map(|b| b.len()), Some(3), "the bare name resolves too");
    }

    /// A volume directory longer than one block: thirteen entries per block and
    /// a `next` pointer, which every disk in the corpus uses.
    #[test]
    fn a_volume_directory_that_spans_several_blocks_is_followed() {
        let mut b = VolumeBuilder::new("TEST.DISK");
        for i in 0..20 {
            b.add_file(&format!("FILE.{i:02}"), format!("contents {i}").as_bytes());
        }
        let fs = ProDos::mount(b.finish()).expect("mounts");
        assert_eq!(fs.files().len(), 20, "twelve in the key block, the rest in the next");
        assert_eq!(fs.read_named("FILE.19").as_deref(), Some(&b"contents 19"[..]));
    }

    #[test]
    fn finds_a_story_by_content_not_by_name_or_file_type() {
        let mut b = VolumeBuilder::new("TEST.DISK");
        // Type $B3 is a GS/OS application and $F8 is what these disks call a
        // saved game; neither decides anything.
        b.add_file_typed("BZ.SYS16", 0xb3, b"\x00\x00\x00\x00", 0);
        b.add_file_typed("SAVE", 0xf8, &vec![0x03u8; 19456], 0);
        b.add_file_typed("BZ.DAT", 0xaf, &fake_story(4096), 0);
        let fs = ProDos::mount(b.finish()).expect("mounts");
        let (path, bytes) = fs.story().expect("the story is found under any type");
        assert_eq!(path, "BZ.DAT");
        assert_eq!(bytes.len(), 4096);
    }

    /// No conventional name to break the tie on this format, so the largest
    /// candidate wins and the answer is deterministic.
    #[test]
    fn a_compilation_offers_every_game_and_opens_the_largest() {
        let mut b = VolumeBuilder::new("INFOCOM2");
        b.add_file("ZORK.I", &fake_story(4096));
        b.add_file("BEYOND.ZORK", &fake_story(16384));
        b.add_file("ZORK.II", &fake_story(8192));
        let fs = ProDos::mount(b.finish()).expect("mounts");
        let stories: Vec<String> = fs
            .files()
            .iter()
            .filter(|e| fs.read(e).is_some_and(|b| looks_like_story(&b)))
            .map(ProDosEntry::path)
            .collect();
        assert_eq!(stories, ["ZORK.I", "BEYOND.ZORK", "ZORK.II"], "in disk order");
        assert_eq!(fs.story().expect("a story").0, "BEYOND.ZORK", "the largest opens");
    }

    /// A launcher disk carries files and no game — *Lost Treasures* volume 1 is
    /// exactly this. Saying so is what lets a caller ask "is this the right
    /// disk?" instead of reporting a corrupt story.
    #[test]
    fn a_disk_with_no_game_mounts_and_offers_nothing() {
        let mut b = VolumeBuilder::new("INFOCOM1");
        b.add_dir("SYSTEM", &[("GS.OS", &vec![0x11u8; 4000]), ("START.GS.OS", &vec![0x22u8; 900])]);
        b.add_file_typed("PRODOS", 0xff, &vec![0x33u8; 1481], 0);
        let fs = ProDos::mount(b.finish()).expect("a launcher disk still mounts");
        assert_eq!(fs.files().len(), 3);
        assert_eq!(fs.story(), None);
        assert!(fs.pictures().is_none());
    }

    /// A directory chained into a cycle is a damaged disk, not a hang.
    #[test]
    fn a_cyclic_directory_chain_terminates() {
        let mut b = VolumeBuilder::new("TEST.DISK");
        b.add_file("STORY", &fake_story(4096));
        let mut volume = b.finish();
        let key = usize::from(VOLUME_DIR_BLOCK) * BLOCK;
        volume[key + DIR_NEXT..key + DIR_NEXT + 2]
            .copy_from_slice(&VOLUME_DIR_BLOCK.to_le_bytes());
        let fs = ProDos::mount(volume).expect("mounts");
        assert_eq!(fs.files().len(), 1, "the key block is not walked twice");
    }

    // ── Real media ───────────────────────────────────────────────────────────

    /// The user's `stories/` directory, which is gitignored — every test over it
    /// skips vacuously.
    fn stories_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories")
    }

    fn read_fixture(name: &str) -> Option<Vec<u8>> {
        let path = stories_dir().join(name);
        match std::fs::read(&path) {
            Ok(b) => Some(b),
            Err(_) => {
                eprintln!("SKIP: gitignored medium missing at {}", path.display());
                None
            }
        }
    }

    /// The seven *Lost Treasures* volumes, by their filename in `stories/`.
    fn lost_treasures(n: usize) -> String {
        format!(
            "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk {n} of 7).2mg"
        )
    }

    /// `Beyond Zork (1988)(Infocom).2mg` — the standalone Apple IIgs release.
    /// One game on a GS/OS boot disk, so it is the format's simplest real case.
    #[test]
    fn real_apple_iigs_single_game_disk() {
        let Some(bytes) = read_fixture("Beyond Zork (1988)(Infocom).2mg") else { return };
        assert_eq!(bytes.len(), 819_264, "800 KB plus a 64-byte 2IMG header");
        assert_eq!(&bytes[..4], TWO_IMG_MAGIC);
        assert_eq!(&bytes[4..8], b"WOOF", "CiderPress wrote it");
        assert_eq!(le32(&bytes, TWO_IMG_DATA_LENGTH), 0, "…and left the length field zero");
        assert!(ProDos::looks_like_prodos(&bytes));

        let fs = ProDos::mount(bytes).expect("the disk mounts");
        assert_eq!(fs.volume_name(), "BEYOND.ZORK");
        assert_eq!(fs.files().len(), 26);
        // The machine, out of the disk's own contents: GS/OS and a `SYS16`
        // application are Apple IIgs and nothing else.
        for gs in ["SYSTEM/GS.OS", "SYSTEM/START.GS.OS", "BZ.SYS16"] {
            assert!(fs.read_named(gs).is_some(), "{gs} is on the disk");
        }
        // Two levels of directory, which a root-only walk would miss entirely.
        assert_eq!(
            fs.read_named("SYSTEM/SYSTEM.SETUP/TOOL.SETUP").map(|b| b.len()),
            Some(33_956)
        );

        let (name, story) = fs.story().expect("Beyond Zork is on this disk");
        assert_eq!(name, "BZ.DAT");
        assert_eq!(story.len(), 261_388);
        assert_eq!((story[0], u16::from_be_bytes([story[2], story[3]])), (5, 57));
        assert_eq!(&story[0x12..0x18], b"871221");
        // A ProDOS release carries no Infocom picture archive; the question is
        // still answerable.
        assert!(fs.pictures().is_none());
    }

    /// **`Arthur` and `Journey` carry no whole story file**, and that is the
    /// finding rather than a gap in the reader.
    ///
    /// Both are the ProDOS **8** Apple II press — `INFOCOM.SYSTEM` beside
    /// `BASIC.SYSTEM` — and the game is split across `ARTHUR.D1`…`D5` /
    /// `JOURNEY.D1`…`D4`, none of which begins with a Z-machine header. So no
    /// FILE on either disk is a story, which is what this case pins.
    ///
    /// What comes back differs, and SQ-0852 is the difference: *Arthur*'s five
    /// segments are all here and reassemble into release 63 (see
    /// [`ProDos::packed_story`] and [`crate::infocom_packed`], where that is
    /// measured), while `Journey.2mg` declares five segments and carries four,
    /// so 92 of its 552 pages are not on the image and it still answers `None`.
    /// **Two disks that look identical to every per-file test, and only one
    /// holds a game.** The artwork splits the same way and for the same reason
    /// (SQ-0863): *Arthur* yields 168 pictures merged out of four floppies,
    /// *Journey* yields none, because the segment it is missing carries a
    /// quarter of them.
    #[test]
    fn real_apple_ii_segmented_releases_hold_no_story_file() {
        for (fixture, volume, files, segments, packed) in [
            ("Arthur Quest 4 Excalibur.2mg", "ARTHUR.3.5", 11, &["ARTHUR.1/ARTHUR.D1"][..], true),
            ("Journey.2mg", "JOURNEY", 9, &["JOURNEY.D1", "JOURNEY.D4"][..], false),
        ] {
            let Some(bytes) = read_fixture(fixture) else { continue };
            assert!(ProDos::looks_like_prodos(&bytes), "{fixture}");
            let fs = ProDos::mount(bytes).expect("the disk mounts");
            assert_eq!(fs.volume_name(), volume, "{fixture}");
            assert_eq!(fs.files().len(), files, "{fixture}");
            assert!(
                fs.read_named("INFOCOM.SYSTEM").is_some(),
                "{fixture}: the ProDOS 8 interpreter, not a GS/OS one"
            );
            for segment in segments {
                let seg = fs.read_named(segment).unwrap_or_else(|| panic!("{fixture}: {segment}"));
                assert!(
                    !looks_like_story(&seg),
                    "{fixture}: {segment} is a segment of a container, not a story image"
                );
            }
            assert!(
                fs.files().iter().filter_map(|e| fs.read(e)).all(|b| !looks_like_story(&b)),
                "{fixture}: no file on this volume is a story"
            );
            assert_eq!(
                fs.packed_story().is_some(),
                packed,
                "{fixture}: the packed volume reassembles = {packed}"
            );
            // `story()` therefore answers with the packed volume or with nothing,
            // and never with a file.
            assert_eq!(fs.story().is_some(), packed, "{fixture}");
            if packed {
                let (name, story) = fs.story().expect("Arthur reassembles");
                assert_eq!(name, "ARTHUR.1/ARTHUR.D1");
                assert_eq!((story[0], u16::from_be_bytes([story[2], story[3]])), (6, 63));
                assert_eq!(&story[0x12..0x18], b"890622");
            }
            // The packed door reads the artwork, and follows the story exactly
            // (SQ-0863): *Arthur*'s four archives are all here; *Journey*'s
            // fifth segment is missing, so its set is refused whole rather than
            // served short.
            assert_eq!(fs.packed_pictures().is_some(), packed, "{fixture}");
            // No FILE on either volume is an archive, so `pictures()` is the
            // packed door's answer and nothing else — the fall-through SQ-0863
            // opened, and the only reason either disk draws.
            assert_eq!(
                fs.pictures().map(|(n, p)| (n, p.entries().len())),
                fs.packed_pictures().map(|(n, p)| (n, p.entries().len())),
                "{fixture}: no FILE here is an archive, so the packed door is the answer"
            );
            if packed {
                let (name, pics) = fs.packed_pictures().expect("Arthur's artwork");
                assert_eq!(name, "ARTHUR.1/ARTHUR.D1");
                assert_eq!(pics.entries().len(), 168);
                assert_eq!(pics.parts(), 4, "one archive per floppy, disks 2..=5");
                // 140×192 is the Apple's picture space and it is what moves the
                // story's screen; see `ProDos::pictures`.
                assert_eq!((pics.picture_space_width(), pics.picture_space_height()), (140, 192));
            }
        }
    }

    /// **The whole *Lost Treasures* set, volume by volume.** Seven Apple IIgs
    /// disks: one GS/OS launcher with no game on it at all, and **thirty** games
    /// across the other six.
    ///
    /// The point of listing every game rather than only the one `story()` opens
    /// is the requirement itself — a compilation is a LIST, and a reader that
    /// found the first game and stopped would pass a single-story test.
    /// One game as the header records it: name, version, release, serial —
    /// with **bit 7 masked off** each serial byte, because one of them is written
    /// in the Apple II's high ASCII and there is no other way to read it (see
    /// `INFOCOM6` below). Every other serial in the set has bit 7 clear, so the
    /// mask changes nothing anywhere else.
    type Game = (&'static str, u8, u16, &'static str);
    /// One volume: its ProDOS name, the files on it, and the games in disk order.
    type Volume = (&'static str, usize, &'static [Game]);

    #[test]
    fn real_lost_treasures_volumes_list_every_game_on_them() {
        let expected: [Volume; 7] = [
            ("INFOCOM1", 53, &[]),
            ("INFOCOM2", 10, &[
                ("ZORK.III", 3, 17, "840727"),
                ("ZORK.II", 3, 48, "840904"),
                ("ZORK.I", 3, 88, "840726"),
                ("HITCHHIKER", 5, 31, "871119"),
                ("BEYOND.ZORK", 5, 57, "871221"),
            ]),
            ("INFOCOM3", 8, &[
                ("STATIONFALL", 3, 107, "870430"),
                ("STARCROSS", 3, 17, "821021"),
                ("SPELLBREAKER", 3, 87, "860904"),
                ("SORCERER", 3, 15, "851108"),
                ("PLANETFALL", 3, 37, "851003"),
                ("MOONMIST", 3, 9, "861022"),
                ("ENCHANTER", 3, 29, "860820"),
            ]),
            ("INFOCOM4", 8, &[
                ("WITNESS", 3, 22, "840924"),
                ("SUSPENDED", 3, 8, "840521"),
                ("SUSPECT", 3, 14, "841005"),
                ("INFIDEL", 3, 22, "830916"),
                ("HORROR", 3, 203, "870506"),
                ("DEADLINE", 3, 27, "831005"),
                ("BALLYHOO", 3, 97, "851218"),
            ]),
            ("INFOCOM5", 4, &[
                ("AMINDFV", 4, 77, "850814"),
                ("TRINITY", 4, 12, "860926"),
                ("BUREAUCRACY", 4, 116, "870602"),
            ]),
            // **`LEATHRGODDESSES` is the reason this table masks bit 7**, and
            // it was missing from this list until SQ-0856. Its header is a
            // structurally valid v3 story — release 0, declared length
            // `0xfbf3 * 2` == its 128998 bytes exactly — with a serial of
            // `C2 EC EF F7 EE A1`, which is `42 6C 6F 77 6E 21` once bit 7 comes
            // off: **"Blown!"**, a joke serial typed on a machine whose character
            // set sets the high bit. `looks_like_story` demanded bit 7 clear, so
            // Leather Goddesses of Phobos was simply invisible on this volume and
            // it listed four games where it holds five. The serial check still
            // does the job it exists for — a saved game's `$12..$18` is binary,
            // and binary is control bytes — it just no longer mistakes the Apple
            // II's own text encoding for corruption.
            ("INFOCOM6", 6, &[
                ("SHERLOCK", 5, 21, "871214"),
                ("BORDERZONE", 5, 9, "871008"),
                ("NORDANDBERT", 4, 19, "870722"),
                ("LEATHRGODDESSES", 3, 0, "Blown!"),
                ("PLUNDERED", 3, 26, "870730"),
            ]),
            ("INFOCOM7", 4, &[
                ("HOLLYWOOD", 3, 37, "861215"),
                ("CUTTTHROAT", 3, 23, "840809"),
                ("WISHBRINGER", 3, 69, "850920"),
            ]),
        ];
        let mut ran = 0;
        for (n, (volume, files, stories)) in expected.iter().enumerate() {
            let Some(bytes) = read_fixture(&lost_treasures(n + 1)) else { continue };
            ran += 1;
            assert!(ProDos::looks_like_prodos(&bytes), "{volume}");
            let fs = ProDos::mount(bytes).expect("the volume mounts");
            assert_eq!(fs.volume_name(), *volume);
            assert_eq!(fs.files().len(), *files, "{volume}: files on the disk");

            let found: Vec<(String, u8, u16, String)> = fs
                .files()
                .iter()
                .filter_map(|e| fs.read(e).map(|b| (e.path(), b)))
                .filter(|(_, b)| looks_like_story(b))
                .map(|(p, b)| {
                    (
                        p,
                        b[0],
                        u16::from_be_bytes([b[2], b[3]]),
                        b[0x12..0x18].iter().map(|c| char::from(c & 0x7f)).collect(),
                    )
                })
                .collect();
            let want: Vec<(String, u8, u16, String)> = stories
                .iter()
                .map(|(n, v, r, s)| (n.to_string(), *v, *r, s.to_string()))
                .collect();
            assert_eq!(found, want, "{volume}: the games on it, in disk order");

            // The saved games sitting beside them are never offered as games.
            assert!(
                !found.iter().any(|(p, ..)| p.ends_with(".SAV")),
                "{volume}: a saved game was offered as a story"
            );
        }
        assert!(
            ran > 0 || !stories_dir().join(lost_treasures(1)).exists(),
            "the media are present but none were read"
        );
    }

    /// Volume 1 is the launcher: fifty-three files of GS/OS and not one game.
    /// The guard for `MountError::Unreadable` being reported instead of "corrupt
    /// story file" is only worth having if a real disk exercises it.
    #[test]
    fn the_real_launcher_volume_mounts_with_no_game_on_it() {
        let Some(bytes) = read_fixture(&lost_treasures(1)) else { return };
        let fs = ProDos::mount(bytes).expect("the launcher mounts");
        assert_eq!(fs.volume_name(), "INFOCOM1");
        assert_eq!(fs.story(), None, "no game is on this disk");
        // Its GS/OS forked files read through their data-fork mini-entries, and
        // the entry's own 512-byte EOF is not what any of them is.
        let forked = fs
            .files()
            .iter()
            .filter(|e| e.storage == ST_EXTENDED)
            .map(|e| (e.path(), e.size))
            .collect::<Vec<_>>();
        assert_eq!(
            forked,
            vec![
                ("SYSTEM/CDEVS/PRINTER".to_string(), 0),
                ("SYSTEM/CDEVS/TIME".to_string(), 0),
                ("SYSTEM/DESK.ACCS/CONTROLPANEL".to_string(), 16_071),
                ("SYSTEM/SYSTEM.SETUP/SYS.RESOURCES".to_string(), 0),
                ("LOST2.SYS16".to_string(), 69_045),
                ("LOST1.SYS16".to_string(), 69_256),
                ("ICONS/INFOCOM.ICONS".to_string(), 872),
            ],
            "every extended file's DATA fork length"
        );
    }

    /// Every name a real ProDOS volume lists is a name it will read back — the
    /// property `blorb::medium` pins across formats, on the disk with the deepest
    /// directory tree in the corpus.
    ///
    /// FALSIFICATION: collapse [`ProDos::read_named`]'s two passes back into one
    /// `path == name || name == name` predicate and this fails on `FINDER.DATA`,
    /// the root file whose two namesakes under `SYSTEM/FONTS` and `ICONS` sit
    /// earlier in the walk.
    #[test]
    fn a_real_volume_reads_back_every_path_it_lists() {
        let Some(bytes) = read_fixture(&lost_treasures(1)) else { return };
        let fs = ProDos::mount(bytes).expect("mounts");
        // Three files of one name, and only the path tells them apart.
        let namesakes: Vec<String> = fs
            .files()
            .iter()
            .filter(|e| e.name == "FINDER.DATA")
            .map(ProDosEntry::path)
            .collect();
        assert_eq!(namesakes, ["SYSTEM/FONTS/FINDER.DATA", "ICONS/FINDER.DATA", "FINDER.DATA"]);
        for e in fs.files() {
            let path = e.path();
            let bytes = fs.read(e).unwrap_or_else(|| panic!("{path}: listed and would not read"));
            assert_eq!(bytes.len(), e.size, "{path}: the entry's own length");
            assert_eq!(fs.read_named(&path).as_deref(), Some(&bytes[..]), "{path}");
            assert_eq!(
                fs.read_named(&path.to_ascii_lowercase()).as_deref(),
                Some(&bytes[..]),
                "{path}: case-sensitive"
            );
        }
    }

    /// **Every ProDOS image in the corpus opens**, and nothing else in
    /// `stories/` is claimed as one. The sniff has to stay disjoint from the four
    /// formats already in the table, and a directory full of Amiga, Macintosh,
    /// DOS and Atari ST floppies beside bare story files is the strongest
    /// available statement of that.
    ///
    /// **All three spellings, since SQ-0863.** Fourteen `.dsk` images are ProDOS
    /// volumes too — 5.25-inch dumps whose sectors are in the drive's order
    /// (SQ-0864) — and three `.po` images are BARE volumes with no wrapper at
    /// all. Every one of them opens here.
    ///
    /// # Why this is not written as "claimed if and only if it is spelled right"
    ///
    /// It used to be, and the corpus refuted it twice in one afternoon. The
    /// spelling was never the claim — the module's rule is that recognition is by
    /// CONTENT — and a directory that happened to contain only well-named files
    /// let an extension test masquerade as a content test for two quests. So the
    /// two directions are stated separately, and only one of them mentions a
    /// name:
    ///
    /// - **Disjointness**, swept over everything: anything claimed here must be
    ///   a ProDOS volume that mounts, and must not wear another format's
    ///   spelling. That is what keeps the sniff off the Amiga, Macintosh, DOS and
    ///   Atari ST floppies sitting beside these.
    /// - **Coverage**, from [`PRODOS_MEDIA`]: every image we say is a ProDOS
    ///   volume is claimed and opens. A reader that quietly stopped recognising
    ///   the 5.25-inch press fails here rather than passing an emptier sweep.
    ///
    /// The two files that make the difference are worth naming, and one of them
    /// changed sides in SQ-0889. `Shogun.po` is a DiskCopy 4.2 image wearing a
    /// ProDOS extension; it used to be declined here and that decline was this
    /// case's headline claim. It is claimed now — not because of the name, which
    /// is still not evidence, but because the wrapper is unwrapped and there is
    /// an ordinary 800 KB `SHOGUN` volume 84 bytes in. `Planetfall r29 (clean
    /// copy from retail disk).dsk` is the half that has not moved: a
    /// 143,360-byte Apple II 5.25-inch dump that is **DOS 3.3**, which
    /// [`dos_ordered`] re-orders as willingly as any other and then declines,
    /// because nothing but a ProDOS volume directory is allowed to make this
    /// answer `Some`.
    #[test]
    fn only_the_prodos_images_in_the_corpus_look_like_prodos() {
        let Ok(dir) = std::fs::read_dir(stories_dir()) else {
            eprintln!("SKIP: no stories directory");
            return;
        };
        // Direction one: whatever is claimed really is one, and wears one of the
        // three spellings the census names.
        let mut seen = 0;
        for entry in dir.flatten() {
            let path = entry.path();
            let Ok(raw) = std::fs::read(&path) else { continue };
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if !ProDos::looks_like_prodos(&raw) {
                continue;
            }
            seen += 1;
            let lower = name.to_ascii_lowercase();
            assert!(
                [".2mg", ".dsk", ".po"].iter().any(|e| lower.ends_with(e)),
                "{name}: claimed as ProDOS under a spelling no format row lists"
            );
            let fs = ProDos::mount(raw).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert!(!fs.files().is_empty(), "{name}: mounted but empty");
            assert!(!fs.volume_name().is_empty(), "{name}: no volume name");
        }

        // Direction two: every image we NAME as one is claimed, at the geometry
        // its wrapper implies.
        let (mut five_and_a_quarter, mut bare, mut diskcopy, mut ran) = (0, 0, 0, 0);
        for (name, kind) in PRODOS_MEDIA {
            let Ok(raw) = std::fs::read(stories_dir().join(name)) else { continue };
            ran += 1;
            assert!(ProDos::looks_like_prodos(&raw), "{name}: must be claimed as ProDOS");
            match kind {
                ProDosMedium::FiveAndAQuarter => {
                    five_and_a_quarter += 1;
                    assert_eq!(raw.len(), crate::dos_order::DOS_ORDER_LEN, "{name}: 35 × 16 × 256");
                }
                ProDosMedium::Bare => {
                    bare += 1;
                    // A bare volume is the whole file and nothing else — 1600
                    // blocks of 3.5-inch media, with no header to skip.
                    assert_eq!(raw.len(), 1600 * BLOCK, "{name}: 1600 × 512, bare");
                }
                ProDosMedium::DiskCopy => {
                    diskcopy += 1;
                    // The same 1600 blocks, plus the header in front and the
                    // sector tags behind — the wrapper accounts for every byte.
                    assert_eq!(raw.len(), 84 + 1600 * BLOCK + 19_200, "{name}: wrapped 800 KB");
                }
                ProDosMedium::TwoImg => {}
            }
        }
        assert_eq!(ran, seen, "an image is claimed that PRODOS_MEDIA does not name");
        assert!(
            five_and_a_quarter == 14 || ran == 0,
            "expected fourteen 5.25-inch volumes, got {five_and_a_quarter}"
        );
        assert!(bare == 3 || ran == 0, "expected three bare 3.5-inch volumes, got {bare}");
        assert!(diskcopy == 1 || ran == 0, "expected one DiskCopy 3.5-inch volume, got {diskcopy}");
        if ran == 0 {
            eprintln!("SKIP: no ProDOS media present");
        }
    }

    /// How a ProDOS volume in the corpus is packaged: what has to be got past
    /// before the volume directory is where this reader looks for it.
    enum ProDosMedium {
        /// A 2IMG header bolted onto an 800 KB volume.
        TwoImg,
        /// A 143,360-byte 5.25-inch dump in the drive's sector order.
        FiveAndAQuarter,
        /// An 800 KB volume and nothing else.
        Bare,
        /// An 800 KB volume behind an 84-byte DiskCopy 4.2 header, with the
        /// sector tags trailing it (SQ-0889).
        DiskCopy,
    }

    /// Every ProDOS image in `stories/`, named. See the case above for why the
    /// coverage half of that test reads a list rather than a directory.
    const PRODOS_MEDIA: &[(&str, ProDosMedium)] = &[
        ("Beyond Zork (1988)(Infocom).2mg", ProDosMedium::TwoImg),
        ("Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 1 of 7).2mg", ProDosMedium::TwoImg),
        ("Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 2 of 7).2mg", ProDosMedium::TwoImg),
        ("Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 3 of 7).2mg", ProDosMedium::TwoImg),
        ("Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 4 of 7).2mg", ProDosMedium::TwoImg),
        ("Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 5 of 7).2mg", ProDosMedium::TwoImg),
        ("Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 6 of 7).2mg", ProDosMedium::TwoImg),
        ("Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 7 of 7).2mg", ProDosMedium::TwoImg),
        ("Arthur Quest 4 Excalibur.2mg", ProDosMedium::TwoImg),
        ("Journey.2mg", ProDosMedium::TwoImg),
        ("Arthur.po", ProDosMedium::Bare),
        ("Journey.po", ProDosMedium::Bare),
        ("ZorkZero.po", ProDosMedium::Bare),
        ("Shogun.po", ProDosMedium::DiskCopy),
        ("shogun_s1.dsk", ProDosMedium::FiveAndAQuarter),
        ("shogun_s2.dsk", ProDosMedium::FiveAndAQuarter),
        ("shogun_s3.dsk", ProDosMedium::FiveAndAQuarter),
        ("shogun_s4.dsk", ProDosMedium::FiveAndAQuarter),
        ("shogun_s5.dsk", ProDosMedium::FiveAndAQuarter),
        ("zork_zero_1.dsk", ProDosMedium::FiveAndAQuarter),
        ("zork_zero_2.dsk", ProDosMedium::FiveAndAQuarter),
        ("zork_zero_3.dsk", ProDosMedium::FiveAndAQuarter),
        ("zork_zero_4.dsk", ProDosMedium::FiveAndAQuarter),
        ("journey_s1.dsk", ProDosMedium::FiveAndAQuarter),
        ("journey_s2.dsk", ProDosMedium::FiveAndAQuarter),
        ("journey_s3.dsk", ProDosMedium::FiveAndAQuarter),
        ("journey_s4.dsk", ProDosMedium::FiveAndAQuarter),
        ("journey_s5.dsk", ProDosMedium::FiveAndAQuarter),
    ];

    /// **A file in the corpus whose NAME says ProDOS and whose bytes do not**
    /// (SQ-0863).
    ///
    /// The module's rule is that recognition is by content; a corpus of
    /// well-named files cannot demonstrate that, and this one can. It used to
    /// have a companion — `Shogun.po`, declined for wearing a `.po` over a
    /// DiskCopy 4.2 wrapper — and SQ-0889 took that companion away by unwrapping
    /// the wrapper. The claim survives the loss intact, because the reason the
    /// two were declined was never the same reason: `Shogun.po` was declined for
    /// a placement this reader had not been taught, and holds a real ProDOS
    /// volume; this file is declined because there is no ProDOS volume in it at
    /// any placement or ordering. See
    /// [`a_diskcopy_wrapper_is_unwrapped_like_any_other`] for where the other
    /// half went.
    ///
    /// An Apple II 5.25-inch dump that is DOS 3.3 rather than ProDOS:
    /// [`dos_ordered`] re-orders it as willingly as any other and then declines
    /// it, which is the de-interleave being a wrapper and never a verdict.
    #[test]
    fn a_prodos_spelling_is_not_a_prodos_volume() {
        let planetfall = "Planetfall r29 (clean copy from retail disk).dsk";
        let Ok(raw) = std::fs::read(stories_dir().join(planetfall)) else {
            eprintln!("SKIP: no {planetfall}");
            return;
        };
        assert_eq!(raw.len(), crate::dos_order::DOS_ORDER_LEN, "the 5.25-inch geometry");
        assert!(crate::dos_order::prodos_order(&raw).is_some(), "it re-orders willingly");
        assert!(!ProDos::looks_like_prodos(&raw), "and holds no ProDOS volume directory");
    }

    /// **`Shogun.po` is a DiskCopy 4.2 image and it mounts** (SQ-0889).
    ///
    /// The inverse of what this module asserted for two quests. Nothing about
    /// the file changed; what changed is that [`volume_at`] now tries the
    /// wrapper's placement, borrowing the unwrap from [`crate::hfs`] rather than
    /// growing a second one.
    ///
    /// Every number below was measured, and the arithmetic is what makes the
    /// placement safe to trust rather than a lucky offset: the header declares
    /// `dataSize` and `tagSize`, and 84 + `dataSize` + `tagSize` is the file
    /// length to the byte. What comes out is the **Apple II press, release 311 /
    /// serial 890510, checksum `$E200`** — the same release the five-volume
    /// 5.25-inch set `shogun_s1.dsk`…`s5` reassembles to, which is the outside
    /// evidence that the unwrap landed on the right bytes rather than on
    /// something merely well-formed. The story's own header agrees: its declared
    /// length is exactly the bytes that came out, and the checksum over
    /// `$40..len` is the one the header carries.
    #[test]
    fn a_diskcopy_wrapper_is_unwrapped_like_any_other() {
        let Ok(raw) = std::fs::read(stories_dir().join("Shogun.po")) else {
            eprintln!("SKIP: no Shogun.po");
            return;
        };
        // The premise: a DiskCopy 4.2 name field, not a ProDOS boot block.
        assert_eq!(&raw[..7], b"\x06SHOGUN", "the DiskCopy 4.2 name field");
        assert_eq!(&raw[0x52..0x54], &[0x01, 0x00], "the DiskCopy 4.2 magic at +$52");
        let data = u32::from_be_bytes(raw[0x40..0x44].try_into().unwrap()) as usize;
        let tag = u32::from_be_bytes(raw[0x44..0x48].try_into().unwrap()) as usize;
        assert_eq!(data, 1600 * BLOCK, "dataSize: an 800 KB volume");
        assert_eq!(tag, 19_200, "tagSize: the sector tags that follow it");
        assert_eq!(84 + data + tag, raw.len(), "the wrapper accounts for every byte");

        assert!(ProDos::looks_like_prodos(&raw), "a wrapped volume is still a volume");
        let fs = ProDos::mount(raw).expect("Shogun.po mounts");
        assert_eq!(fs.volume_name(), "SHOGUN");
        // The segmented Apple II press, as on the 5.25-inch set: five segments
        // and the ProDOS 8 launcher beside them, no whole story file.
        for seg in ["SHOGUN.D1", "SHOGUN.D2", "SHOGUN.D3", "SHOGUN.D4", "SHOGUN.D5"] {
            assert!(fs.files().iter().any(|f| f.path() == seg), "{seg} missing");
        }
        let (name, story) = fs.story().expect("the segments reassemble into a story");
        assert_eq!(name, "SHOGUN.D1", "the reassembly is named for its first segment");
        assert_eq!(story[0], 6, "Version 6");
        assert_eq!(u16::from_be_bytes([story[2], story[3]]), 311, "release 311");
        assert_eq!(&story[0x12..0x18], b"890510", "serial 890510");
        assert_eq!(u16::from_be_bytes([story[0x1c], story[0x1d]]), 0xe200, "checksum $E200");
        // ZMSD §11.1: `$1a` is the file length in words, and Version 6 scales it
        // by 8. It has to be the bytes actually in hand.
        let declared = usize::from(u16::from_be_bytes([story[0x1a], story[0x1b]])) * 8;
        assert_eq!(declared, story.len(), "the header's own length matches the reassembly");
        let sum: u32 = story[0x40..].iter().map(|b| u32::from(*b)).sum();
        assert_eq!(sum & 0xffff, 0xe200, "and the bytes check out against it");
        assert!(fs.pictures().is_some(), "the Apple artwork comes off the same disk");
    }

    /// **The 5.25-inch press, volume by volume** (SQ-0864).
    ///
    /// SQ-0852 read these images as bare packed volumes with no filesystem —
    /// "there is no ProDOS volume directory on any of them". There is one on
    /// every one of them; it is 1,024 bytes into the image only once the sectors
    /// are in ProDOS's order rather than the drive's. Each volume names itself
    /// and carries its segment as an ordinary ProDOS file, which is what makes
    /// [`crate::infocom_packed`]'s pairing-by-basename work unchanged.
    ///
    /// FALSIFICATION: reverse any two entries of `dos_order::SECTOR_OF` and no
    /// image here mounts at all — the volume directory is simply not there.
    #[test]
    fn the_five_and_a_quarter_inch_volumes_name_themselves_and_their_segments() {
        // (image, volume name, every file on it in directory order)
        let expected: &[(&str, &str, &[&str])] = &[
            ("shogun_s1.dsk", "SHOGUN.1", &[
                "INFOCOM",
                "SHOGUN.D1",
                "INFOCOM.SYSTEM",
                "INFODOS",
            ]),
            ("shogun_s2.dsk", "SHOGUN.2", &["SHOGUN.D2"]),
            ("shogun_s3.dsk", "SHOGUN.3", &["SHOGUN.D3"]),
            ("shogun_s4.dsk", "SHOGUN.4", &["SHOGUN.D4"]),
            ("shogun_s5.dsk", "SHOGUN.5", &["SHOGUN.D5"]),
            ("zork_zero_1.dsk", "ZORK0.1", &[
                "ZORK0.D1",
                "INFOCOM",
                "INFOCOM.SYSTEM",
                "INFODOS",
            ]),
            ("zork_zero_2.dsk", "ZORK0.2", &["ZORK0.D2"]),
            ("zork_zero_3.dsk", "ZORK0.3", &["ZORK0.D3"]),
            ("zork_zero_4.dsk", "ZORK0.4", &["ZORK0.D4"]),
        ];
        let mut ran = 0;
        for (file, volume, files) in expected {
            let Some(raw) = read_fixture(file) else { continue };
            ran += 1;
            let fs = ProDos::mount(raw).unwrap_or_else(|e| panic!("{file}: {e:?}"));
            assert_eq!(fs.volume_name(), *volume, "{file}");
            let names: Vec<String> = fs.files().iter().map(ProDosEntry::path).collect();
            assert_eq!(names, *files, "{file}");
            // Not one of them is a story on its own — a quarter of a game is
            // not a game, and this is what makes the SET the answer.
            for e in fs.files() {
                let bytes = fs.read(e).unwrap_or_else(|| panic!("{file}: {} unreadable", e.name));
                assert!(!looks_like_story(&bytes), "{file}: {} is a whole story?", e.name);
            }
            assert_eq!(fs.packed_story(), None, "{file}: no ONE volume holds the story");
        }
        // CI has no `stories/` at all, so the premise is guarded rather than
        // asserted outright.
        assert!(ran > 0 || !stories_dir().join("shogun_s1.dsk").exists());
        if ran == 0 {
            eprintln!("SKIP: no 5.25-inch media present");
        }
    }
}
