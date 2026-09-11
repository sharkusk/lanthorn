//! The **DOS sector order** a 5.25-inch Apple II floppy is dumped in, and the
//! two things this module does about it: put the sectors back where ProDOS
//! expects them (SQ-0864), and put them back where DOS 3.3 numbered them
//! (SQ-0868).
//!
//! # What it is, and why it is a wrapper rather than a format
//!
//! An Apple II 5.25-inch disk is 35 tracks of 16 256-byte sectors — 143,360
//! bytes — and a raw dump of one stores those sectors in the order the *drive*
//! numbers them. ProDOS does not read sectors; it reads 512-byte **blocks**, two
//! sectors each, and it numbers them its own way. The two numberings disagree,
//! so a `.dsk` taken off an Apple II is a perfectly ordinary ProDOS volume whose
//! bytes are shuffled — the volume directory is not at offset 1024 and the sniff
//! in [`crate::prodos`] declines it, which is exactly what that module's header
//! said would happen until a reader arrived:
//!
//! > Image formats 0 and 2 are **not** re-ordered or decoded: a DOS-order or
//! > nibble image has its blocks somewhere else, so its volume directory does
//! > not validate and the sniff simply declines it.
//!
//! This is that re-ordering, and it is deliberately nothing else. It is the
//! third wrapper [`crate::prodos`] unwraps, beside a bare volume and a `2IMG`
//! header, and it leaves a ProDOS volume behind for the same reader to open.
//! **It is not a new disk format**: `blorb::medium` gains no row for it, because
//! what comes out the other side is ProDOS and answers to the ProDOS row.
//!
//! # The interleave
//!
//! Within one track, ProDOS block `b` (0..8) is DOS sectors `SECTOR_OF[2b]` and
//! `SECTOR_OF[2b + 1]`:
//!
//! ```text
//!   block  0   1   2   3   4   5   6   7
//!   first  0  13  11   9   7   5   3   1
//!   second 14  12  10   8   6   4   2  15
//! ```
//!
//! Sectors 0 and 15 stay put and the fourteen between them run backwards, which
//! is the shape every published table of this mapping has.
//!
//! # One table, two traversals (SQ-0868)
//!
//! Read that same grid **row-wise** rather than column-wise and it is a
//! different, older mapping: `0 13 11 9 7 5 3 1 14 12 10 8 6 4 2 15` is the
//! physical sector holding DOS 3.3 **logical** sector 0, 1, 2, … — the software
//! skew DOS 3.3 itself applies, and the order the sectors of a file are in on a
//! disk that has no ProDOS on it at all.
//!
//! So the two orders are not two tables. `PHYSICAL_OF` is the one fact, and
//! `SECTOR_OF` is derived from it by the relation the grid states: **ProDOS
//! block `b` of a track is DOS logical sectors `b` and `b + 8`.** That is stated
//! once, in the `const` block below, so the ProDOS order cannot drift from the
//! logical one — and the existing tests that pin `SECTOR_OF`'s shape now pin the
//! derivation too.
//!
//! The corroboration for the logical order is the same kind as for the ProDOS
//! one, and just as unforgiving: `Planetfall r29 (clean copy from retail disk).dsk`
//! is a raw self-booting disk with no filesystem of any kind, and the Version 3
//! story sitting on it verifies against its own header checksum `$842E` **only**
//! under this order. Physical order sums to `$529D` and ProDOS block order to
//! `$97D5`. See [`crate::infocom_boot`], which is the reader that needs it.
//!
//! **Measured, not recalled.** The table is corroborated by the media itself,
//! which is the only authority that matters for a byte layout: applying it to
//! all fourteen 5.25-inch images in the corpus puts a valid ProDOS volume
//! directory at block 2 of every one of them — `SHOGUN.1`…`SHOGUN.5`,
//! `ZORK0.1`…`ZORK0.4` and (since SQ-0863) `JOURNEY.1`…`JOURNEY.5` — each
//! naming the segment files whose reassembly then verifies
//! against the story's own header checksum (see [`crate::infocom_packed`]). A
//! wrong table produces no volume directory at all, so there is nothing here for
//! a shared wrong assumption to hide behind.
//!
//! # Only this geometry
//!
//! DOS sector order is a 5.25-inch phenomenon and 5.25-inch DOS 3.3 media is 35
//! tracks, so [`prodos_order`] answers for exactly 143,360 bytes and declines
//! everything else. That is the same rule the rest of this crate applies to
//! spellings no medium in the corpus wears: a 40-track variant arrives with a
//! disk that is one, not before.

/// Tracks on a 5.25-inch DOS 3.3 floppy.
const TRACKS: usize = 35;

/// Sectors per track.
const SECTORS: usize = 16;

/// Bytes per sector. Two of them make one ProDOS block.
const SECTOR: usize = 256;

/// The one size a DOS-order 5.25-inch dump has: 35 × 16 × 256.
pub const DOS_ORDER_LEN: usize = TRACKS * SECTORS * SECTOR;

/// The PHYSICAL sector holding each DOS 3.3 **logical** sector of a track — the
/// module header's grid read row-wise, and the one table this module states.
const PHYSICAL_OF: [usize; SECTORS] = [0, 13, 11, 9, 7, 5, 3, 1, 14, 12, 10, 8, 6, 4, 2, 15];

/// The DOS sector holding each successive half-block of a track, in ProDOS block
/// order — the same grid read column-wise, and therefore **derived** rather than
/// restated: ProDOS block `b` is DOS logical sectors `b` and `b + 8`.
const SECTOR_OF: [usize; SECTORS] = {
    let mut half = [0usize; SECTORS];
    let mut block = 0;
    while block < SECTORS / 2 {
        half[2 * block] = PHYSICAL_OF[block];
        half[2 * block + 1] = PHYSICAL_OF[block + SECTORS / 2];
        block += 1;
    }
    half
};

/// `raw` with its sectors put back into ProDOS block order, or `None` when it is
/// not a 5.25-inch DOS-order dump at all.
///
/// Says nothing about what the re-ordered bytes hold — that is
/// [`crate::prodos`]'s question, and a dump of a DOS 3.3 or Pascal disk comes
/// through here just as happily and is then declined by the volume sniff.
pub fn prodos_order(raw: &[u8]) -> Option<Vec<u8>> {
    reorder(raw, &SECTOR_OF)
}

/// `raw` with its sectors put back into DOS 3.3 **logical** order, or `None`
/// when it is not a 5.25-inch DOS-order dump at all.
///
/// [`prodos_order`]'s sibling and its equal in reticence: this says nothing
/// about what the re-ordered bytes hold either. A raw self-booting Infocom disk
/// keeps its story in this order ([`crate::infocom_boot`]); so does an ordinary
/// DOS 3.3 disk, which has a VTOC and a catalog this crate reads nothing of.
pub fn logical_order(raw: &[u8]) -> Option<Vec<u8>> {
    reorder(raw, &PHYSICAL_OF)
}

/// A dump of a disk whose sectors are in DOS 3.3 logical order — the inverse of
/// [`logical_order`], and the only function here that goes that way.
///
/// It exists so a test can BUILD one of these disks rather than only take one
/// apart: every fixture in the corpus is already interleaved, so without this a
/// synthetic sample would have to restate the table and could restate it wrong.
/// Test-only for exactly that reason — nothing in the shipped path ever writes a
/// floppy.
#[cfg(test)]
pub(crate) fn dos_order_dump(logical: &[u8]) -> Option<Vec<u8>> {
    if logical.len() != DOS_ORDER_LEN {
        return None;
    }
    let mut out = vec![0u8; DOS_ORDER_LEN];
    for track in 0..TRACKS {
        let base = track * SECTORS * SECTOR;
        for (slot, &sector) in PHYSICAL_OF.iter().enumerate() {
            let (from, to) = (base + slot * SECTOR, base + sector * SECTOR);
            out[to..to + SECTOR].copy_from_slice(&logical[from..from + SECTOR]);
        }
    }
    Some(out)
}

/// The de-interleave both orders are: gather each track's sectors in the order
/// `table` names them.
fn reorder(raw: &[u8], table: &[usize; SECTORS]) -> Option<Vec<u8>> {
    if raw.len() != DOS_ORDER_LEN {
        return None;
    }
    let mut out = Vec::with_capacity(DOS_ORDER_LEN);
    for track in 0..TRACKS {
        let base = track * SECTORS * SECTOR;
        for &sector in table {
            let at = base + sector * SECTOR;
            out.extend_from_slice(&raw[at..at + SECTOR]);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mapping is a permutation of a track — every sector used exactly once,
    /// which is the one thing a transcription error would break. Both orders,
    /// because [`SECTOR_OF`] is now derived and a derivation can drop a sector as
    /// easily as a typist can.
    #[test]
    fn the_interleave_is_a_permutation_of_a_track() {
        for mut seen in [SECTOR_OF, PHYSICAL_OF] {
            seen.sort_unstable();
            assert_eq!(seen, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
        }
    }

    /// **The derivation, pinned by its result** (SQ-0868). `SECTOR_OF` was a
    /// literal until the logical order arrived and showed the two to be one grid
    /// read two ways; this is the literal it used to be, so a change to
    /// [`PHYSICAL_OF`] that would move every ProDOS volume in the corpus fails
    /// here and not fourteen fixtures later.
    #[test]
    fn the_prodos_order_is_still_the_table_it_was_written_as() {
        assert_eq!(SECTOR_OF, [0, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 15]);
    }

    /// The relation the module header states, said once more as an assertion:
    /// ProDOS block `b` of a track is DOS logical sectors `b` and `b + 8`.
    #[test]
    fn a_prodos_block_is_two_logical_sectors_eight_apart() {
        for block in 0..SECTORS / 2 {
            assert_eq!(SECTOR_OF[2 * block], PHYSICAL_OF[block], "block {block} first half");
            assert_eq!(SECTOR_OF[2 * block + 1], PHYSICAL_OF[block + 8], "block {block} second");
        }
    }

    /// The logical order's own shape: sectors 0 and 15 stay put, and the fourteen
    /// between them step by two with a wrap — the classic DOS 3.3 software skew.
    #[test]
    fn the_logical_order_is_the_dos_three_three_skew() {
        assert_eq!(PHYSICAL_OF[0], 0);
        assert_eq!(PHYSICAL_OF[15], 15);
        for (logical, &physical) in PHYSICAL_OF.iter().enumerate().take(15).skip(1) {
            assert_eq!(physical, (logical * 13) % 15, "logical sector {logical}");
        }
    }

    /// [`dos_order_dump`] really is [`logical_order`] backwards — the property
    /// every synthetic boot-disk fixture rests on.
    #[test]
    fn a_dump_and_the_logical_order_undo_each_other() {
        let logical: Vec<u8> = (0..DOS_ORDER_LEN).map(|i| (i / SECTOR) as u8).collect();
        let dump = dos_order_dump(&logical).expect("the 5.25-inch geometry");
        assert_ne!(dump, logical, "the sectors really move");
        assert_eq!(logical_order(&dump).expect("the geometry"), logical);
        assert_eq!(dos_order_dump(&[]), None);
    }

    /// Both orders move whole sectors, and each is the other's disagreement:
    /// applying one where the other belongs is a scramble, not a near miss.
    #[test]
    fn the_two_orders_are_not_the_same_order() {
        let raw: Vec<u8> = (0..DOS_ORDER_LEN).map(|i| (i / SECTOR) as u8).collect();
        let prodos = prodos_order(&raw).expect("the 5.25-inch geometry");
        let logical = logical_order(&raw).expect("the 5.25-inch geometry");
        assert_ne!(prodos, logical);
        assert_eq!(logical_order(&[]), None);
        assert_eq!(logical_order(&vec![0u8; DOS_ORDER_LEN - 1]), None);
    }

    /// Sector 0 opens the track and sector 15 closes it; the fourteen between
    /// them descend. Stated as a test because it is the whole shape of the
    /// table, and a table that lost it would still be a permutation.
    #[test]
    fn the_ends_stay_put_and_the_middle_runs_backwards() {
        assert_eq!(SECTOR_OF[0], 0);
        assert_eq!(SECTOR_OF[15], 15);
        assert!(SECTOR_OF[1..15].windows(2).all(|w| w[0] == w[1] + 1), "{SECTOR_OF:?}");
    }

    /// De-interleaving moves whole sectors and never a byte across a sector
    /// boundary: track `t`'s ProDOS block `b` is DOS sectors `2b` and `2b+1` of
    /// the table, byte for byte.
    #[test]
    fn every_sector_lands_where_the_table_says() {
        // Each sector filled with its own global index, so a misplacement is
        // visible in one byte.
        let raw: Vec<u8> = (0..DOS_ORDER_LEN).map(|i| (i / SECTOR) as u8).collect();
        let out = prodos_order(&raw).expect("the 5.25-inch geometry");
        assert_eq!(out.len(), DOS_ORDER_LEN);
        for track in 0..TRACKS {
            for (half, sector) in SECTOR_OF.iter().enumerate() {
                let at = (track * SECTORS + half) * SECTOR;
                let want = ((track * SECTORS + sector) % 256) as u8;
                assert_eq!(out[at], want, "track {track} half {half}");
                assert!(out[at..at + SECTOR].iter().all(|&b| b == want), "sector split");
            }
        }
    }

    /// Block 0 of every track is sectors 0 and 14 — the one case worth naming
    /// outright, because it is where a volume directory search begins.
    #[test]
    fn block_zero_of_a_track_is_sectors_zero_and_fourteen() {
        let mut raw = vec![0u8; DOS_ORDER_LEN];
        raw[0] = 0xA1; // track 0, sector 0
        raw[14 * SECTOR] = 0xA2; // track 0, sector 14
        let out = prodos_order(&raw).expect("the 5.25-inch geometry");
        assert_eq!(out[0], 0xA1);
        assert_eq!(out[SECTOR], 0xA2);
    }

    /// Any other size is not this medium, and is declined rather than
    /// re-ordered into nonsense. 800 KB is the size that matters — it is a
    /// multiple of a track and it is every `.2mg` in the corpus.
    #[test]
    fn only_the_five_and_a_quarter_inch_geometry_is_claimed() {
        assert_eq!(prodos_order(&[]), None);
        assert_eq!(prodos_order(&vec![0u8; DOS_ORDER_LEN - 1]), None);
        assert_eq!(prodos_order(&vec![0u8; DOS_ORDER_LEN + 1]), None);
        assert_eq!(prodos_order(&vec![0u8; 819_200]), None, "an 800 KB 3.5-inch volume");
        assert!(prodos_order(&vec![0u8; DOS_ORDER_LEN]).is_some());
    }
}
