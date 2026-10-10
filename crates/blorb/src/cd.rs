//! The container a CD-ROM's filesystem sits inside: raw sectors, and the Apple
//! Partition Map (SQ-0870).
//!
//! This is a **wrapper, not a reader**. It is the same shape
//! [`crate::hfs::Hfs`] already uses to see past a DiskCopy 4.2 header — decide
//! where the volume really starts, then hand the volume to the filesystem that
//! knows what to do with it. Nothing here parses a filesystem, and nothing here
//! is Macintosh-specific except the one partition type [`hfs_partition`] asks
//! for.
//!
//! It exists because the *Classic Text Adventure Masterpieces of Infocom* CD is
//! a hybrid disc: one pressing carrying a Macintosh volume and a PC one, and the
//! Macintosh half is a partition three layers down. Reading it by hand — dd the
//! partition out, mount the extraction — is the extraction step
//! [`crate::hfs`]'s header opens by refusing to make anyone do.
//!
//! # Layer 1 — raw sectors, whose stride is measured rather than assumed
//!
//! A CD sector carries 2048 bytes of user data inside a larger frame, and a dump
//! may keep the whole frame (`.bin`, "raw") or only the user data (`.iso`,
//! "cooked"). A raw frame opens with a 12-byte sync pattern — `00` then ten
//! `FF` then `00` — followed by a 3-byte address and a mode byte:
//!
//! ```text
//!   0   sync (12)      00 ff ff ff ff ff ff ff ff ff ff 00
//!  12   address (3)    minute, second, frame, BCD
//!  15   mode (1)       1 = MODE1, 2 = MODE2
//!  16   user data      2048 bytes (MODE1), or a subheader then 2048 (MODE2/FORM1 at +24)
//! ```
//!
//! **The stride is the distance from one sync to the next**, and measuring it
//! beats matching a constant: 2352 is the usual raw frame and 2448 is the same
//! frame with 96 bytes of subchannel data appended, both circulate, and neither
//! number has to be known in advance to find it. No sync at all means the file
//! is already user data, which is the cooked case and needs no unwrapping — and
//! the two cannot be confused, because a cooked image opens with a filesystem's
//! own bytes and not with `00 ff ff ff ff ff ff ff ff ff ff 00`.
//!
//! **The mode byte is read rather than assumed** for the same reason: MODE1 puts
//! user data at +16 and MODE2/FORM1 puts it at +24, so a MODE2 disc read at +16
//! yields garbage that looks like a corrupt filesystem rather than like a wrong
//! offset.
//!
//! Measured on `masterpieces/Classic Text Adventure Masterpieces of Infocom
//! (USA).bin`, 354,011,280 bytes:
//!
//! ```text
//!   sync at 0, and again at 2352, 4704, 7056, 9408, 11760   -> stride 2352
//!   mode byte at 15                                          -> 1, MODE1, data at +16
//!   size % 2352 = 0, i.e. 150,515 whole sectors
//!   size % 2048 = 144, so a cooked read is not even arithmetically possible
//! ```
//!
//! # Layer 2 — the Apple Partition Map
//!
//! Logical block 0 of an Apple-partitioned medium is the driver descriptor: `ER`,
//! then the block size and the block count. The partition map itself starts at
//! block 1, one `PM` entry per block, each naming a partition's first block, its
//! length and its type. Structure from *Inside Macintosh: Devices*, chapter 3
//! ("SCSI Manager"), "The Partition Map".
//!
//! On the same disc:
//!
//! ```text
//!   block 0   ER, block size 512, block count 1,300,492
//!   entry 1   'QuickTOPiX II by OMI'  Apple_partition_map   start 1     3 blocks
//!   entry 2   'ISO9660_system'        ISO9660_system        start 4     509 blocks
//!   entry 3   'Masterpieces'          Apple_HFS             start 513   1,299,979 blocks
//! ```
//!
//! So the Macintosh volume begins at logical byte 262,656 and runs to the end of
//! the disc: 307,992,064 bytes present, against the 665,589,248 the map claims.
//! That gap is not damage and is the ordinary case on a hybrid disc — the
//! partition is sized for the medium and shares it with the ISO9660 side — which
//! is the other half of SQ-0870 and is answered in [`crate::hfs`], where the
//! bound on a volume's length lives.
//!
//! **The ISO9660 side is not read here.** It holds this release's PC collection
//! and is SQ-0871's business. What it is used for is corroboration: ISO9660 puts
//! its primary volume descriptor at logical sector 16, so `CD001` appearing
//! there is an independent check that the stride and the data offset are right —
//! see `the_sector_mapping_is_confirmed_by_two_independent_signatures`. It is
//! asserted in the tests rather than required by [`hfs_partition`], because a
//! Macintosh-only CD-ROM has an Apple Partition Map and no ISO9660 side at all,
//! and refusing one would be requiring evidence that a valid disc need not carry.

/// Bytes of user data in one CD sector: the logical block a filesystem on the
/// disc is written in, and the whole of what a cooked image contains.
pub(crate) const USER_DATA: usize = 2048;

/// The 12-byte sync pattern a raw sector opens with.
const SYNC: [u8; 12] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];
/// The mode byte, past the sync and the 3-byte address.
const MODE_OFF: usize = 15;
/// MODE1's user data begins straight after that 16-byte header.
const MODE1_DATA: usize = 16;
/// MODE2/FORM1 puts an 8-byte subheader first.
const MODE2_DATA: usize = 24;
/// The widest raw frame we look for the second sync inside: 2352 plus 96 bytes
/// of subchannel.
const MAX_STRIDE: usize = 2448;

/// Apple's driver descriptor record, at logical block 0.
const DDR_SIGNATURE: [u8; 2] = *b"ER";
/// `sbBlkSize`, the medium's block size in bytes.
const DDR_BLK_SIZE: usize = 2;
/// One partition map entry.
const PM_SIGNATURE: [u8; 2] = *b"PM";
/// `pmMapBlkCnt` — how many blocks the map itself occupies, i.e. how many
/// entries there are.
const PM_MAP_BLK_CNT: usize = 4;
/// `pmPyPartStart` — the partition's first block, from the start of the medium.
const PM_PY_PART_START: usize = 8;
/// `pmPartBlkCnt` — its length in blocks.
const PM_PART_BLK_CNT: usize = 12;
/// `pmParType` — a null-terminated ASCII type name.
const PM_PART_TYPE: usize = 48;
/// The type a Macintosh volume wears.
const APPLE_HFS: &str = "Apple_HFS";
/// The largest map we walk. Apple's own limit is far lower; this only has to
/// stop a corrupt `pmMapBlkCnt` from being a loop bound.
const MAX_PARTITIONS: usize = 64;
/// A block size has to be a whole number of these, and no larger than a sector.
const BLOCK_UNIT: usize = 512;

/// How an image's bytes relate to the logical blocks a filesystem is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sectors {
    /// The file is user data already: logical byte *n* is byte *n*.
    Cooked,
    /// The file is raw frames: `stride` bytes each, user data `data` bytes in.
    Raw { stride: usize, data: usize },
}

impl Sectors {
    /// Measure `image`'s framing — see this module's header for why the stride
    /// is measured and the mode byte read.
    pub(crate) fn of(image: &[u8]) -> Sectors {
        let Some(head) = image.get(..MAX_STRIDE + SYNC.len()) else { return Sectors::Cooked };
        if head[..SYNC.len()] != SYNC {
            return Sectors::Cooked;
        }
        let data = match head[MODE_OFF] {
            1 => MODE1_DATA,
            2 => MODE2_DATA,
            _ => return Sectors::Cooked,
        };
        // The distance to the next sync IS the frame length, whatever it is.
        let Some(stride) =
            (SYNC.len()..=MAX_STRIDE).find(|&at| head[at..at + SYNC.len()] == SYNC)
        else {
            return Sectors::Cooked;
        };
        if stride < data + USER_DATA { Sectors::Cooked } else { Sectors::Raw { stride, data } }
    }

    /// How many logical bytes the image holds. A trailing partial frame is not
    /// one and is dropped.
    fn logical_len(self, image: &[u8]) -> usize {
        self.logical_len_of(image.len())
    }

    /// [`Self::logical_len`] for a file of `file_len` bytes that is not in hand —
    /// the file-backed reader's question (SQ-1761).
    pub(crate) fn logical_len_of(self, file_len: usize) -> usize {
        match self {
            Sectors::Cooked => file_len,
            Sectors::Raw { stride, .. } => file_len / stride * USER_DATA,
        }
    }

    /// `len` logical bytes from logical offset `at`, short at the end of the
    /// image. Copied rather than borrowed, because raw user data is not
    /// contiguous in the file.
    fn copy(self, image: &[u8], at: usize, len: usize) -> Vec<u8> {
        match self {
            Sectors::Cooked => {
                image.get(at..(at + len).min(image.len())).unwrap_or_default().to_vec()
            }
            Sectors::Raw { stride, data } => {
                let have = self.logical_len(image).saturating_sub(at);
                let mut out = Vec::with_capacity(len.min(have));
                let (mut sector, mut off) = (at / USER_DATA, at % USER_DATA);
                while out.len() < len {
                    let from = sector * stride + data + off;
                    let take = (USER_DATA - off).min(len - out.len());
                    let Some(chunk) = image.get(from..from + take) else { break };
                    out.extend_from_slice(chunk);
                    sector += 1;
                    off = 0;
                }
                out
            }
        }
    }
}

/// The Apple_HFS partition of a partitioned disc image: where it is, how much of
/// it the image actually holds, and how to get at its bytes.
///
/// **The length is what is PRESENT, not what the map claims.** On a hybrid disc
/// those differ by design — see this module's header — and every caller here
/// wants the first.
#[derive(Debug)]
pub(crate) struct HfsPartition<'a> {
    image: &'a [u8],
    sectors: Sectors,
    /// The partition's first byte, in logical bytes.
    at: usize,
    /// How many of its bytes the image holds, clamped to the image.
    len: usize,
}

/// The Apple_HFS partition `image` carries, or `None` when it is not an
/// Apple-partitioned medium or has no Macintosh volume on it.
///
/// Cheap: a handful of 512-byte reads, whatever the size of the image. Nothing
/// is copied and no filesystem is touched, so a caller may ask this of every
/// file in a directory.
pub(crate) fn hfs_partition(image: &[u8]) -> Option<HfsPartition<'_>> {
    let sectors = Sectors::of(image);
    let total = sectors.logical_len(image);
    let (at, len) = locate_hfs(total, |at, len| sectors.copy(image, at, len))?;
    Some(HfsPartition { image, sectors, at, len })
}

/// Where the Apple_HFS partition starts and how much of it is present, on a
/// medium of `total` logical bytes whose bytes `read` fetches — `(start, len)`
/// in logical bytes.
///
/// **The one copy of the walk**, shared by the in-memory [`hfs_partition`] and
/// the file-backed reader (SQ-1761), which supply `read` differently and must
/// not be able to disagree about where a partition is. `read` returns what it
/// can, short at the end of the medium, exactly as [`Sectors::copy`] does.
pub(crate) fn locate_hfs(
    total: usize,
    read: impl Fn(usize, usize) -> Vec<u8>,
) -> Option<(usize, usize)> {
    let driver = read(0, BLOCK_UNIT);
    if driver.len() < 8 || driver[..2] != DDR_SIGNATURE {
        return None;
    }
    let block = usize::from(be16(&driver, DDR_BLK_SIZE));
    if !(BLOCK_UNIT..=USER_DATA).contains(&block) || !block.is_multiple_of(BLOCK_UNIT) {
        return None;
    }
    // Entry *n* is block *n*, and the first entry says how many there are —
    // read as the loop's bound rather than trusted as one, so a corrupt count
    // cannot walk the whole medium.
    let mut entries = MAX_PARTITIONS;
    let mut n = 1;
    while n <= entries {
        let e = read(n * block, block);
        if e.len() < PM_PART_TYPE + 32 || e[..2] != PM_SIGNATURE {
            return None;
        }
        if n == 1 {
            entries = (be32(&e, PM_MAP_BLK_CNT) as usize).clamp(1, MAX_PARTITIONS);
        }
        n += 1;
        if !ascii_field(&e[PM_PART_TYPE..]).eq_ignore_ascii_case(APPLE_HFS) {
            continue;
        }
        let at = (be32(&e, PM_PY_PART_START) as usize).checked_mul(block)?;
        let claimed = (be32(&e, PM_PART_BLK_CNT) as usize).checked_mul(block)?;
        // Present, not claimed: the map sizes a hybrid disc's Macintosh
        // partition for the whole medium.
        let len = claimed.min(total.checked_sub(at)?);
        return Some((at, len));
    }
    None
}

impl HfsPartition<'_> {
    /// How many of the partition's bytes the image holds.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// The partition's byte offset **into the image as given**, when the image's
    /// bytes are contiguous — a cooked `.iso`, or any other Apple-partitioned
    /// image. `None` for a raw dump, whose user data is interrupted by a frame
    /// header every 2048 bytes and therefore has no such offset.
    ///
    /// This is what lets the cooked case cost nothing: the volume is read in
    /// place, exactly like a bare volume or one behind a DiskCopy header.
    pub(crate) fn contiguous_at(&self) -> Option<usize> {
        (self.sectors == Sectors::Cooked).then_some(self.at)
    }

    /// The partition's first `len` bytes — enough to look at a volume header
    /// without copying a disc.
    pub(crate) fn head(&self, len: usize) -> Vec<u8> {
        self.sectors.copy(self.image, self.at, len.min(self.len))
    }

    /// The whole partition, as one contiguous volume.
    ///
    /// **The one expensive call here**, and the only one a mount makes: a raw
    /// disc's user data has to be gathered before a filesystem reader can index
    /// it. 308 MB for the Masterpieces CD, held alongside the 354 MB image until
    /// the caller drops it.
    pub(crate) fn extract(&self) -> Vec<u8> {
        self.sectors.copy(self.image, self.at, self.len)
    }
}

/// A null-padded ASCII field, up to its first `NUL`.
fn ascii_field(b: &[u8]) -> String {
    b.iter().take_while(|c| **c != 0).map(|c| char::from(*c)).collect()
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

    /// The disc in the corpus: the hybrid *Classic Text Adventure Masterpieces
    /// of Infocom*, a raw MODE1 dump. Outside the repo, so every test that wants
    /// it skips vacuously — CI has no `masterpieces/` at all.
    pub(crate) fn masterpieces_cd() -> Option<Vec<u8>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../masterpieces/Classic Text Adventure Masterpieces of Infocom (USA).bin");
        std::fs::read(&path).ok()
    }

    /// Lay `volume` out as an Apple-partitioned medium: a driver descriptor, a
    /// three-entry map, and the volume as the third partition — the shape of
    /// every hybrid disc, at a size a test can hold.
    ///
    /// `claimed` is what the map SAYS the Macintosh partition is, in blocks,
    /// which on a real hybrid disc is far more than the medium holds.
    pub(crate) fn partitioned(volume: &[u8], claimed: usize) -> Vec<u8> {
        let block = BLOCK_UNIT;
        let start = 4;
        let mut out = vec![0u8; start * block];
        out[..2].copy_from_slice(&DDR_SIGNATURE);
        out[DDR_BLK_SIZE..DDR_BLK_SIZE + 2].copy_from_slice(&(block as u16).to_be_bytes());
        let entry = |at: usize, count: usize, kind: &str| {
            let mut e = vec![0u8; block];
            e[..2].copy_from_slice(&PM_SIGNATURE);
            e[PM_MAP_BLK_CNT..PM_MAP_BLK_CNT + 4].copy_from_slice(&3u32.to_be_bytes());
            e[PM_PY_PART_START..PM_PY_PART_START + 4].copy_from_slice(&(at as u32).to_be_bytes());
            e[PM_PART_BLK_CNT..PM_PART_BLK_CNT + 4].copy_from_slice(&(count as u32).to_be_bytes());
            e[PM_PART_TYPE..PM_PART_TYPE + kind.len()].copy_from_slice(kind.as_bytes());
            e
        };
        out[block..2 * block].copy_from_slice(&entry(1, 3, "Apple_partition_map"));
        out[2 * block..3 * block].copy_from_slice(&entry(3, 1, "ISO9660_system"));
        out[3 * block..4 * block].copy_from_slice(&entry(start, claimed, APPLE_HFS));
        out.extend_from_slice(volume);
        out
    }

    /// Cut a cooked image into raw MODE1 frames, sync pattern and all.
    pub(crate) fn raw_sectors(cooked: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for (n, chunk) in cooked.chunks(USER_DATA).enumerate() {
            let mut frame = vec![0u8; 2352];
            frame[..SYNC.len()].copy_from_slice(&SYNC);
            frame[12] = (n / 4500) as u8;
            frame[MODE_OFF] = 1;
            frame[MODE1_DATA..MODE1_DATA + chunk.len()].copy_from_slice(chunk);
            out.extend_from_slice(&frame);
        }
        out
    }

    /// A disc's bytes, read back through the mapping, are the bytes that went in
    /// — whatever the framing.
    #[test]
    fn a_raw_dump_and_a_cooked_one_read_the_same() {
        let cooked: Vec<u8> = (0..10 * USER_DATA).map(|i| (i % 251) as u8).collect();
        let raw = raw_sectors(&cooked);
        assert_eq!(raw.len(), 10 * 2352, "ten MODE1 frames");
        let s = Sectors::of(&raw);
        assert_eq!(s, Sectors::Raw { stride: 2352, data: MODE1_DATA }, "the stride is measured");
        assert_eq!(s.logical_len(&raw), cooked.len());
        assert_eq!(s.copy(&raw, 0, cooked.len()), cooked, "read back whole");
        // …and from an arbitrary offset, across frame boundaries.
        assert_eq!(s.copy(&raw, 3000, 4000), cooked[3000..7000], "across three frames");

        let c = Sectors::of(&cooked);
        assert_eq!(c, Sectors::Cooked, "no sync means the file already IS user data");
        assert_eq!(c.copy(&cooked, 3000, 4000), cooked[3000..7000]);
        // A short read stops at the end rather than inventing bytes.
        assert_eq!(s.copy(&raw, cooked.len() - 10, 500).len(), 10);
    }

    /// A 2448-byte frame is 2352 plus subchannel data, and measuring the stride
    /// reads it with nothing taught about the number.
    #[test]
    fn the_stride_is_whatever_the_disc_uses() {
        let cooked: Vec<u8> = (0..4 * USER_DATA).map(|i| (i % 241) as u8).collect();
        let mut wide = Vec::new();
        for chunk in cooked.chunks(USER_DATA) {
            let mut frame = vec![0u8; 2448];
            frame[..SYNC.len()].copy_from_slice(&SYNC);
            frame[MODE_OFF] = 1;
            frame[MODE1_DATA..MODE1_DATA + chunk.len()].copy_from_slice(chunk);
            wide.extend_from_slice(&frame);
        }
        assert_eq!(Sectors::of(&wide), Sectors::Raw { stride: 2448, data: MODE1_DATA });
        assert_eq!(Sectors::of(&wide).copy(&wide, 0, cooked.len()), cooked);

        // MODE2/FORM1 moves the user data 8 bytes later, and the mode byte is
        // read rather than assumed — at +16 this would be the subheader.
        let mut mode2 = Vec::new();
        for chunk in cooked.chunks(USER_DATA) {
            let mut frame = vec![0u8; 2352];
            frame[..SYNC.len()].copy_from_slice(&SYNC);
            frame[MODE_OFF] = 2;
            frame[MODE2_DATA..MODE2_DATA + chunk.len()].copy_from_slice(chunk);
            mode2.extend_from_slice(&frame);
        }
        assert_eq!(Sectors::of(&mode2), Sectors::Raw { stride: 2352, data: MODE2_DATA });
        assert_eq!(Sectors::of(&mode2).copy(&mode2, 0, cooked.len()), cooked);
    }

    /// The partition map is walked to the Apple_HFS entry, and the length it
    /// reports is what the image HOLDS — the whole of SQ-0870's second half, at
    /// the layer where the claim is made.
    #[test]
    fn the_map_reports_the_partition_that_is_present_not_the_one_it_claims() {
        let volume: Vec<u8> = (0..8 * BLOCK_UNIT).map(|i| (i % 253) as u8).collect();
        // Claim a partition ten times the size of the one that is here, exactly
        // as a hybrid disc's map does.
        let disc = partitioned(&volume, 80);
        for (what, image) in [("cooked", disc.clone()), ("raw", raw_sectors(&disc))] {
            let p = hfs_partition(&image).unwrap_or_else(|| panic!("{what}: no partition"));
            assert_eq!(p.len(), volume.len(), "{what}: present, not claimed");
            assert_eq!(p.extract(), volume, "{what}: and the bytes are the volume's");
            assert_eq!(p.head(16), volume[..16], "{what}");
        }
        // Read in place when the bytes are contiguous, copied when they are not.
        assert_eq!(hfs_partition(&disc).unwrap().contiguous_at(), Some(4 * BLOCK_UNIT));
        assert_eq!(hfs_partition(&raw_sectors(&disc)).unwrap().contiguous_at(), None);
    }

    /// Nothing that is not an Apple-partitioned medium is claimed as one.
    #[test]
    fn an_unpartitioned_image_is_not_claimed() {
        assert!(hfs_partition(b"").is_none());
        assert!(hfs_partition(&vec![0u8; 64 * 1024]).is_none(), "no driver descriptor");
        assert!(hfs_partition(&raw_sectors(&vec![0u8; 64 * 1024])).is_none());
        // An Apple-partitioned medium with no Macintosh volume on it: a map, and
        // nothing of ours in it.
        let mut disc = partitioned(&vec![0u8; 512], 1);
        disc[3 * BLOCK_UNIT + PM_PART_TYPE..3 * BLOCK_UNIT + PM_PART_TYPE + 9]
            .copy_from_slice(b"Apple_UNI");
        assert!(hfs_partition(&disc).is_none(), "no Apple_HFS partition");
        // A driver descriptor whose map is not one.
        let mut broken = partitioned(&vec![0u8; 512], 1);
        broken[BLOCK_UNIT] = b'X';
        assert!(hfs_partition(&broken).is_none());
    }

    /// **Real media**: the mapping this module derives is confirmed by two
    /// signatures it never looked for while deriving it.
    ///
    /// ISO9660 puts its primary volume descriptor at logical sector 16 and Apple
    /// puts its driver descriptor at logical block 0; if the stride or the data
    /// offset were wrong, neither would land where it lands. That is a far
    /// stronger check than the file size dividing by 2352, which a 354 MB file
    /// can satisfy by coincidence.
    ///
    /// `CD001` is corroboration only — [`hfs_partition`] does not require it,
    /// because a Macintosh-only CD-ROM carries an Apple Partition Map and no
    /// ISO9660 side at all. What is behind that descriptor — this release's PC
    /// collection — is SQ-0871 and is not read here.
    #[test]
    fn the_sector_mapping_is_confirmed_by_two_independent_signatures() {
        let Some(bin) = masterpieces_cd() else {
            eprintln!("SKIP: the Masterpieces CD is absent");
            return;
        };
        assert_eq!(bin.len(), 354_011_280);
        assert_eq!(bin.len() % USER_DATA, 144, "a cooked read is not arithmetically possible");
        let s = Sectors::of(&bin);
        assert_eq!(s, Sectors::Raw { stride: 2352, data: MODE1_DATA }, "MODE1/2352, measured");
        assert_eq!(s.logical_len(&bin), 150_515 * USER_DATA);
        assert_eq!(&s.copy(&bin, 0, 2)[..], b"ER", "Apple driver descriptor at logical block 0");
        assert_eq!(
            &s.copy(&bin, 16 * USER_DATA + 1, 5)[..],
            b"CD001",
            "ISO9660's primary volume descriptor at logical sector 16"
        );

        let p = hfs_partition(&bin).expect("the disc carries a Macintosh partition");
        assert_eq!(p.contiguous_at(), None, "a raw dump's user data is not contiguous");
        assert_eq!(p.at, 513 * BLOCK_UNIT, "start block 513, from the map");
        assert_eq!(p.len(), 307_992_064, "present, against the 665,589,248 the map claims");
        assert_eq!(&p.head(1026)[1024..1026], b"BD", "and an HFS volume is there");
    }
}
