//! The **Atari 8-bit** S.A.G.A. picture sides (SQ-1483, SQ-1484).
//!
//! Every US S.A.G.A. release for the Atari 8-bit is two single-density disks:
//! side A holds the §12 binary database ([`crate::saga_us`] reads it) and side
//! B holds nothing but artwork. Side B has **no filesystem** — sectors 361-368,
//! where an Atari DOS 2 directory would be, hold picture data like every other
//! sector — so §8.3 is right that "there is no filesystem walk" and §12.10 is
//! right that the association "cannot be recovered from the database".
//!
//! # What the seven sides actually hold, measured
//!
//! **Two formats, not one**, and which one a title uses is not stated anywhere
//! in the specification:
//!
//! | title | side B | this module |
//! |---|---|---|
//! | #1 Adventureland | line-drawing token stream | no |
//! | #2 Pirate Adventure | line-drawing token stream | no |
//! | #3 Mission Impossible | line-drawing token stream | no |
//! | #6 Strange Odyssey | line-drawing token stream | no |
//! | #4 Voodoo Castle | family-C bitmaps, [`FamilyCScheme::NoLiteral`] | **yes** |
//! | #5 The Count | family-C bitmaps, [`FamilyCScheme::NoLiteral`] | **yes** |
//! | #13 Claymorgue Castle | family-C bitmaps, [`FamilyCScheme::Standard`] | **yes** |
//!
//! The split is exactly the split §7.4's string test makes on the **Apple II**
//! releases of the same seven titles: the four whose Apple side A is an
//! ordinary DOS 3.3 disk draw with line tokens on both machines, and the three
//! "scrambled" ones ship bitmaps on both. The line-drawing format is the one
//! Appendix A item 26 measured for the Apple II — three-byte tokens with a
//! command in bits 7-5, `bit 0` carrying bit 8 of *x* — and all four Atari
//! sides open with the very same bytes at file offset `0x1000`. Reading it is
//! [`crate::apple_pictures`]' business, not this module's; nothing addresses
//! those four titles' pictures yet.
//!
//! # The record, as measured
//!
//! §8.3 describes the Commodore 64 record and says the Atari's is the same
//! thing reached differently — "a picture's record starts **two bytes before**
//! the listed offset, and its length is the little-endian word at the listed
//! offset **plus two**". Measured on all three specimens, the Atari record is
//! **ten bytes of header and no tail**:
//!
//! | offset | field |
//! |---|---|
//! | 0-1 | little-endian **record size**, the whole record, this header included |
//! | 2 | left edge in 8-pixel columns **plus 3** |
//! | 3 | top row |
//! | 4 | right edge in columns **plus 3** |
//! | 5 | bottom row |
//! | 6-9 | four colour bytes |
//! | 10.. | compressed data, to the end of the record |
//!
//! So there is **no two-byte load address** — §8.3's "two bytes before the
//! listed offset" are the previous record's last data bytes, and they read as
//! `$FFFF`, `$3F3F` and `$C3C3` as often as anything else — and **no two-byte
//! tail**. The consistency check below is what settles that rather than the
//! prose: read with a twelve-byte header the data is two bytes short and no
//! record's region ever fills.
//!
//! # Finding the records
//!
//! Records are laid end to end from file offset `0x297` — after a boot loader
//! that is byte-identical on all seven sides, and after three bytes holding
//! the release's own Adventure International number — with **nought to six
//! bytes of filler between them**. The filler is why a reader cannot simply
//! add sizes, and [`scan_picture_side`] therefore re-finds each record rather
//! than trusting the previous one's arithmetic.
//!
//! What makes that safe is that a family-C record is **self-proving**. Its
//! header says how many byte pairs it must hold — `cols * pairs`, from
//! [`StripLayout`] — and decoding it either produces exactly that many and
//! stops within a byte of the declared size, or it does not. Measured over all
//! three sides, 241 records satisfy that and every one of them decodes to a
//! coherent picture; the declared size exceeds the decoded length by 0 on 198
//! of them and by exactly 1 on the other 43, and never by more.
//!
//! # The one thing still missing
//!
//! **Which picture index a record answers to is not determined**, and it is
//! not in the data. §8.6 needs a (usage, index) pair per record and §12.10
//! says the database has none; measured here, the disk's own order is not it
//! either — *The Count*'s twenty-five full-canvas records include its darkness
//! card (§8.6's reserved index 0) and its room 1 next to each other and in
//! that order, but the record *before* them is room 2's picture, and no table
//! keyed by record offset or by sector appears on either side of the release.
//! Until that is settled a host can decode these records but cannot say which
//! room wants which, so nothing here is wired into the picture band yet.
//!
//! # The volume table of contents
//!
//! §7.3's splice is real and this module applies it: the 128 bytes at file
//! offsets `0xB390`-`0xB40F` are sector 360, they are not picture data (on
//! *The Count*'s side B they are ten `0xFF` bytes, a long run of zeros and
//! eleven more `0xFF`, between two stretches of run-length data that continue
//! across them), and a record spanning them must have them excised.

use crate::saga_pictures::{
    atari_colour, paint_strips, resolve_palette, FamilyCScheme, Picture, PictureError, StripLayout,
    CANVAS_HEIGHT, CANVAS_WIDTH,
};

/// File offset of sector 360, the volume table of contents (§7.3).
///
/// `16 + 359 * 128`, which is the arithmetic §7.3 gives to confirm both the
/// sixteen-byte header and the 128-byte sector stride.
pub const VTOC_OFFSET: usize = 0xB390;

/// How long the volume table of contents is: one sector.
pub const VTOC_LEN: usize = 128;

/// The byte length of a single-density `.atr` image, header included (§7.3).
pub const SIDE_LEN: usize = 92_176;

/// Where [`scan_picture_side`] starts looking.
///
/// The boot loader in front of it is byte-identical on all seven sides — it
/// prints `HIT RETURN … OTHERWISE INSERT OTHER DISK` — and the first record's
/// header sits at `0x297` on all three bitmap sides, right after three bytes
/// holding the release's Adventure International number (`04 04 04` on *Voodoo
/// Castle*, `05 05 05` on *The Count*, `0D 0D 0D` on *Claymorgue Castle*) and
/// four zeros. The scan begins a little in front of that rather than at it, so
/// that a differently-mastered disk is not refused for the sake of one
/// constant.
pub const FIRST_RECORD: usize = 0x290;

/// One family-C record located on a companion picture side.
///
/// Byte offsets are **into the spliced side** ([`splice_vtoc`]), not into the
/// `.atr` file, because a record may span the volume table of contents and a
/// file offset cannot describe that. [`Self::file_offset`] converts back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtariRecord {
    /// Offset of the record's first header byte, into the spliced side.
    pub offset: usize,
    /// The record's declared size — its own header's first word, covering the
    /// whole record including that word.
    pub size: usize,
    /// How many bytes decoding actually read. Either `size` or `size - 1`;
    /// see the module docs.
    pub decoded_len: usize,
    /// Where the record's strips land, already resolved under the release's
    /// scheme.
    pub layout: StripLayout,
    /// The four stored colour bytes, in file order.
    pub colour_bytes: [u8; 4],
}

impl AtariRecord {
    /// The record's offset in the original `.atr` file.
    ///
    /// Equal to [`Self::offset`] in front of the volume table of contents and
    /// [`VTOC_LEN`] more behind it. A record that *spans* the table has a file
    /// offset in front of it and a length that does not describe its extent on
    /// disk, which is exactly why this type carries spliced offsets.
    pub fn file_offset(&self) -> usize {
        if self.offset < VTOC_OFFSET {
            self.offset
        } else {
            self.offset + VTOC_LEN
        }
    }
}

/// Remove sector 360, the volume table of contents, from a companion side
/// (§7.3).
///
/// Returns `side` unchanged when it is too short to contain the table, so that
/// a caller holding a fragment gets a fragment rather than a panic.
pub fn splice_vtoc(side: &[u8]) -> Vec<u8> {
    if side.len() <= VTOC_OFFSET + VTOC_LEN {
        return side.to_vec();
    }
    let mut out = Vec::with_capacity(side.len() - VTOC_LEN);
    out.extend_from_slice(&side[..VTOC_OFFSET]);
    out.extend_from_slice(&side[VTOC_OFFSET + VTOC_LEN..]);
    out
}

/// Read the record whose header sits at `offset` of a spliced side, or `None`
/// when there is not one there.
///
/// The test is the self-proving one the module docs describe, and it is worth
/// naming the parts because every one of them is load-bearing:
///
/// - the declared size is between the ten-byte header plus one unit and a
///   little over a full-canvas record, and lies inside the side;
/// - the four edge bytes describe a region on a 280 x 160 canvas;
/// - the region wants at most a full canvas' worth of byte pairs;
/// - decoding **fills that region exactly** — not one pair fewer, and not one
///   more than the last run overshoots by;
/// - and the bytes that took land within one of the declared size.
///
/// The fourth is the one that cannot be satisfied by accident. A wrong offset
/// gets a plausible header often enough (a 92 KB side offers 92,000 of them),
/// but its region and its data then disagree, and over three whole sides the
/// only offsets that pass are the 242 that are real records.
pub fn record_at(spliced: &[u8], offset: usize, scheme: FamilyCScheme) -> Option<AtariRecord> {
    let head = spliced.get(offset..offset + 10)?;
    let size = usize::from(u16::from_le_bytes([head[0], head[1]]));
    if !(13..=MAX_RECORD).contains(&size) || offset + size > spliced.len() {
        return None;
    }
    // The edges are bounded as STORED bytes rather than as resolved pixels,
    // because the stored form is where the slack is: full-canvas records
    // routinely declare a left edge of column -1 and a right edge one column
    // past the canvas — two of the *Hulk*'s Commodore 64 records do the same —
    // and clipping in `paint_strips` already keeps those off the canvas.
    if head[2] > MAX_COLUMN || head[4] > MAX_COLUMN || head[5] > MAX_ROW {
        return None;
    }
    let layout = StripLayout::resolve(
        i32::from(head[2]),
        i32::from(head[3]),
        i32::from(head[4]),
        i32::from(head[5]),
        scheme,
    )
    .ok()?;
    if layout.pair_count() > MAX_PAIRS {
        return None;
    }
    // A region wholly off the canvas paints nothing, so whatever it is it is
    // not a picture. Cheap, and principled where a minimum size would not be:
    // *Claymorgue Castle*'s smallest real record is fifteen byte pairs.
    if layout.left + layout.cols * 8 <= 0 || layout.left >= CANVAS_WIDTH as i32 {
        return None;
    }
    let painted = paint_strips(&spliced[offset + 10..offset + size], &layout, scheme);
    if painted.emitted != layout.pair_count() {
        return None;
    }
    let decoded_len = 10 + painted.consumed;
    if !(size == decoded_len || size == decoded_len + 1) {
        return None;
    }
    Some(AtariRecord {
        offset,
        size,
        decoded_len,
        layout,
        colour_bytes: [head[6], head[7], head[8], head[9]],
    })
}

/// The largest record any of the three sides holds, rounded up. *The Count*'s
/// biggest is 4,868 bytes and *Claymorgue Castle*'s 5,199.
const MAX_RECORD: usize = 9_000;

/// How many byte pairs a full canvas is, with room to spare: 36 columns x 80
/// pairs is 2,880, and the widest record measured wants exactly that.
const MAX_PAIRS: usize = 4_200;

/// The largest stored edge column byte. [`CANVAS_WIDTH`] is 35 columns and the
/// stored form adds 3, so 38 is the canvas' right edge; a few columns of slack
/// past it costs nothing, because a wider region simply cannot be filled by
/// the record's own data.
const MAX_COLUMN: u8 = 43;

/// The largest stored bottom row. [`CANVAS_HEIGHT`] is 160 and the tallest
/// record measured declares 158.
const MAX_ROW: u8 = 191;

/// Every family-C record on a companion picture side, in disk order.
///
/// `side` is the whole `.atr` file; the volume-table splice is applied here so
/// that a caller does not have to know about it. `scheme` is the release's,
/// from [`crate::SagaUs::picture_scheme`] — **never sniffed**, for the reason
/// [`FamilyCScheme`] gives.
///
/// The walk is greedy and re-syncing: at each offset it asks [`record_at`],
/// steps over a record it finds, and otherwise advances one byte. That copes
/// with the nought-to-six bytes of filler between records without needing to
/// know how much there is, and it recovers by itself if one record in a side
/// is unreadable.
///
/// Note what this does **not** answer: which picture index each record is. See
/// the module docs.
pub fn scan_picture_side(side: &[u8], scheme: FamilyCScheme) -> Vec<AtariRecord> {
    let spliced = splice_vtoc(side);
    let mut out = Vec::new();
    let mut at = FIRST_RECORD.min(spliced.len());
    while at + 10 <= spliced.len() {
        match record_at(&spliced, at, scheme) {
            Some(rec) => {
                at += rec.size;
                out.push(rec);
            }
            None => at += 1,
        }
    }
    out
}

/// Decode one located record to a picture.
///
/// `spliced` must be the same buffer [`scan_picture_side`] walked — the record
/// carries a spliced offset, not a file one. Reach for [`splice_vtoc`] if you
/// have only the `.atr`.
///
/// # Errors
///
/// [`PictureError::TooShort`] when the record runs past the end of `spliced`.
/// A located record cannot fail any other way: [`record_at`] has already
/// decoded it once.
pub fn decode_record(
    spliced: &[u8],
    record: &AtariRecord,
    scheme: FamilyCScheme,
) -> Result<Picture, PictureError> {
    let data = spliced
        .get(record.offset + 10..record.offset + record.size)
        .ok_or(PictureError::TooShort { len: spliced.len().saturating_sub(record.offset) })?;
    let strips = paint_strips(data, &record.layout, scheme);
    let (palette, unrecognised_colours) =
        resolve_palette(record.colour_bytes, |stored| Some(atari_colour(stored)));
    Ok(Picture {
        width: CANVAS_WIDTH,
        height: CANVAS_HEIGHT,
        pixels: strips.pixels,
        palette,
        colour_bytes: record.colour_bytes,
        unrecognised_colours,
        // Measured from the writes, so a record that paints only part of its
        // own region reports what it really covered (SQ-1487's rectangle).
        painted: strips.bounds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one Atari record: the ten-byte header, then no-literal units.
    fn record(size_fudge: i32, edges: [u8; 4], colours: [u8; 4], units: &[(u8, u8, u8)]) -> Vec<u8> {
        let len = 10 + units.len() * 3;
        let declared = (len as i32 + size_fudge) as u16;
        let mut out = declared.to_le_bytes().to_vec();
        out.extend_from_slice(&edges);
        out.extend_from_slice(&colours);
        for (count, hi, lo) in units {
            out.extend_from_slice(&[*count, *hi, *lo]);
        }
        out
    }

    /// A side with `lead` filler bytes, then the records, then trailing zeros,
    /// padded out past the volume table so the splice has something to do.
    fn side(lead: usize, records: &[Vec<u8>], gaps: &[usize]) -> Vec<u8> {
        let mut out = vec![0u8; FIRST_RECORD + lead];
        for (n, rec) in records.iter().enumerate() {
            out.extend_from_slice(rec);
            out.extend(std::iter::repeat_n(0u8, gaps.get(n).copied().unwrap_or(0)));
        }
        out.resize(SIDE_LEN, 0);
        out
    }

    // A hand-built side, with the record shape this module measured and the
    // pixels worked out from §8.3's rules. Two columns of four pairs at
    // x 0..16, y 0..8 (the exclusive reading of edges 3, 0, 5, 8), so eight
    // pairs and two units.
    #[test]
    fn the_scan_finds_a_hand_built_record_and_decodes_its_pixels() {
        let rec = record(
            0,
            [3, 0, 5, 8],
            [0x36, 0x87, 0x0E, 0x00],
            &[(4, 0xFF, 0xFF), (4, 0x00, 0x55)],
        );
        assert_eq!(rec.len(), 16, "ten header bytes and two three-byte units");
        assert_eq!(usize::from(u16::from_le_bytes([rec[0], rec[1]])), 16, "size covers the header");
        let disk = side(7, std::slice::from_ref(&rec), &[]);

        let found = scan_picture_side(&disk, FamilyCScheme::NoLiteral);
        assert_eq!(found.len(), 1, "exactly one record on the side");
        let r = &found[0];
        assert_eq!(r.offset, FIRST_RECORD + 7);
        assert_eq!(r.file_offset(), FIRST_RECORD + 7, "in front of the volume table");
        assert_eq!(r.size, 16);
        assert_eq!(r.decoded_len, 16, "the data ends where the size says");
        assert_eq!((r.layout.cols, r.layout.pairs), (2, 4));
        assert_eq!(r.colour_bytes, [0x36, 0x87, 0x0E, 0x00]);

        let spliced = splice_vtoc(&disk);
        let pic = decode_record(&spliced, r, FamilyCScheme::NoLiteral).expect("decodes");
        let px = |x: usize, y: usize| pic.pixels[y * CANVAS_WIDTH + x];
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(px(x, y), 3, "column one is all value 3 at ({x}, {y})");
            }
        }
        // (0x00, 0x55): even rows all value 0, odd rows 0b01 four times.
        for y in [0usize, 2, 4, 6] {
            for x in 8..16 {
                assert_eq!(px(x, y), 0, "column two, even row {y}");
                assert_eq!(px(x, y + 1), 1, "column two, odd row {}", y + 1);
            }
        }
        assert_eq!(px(16, 0), 0, "nothing right of the record");
        // The palette resolves through the hardware table and nothing is left
        // unnamed, which is the Atari half of §8.3's colour rule.
        assert_eq!(pic.palette[0], (0, 0, 0), "entry 0 is black however the record reads");
        assert_eq!(pic.palette[3], (0xE0, 0xE0, 0xE0), "0x0E is §8.3's substituted white");
        assert!(pic.unrecognised_colours.is_empty(), "every Atari byte has a colour");
    }

    // Records are laid end to end with nought to six bytes of filler between
    // them; the scan re-finds each one rather than adding sizes.
    #[test]
    fn the_scan_steps_over_the_filler_between_records() {
        let a = record(0, [3, 0, 5, 8], [0x36, 0x87, 0x0E, 0], &[(4, 0xFF, 0xFF), (4, 0, 0)]);
        let b = record(0, [5, 0, 8, 4], [0x36, 0x87, 0x0E, 0], &[(6, 0x55, 0x55)]);
        let c = record(1, [3, 10, 6, 16], [0x94, 0x0E, 0, 0], &[(9, 0xAA, 0xAA)]);
        let disk = side(0, &[a, b, c], &[3, 6, 0]);
        let found = scan_picture_side(&disk, FamilyCScheme::NoLiteral);
        assert_eq!(found.len(), 3, "all three, filler notwithstanding");
        assert_eq!(found[0].offset, FIRST_RECORD);
        assert_eq!(found[1].offset, FIRST_RECORD + 16 + 3, "three bytes of filler");
        assert_eq!(found[2].offset, FIRST_RECORD + 16 + 3 + 13 + 6, "and six more");
        assert_eq!((found[1].layout.cols, found[1].layout.pairs), (3, 2));
        assert_eq!((found[2].layout.cols, found[2].layout.pairs), (3, 3));
        // The last one declares one byte more than it needs, which is what 44
        // of the 242 real records do.
        assert_eq!(found[2].size, found[2].decoded_len + 1);
    }

    // The consistency check is what makes the scan safe, so each of its limbs
    // needs a case that fails it.
    #[test]
    fn a_region_the_data_does_not_fill_is_not_a_record() {
        // One unit short of the region.
        let short = record(0, [3, 0, 5, 8], [0x36, 0x87, 0x0E, 0], &[(4, 0xFF, 0xFF)]);
        let spliced = splice_vtoc(&side(0, std::slice::from_ref(&short), &[]));
        assert_eq!(record_at(&spliced, FIRST_RECORD, FamilyCScheme::NoLiteral), None);
        // The right pairs, but a size two bytes past where they end — the
        // twelve-byte-header reading of §8.3, which is what this refutes.
        let fat = record(2, [3, 0, 5, 8], [0x36, 0x87, 0x0E, 0], &[(4, 0xFF, 0xFF), (4, 0, 0)]);
        let spliced = splice_vtoc(&side(0, std::slice::from_ref(&fat), &[]));
        assert_eq!(record_at(&spliced, FIRST_RECORD, FamilyCScheme::NoLiteral), None);
        // An edge far off the canvas.
        let wide = record(0, [3, 0, 40, 8], [0x36, 0x87, 0x0E, 0], &[(4, 0xFF, 0xFF)]);
        let spliced = splice_vtoc(&side(0, std::slice::from_ref(&wide), &[]));
        assert_eq!(record_at(&spliced, FIRST_RECORD, FamilyCScheme::NoLiteral), None);
    }

    // Same bytes, other scheme: the reading is the release's and the data does
    // not tell you which it is (which is why nothing sniffs).
    #[test]
    fn the_scheme_is_the_releases_and_the_wrong_one_finds_nothing() {
        let rec = record(0, [3, 0, 5, 8], [0x36, 0x87, 0x0E, 0], &[(4, 0xFF, 0xFF), (4, 0, 0)]);
        let disk = side(0, std::slice::from_ref(&rec), &[]);
        assert_eq!(scan_picture_side(&disk, FamilyCScheme::NoLiteral).len(), 1);
        assert!(scan_picture_side(&disk, FamilyCScheme::Standard).is_empty());
    }

    // §7.3's splice: the volume table is excised, and a record behind it keeps
    // a file offset that names where it really is on the disk.
    #[test]
    fn the_volume_table_is_excised_and_file_offsets_survive_it() {
        let mut disk = vec![0u8; SIDE_LEN];
        for (n, b) in disk.iter_mut().enumerate().take(VTOC_OFFSET + VTOC_LEN) {
            if n >= VTOC_OFFSET {
                *b = 0xFF;
            }
        }
        let spliced = splice_vtoc(&disk);
        assert_eq!(spliced.len(), SIDE_LEN - VTOC_LEN);
        assert!(spliced.iter().all(|&b| b == 0), "the 0xFF sector is gone");

        let rec = record(0, [3, 0, 5, 8], [0x36, 0x87, 0x0E, 0], &[(4, 0xFF, 0xFF), (4, 0, 0)]);
        // Put it a little way behind the volume table, in FILE coordinates.
        let mut disk = vec![0u8; SIDE_LEN];
        let file_at = VTOC_OFFSET + VTOC_LEN + 32;
        disk[file_at..file_at + rec.len()].copy_from_slice(&rec);
        disk[VTOC_OFFSET..VTOC_OFFSET + VTOC_LEN].fill(0xEE);
        let found = scan_picture_side(&disk, FamilyCScheme::NoLiteral);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].offset, file_at - VTOC_LEN, "spliced coordinates");
        assert_eq!(found[0].file_offset(), file_at, "and back to the file's");
    }

    // A record that spans the volume table decodes across it, which is the
    // whole point of working in spliced coordinates.
    #[test]
    fn a_record_spanning_the_volume_table_decodes_across_it() {
        let rec = record(0, [3, 0, 5, 8], [0x36, 0x87, 0x0E, 0], &[(4, 0xFF, 0xFF), (4, 0, 0)]);
        let mut disk = vec![0u8; SIDE_LEN];
        // Straddle it: four bytes of header in front, the rest behind.
        let file_at = VTOC_OFFSET - 4;
        disk[file_at..file_at + 4].copy_from_slice(&rec[..4]);
        disk[VTOC_OFFSET..VTOC_OFFSET + VTOC_LEN].fill(0xEE);
        let behind = VTOC_OFFSET + VTOC_LEN;
        disk[behind..behind + rec.len() - 4].copy_from_slice(&rec[4..]);
        let found = scan_picture_side(&disk, FamilyCScheme::NoLiteral);
        assert_eq!(found.len(), 1, "one record, read straight through the splice");
        assert_eq!(found[0].file_offset(), file_at);
        let spliced = splice_vtoc(&disk);
        let pic = decode_record(&spliced, &found[0], FamilyCScheme::NoLiteral).expect("decodes");
        assert_eq!(pic.pixels[0], 3, "and its first stored pixel is value 3");
    }
}
