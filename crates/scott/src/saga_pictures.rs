//! Picture family **C** — the US S.A.G.A. four-colour strip bitmaps
//! (spec §8.3), as shipped on the Commodore 64 and Atari 8-bit disk releases
//! of *Adventureland*, *Pirate Adventure*, *Voodoo Castle*, *The Count*,
//! *Claymorgue Castle* and the *Hulk*.
//!
//! This module decodes **one record to one indexed bitmap**. It has no opinion
//! about where a record came from: on the Commodore 64 a record is a named file
//! in the disk image's directory ([`crate::saga_us::is_picture_file_name`]
//! names them), on the Atari it is a byte range at a hard-coded offset into the
//! companion picture side. Container work is the host's — this crate reads no
//! disk images.
//!
//! Which record a room wants is [`crate::SagaUs::room_picture`] and
//! [`crate::saga_us::picture_file_name`]; §12.10 has the short version — "a
//! room's picture index **is** the room number", with the *Hulk*'s five
//! remapped pairs the one exception.

use crate::saga_us::SagaPlatform;

/// The family-C canvas width in pixels (§8.3).
///
/// A stored pixel is two device pixels wide, so a byte's four pixels cover
/// eight horizontal positions and this is 35 whole bytes across. The
/// horizontal placement and width fields are in those 8-pixel columns.
pub const CANVAS_WIDTH: usize = 280;

/// The family-C canvas height in pixels.
///
/// **§8.3 says 158 and the records say 160.** The height field is an
/// *inclusive* limit on the row counter, so the last pair of a column paints
/// rows `height` and `height + 1`: every full-canvas picture on
/// `QUESTPR1.D64` declares height 158 and stores exactly
/// 36 columns x 80 pairs = 5,760 bytes, which is 160 rows, not 158. 160 is
/// also what §8.4 gives as family D's nominal height for the same artwork on
/// the Apple II, and what the MS-DOS family-E twin of each picture decodes to
/// (78 rows per pass, two interleaved passes, 156 rows plus the two the
/// even/odd split rounds off). See [`decode_family_c`] for the arithmetic.
pub const CANVAS_HEIGHT: usize = 160;

/// An RGB triple.
pub type Rgb = (u8, u8, u8);

/// Why a record is not a family-C picture (§11's "name it and refuse it").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureError {
    /// Fewer than the twelve header bytes plus the two-byte tail every record
    /// ends with, so there is no picture here at all.
    TooShort {
        /// The length that was offered.
        len: usize,
    },
    /// The header's placement fields do not describe a region: the right edge
    /// is left of the left edge, or the bottom above the top.
    EmptyRegion {
        /// Left edge in pixels, `header[4] - 3` columns.
        left: i32,
        /// Right edge in pixels, `header[6] - 3` columns.
        right: i32,
        /// Top row, `header[5]`.
        top: i32,
        /// Bottom row, `header[7]`.
        bottom: i32,
    },
    /// The platform does not use family C. The Apple II releases are family D
    /// (§8.4) — a simulated high-resolution page resolved by an artifact
    /// model, nothing this decoder can stand in for.
    NotFamilyC {
        /// The platform asked for.
        platform: SagaPlatform,
    },
    /// The record's signature bytes are not a family-E picture's (§8.5,
    /// SQ-1477) — an `.EXE`, a `.BAT` or a database in the same archive as
    /// the artwork. Raised by
    /// [`crate::saga_dos::decode_family_e`]
    /// only.
    NotFamilyE {
        /// The first five bytes, so a diagnostic can say what it found.
        magic: [u8; 5],
    },
}

impl std::fmt::Display for PictureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PictureError::TooShort { len } => {
                write!(f, "family-C record is {len} bytes, too short for a 12-byte header and a tail")
            }
            PictureError::EmptyRegion { left, right, top, bottom } => write!(
                f,
                "family-C placement describes no region: x {left}..{right}, y {top}..{bottom}"
            ),
            PictureError::NotFamilyC { platform } => {
                write!(f, "{} releases do not use picture family C", platform.label())
            }
            PictureError::NotFamilyE { magic } => {
                write!(f, "not a family-E picture: it opens {magic:02X?}")
            }
        }
    }
}

impl std::error::Error for PictureError {}

/// One decoded family-C picture: an indexed bitmap over a four-entry palette.
///
/// Always the full [`CANVAS_WIDTH`] x [`CANVAS_HEIGHT`] canvas, whatever
/// region the record's header covers, because a picture is *placed* on the
/// screen rather than drawn at the origin: an object overlay declares its own
/// absolute position (§8.6, "overlays here have no position field at all;
/// each object picture carries its own absolute placement") and only makes
/// sense against the same canvas the room picture is on. Pixels outside the
/// record's own region keep value 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// [`CANVAS_WIDTH`].
    pub width: usize,
    /// [`CANVAS_HEIGHT`].
    pub height: usize,
    /// `width * height` pixel values, each 0-3, row-major from the top-left.
    /// Index [`Self::palette`] with one to get a colour.
    pub pixels: Vec<u8>,
    /// The colour each pixel value resolves to. **Entry 0 is always black**
    /// whatever the record says (§8.3), and entries 1-3 are the first three
    /// stored colour bytes; the fourth stored byte is never used. A byte
    /// [`Self::unrecognised_colours`] lists resolved to black here, because
    /// §8.3's instruction is to surface an unknown value rather than invent a
    /// colour for it.
    pub palette: [Rgb; 4],
    /// The four stored colour bytes in file order, so a host can report what
    /// it could not resolve — or, on the Atari, what a fuller palette
    /// reference would resolve.
    pub colour_bytes: [u8; 4],
    /// Stored colour bytes (of the three that are used) with no entry in this
    /// platform's table, in file order and without duplicates.
    ///
    /// **Not an error.** §8.3: "any other value is unrecognised and an
    /// implementer must surface it rather than invent a colour" — so the
    /// picture decodes, the unknown value draws as black, and the fact travels
    /// with the picture for a host to put in a diagnostic. One of the
    /// *Hulk*'s own records still needs it: `R01012` stores 153 for pixel
    /// value 2, which §8.3's table does not list. (`B01250R`'s 232 did too
    /// until a real-machine capture settled it as yellow — SQ-1491.)
    /// On the Atari it is most bytes; see [`atari_colour`].
    pub unrecognised_colours: Vec<u8>,
    /// The canvas rectangle this record's own pixels actually cover, or
    /// `None` for a record that painted nothing at all.
    ///
    /// **Only an overlay needs this, and an overlay cannot be drawn without
    /// it.** §12.11 has the object pictures of the items in a room drawn OVER
    /// the room picture, and every one of them is a sub-image on the shared
    /// canvas — so a host compositing one must copy the sub-image's own
    /// rectangle and leave the rest of the room picture alone. It cannot work
    /// that rectangle out from [`Self::pixels`], because "untouched" and
    /// "painted black" are the same value 0 (§8.3 forces value 0 to black),
    /// and the *Hulk*'s room pictures are mostly black.
    ///
    /// Measured from the writes themselves rather than derived from the
    /// header, so a record whose data runs out early reports the region it
    /// really painted and not the one it promised. Bounds are **inclusive**
    /// and always inside the canvas.
    pub painted: Option<Painted>,
}

/// The canvas rectangle one record's pixels cover — see [`Picture::painted`].
///
/// Inclusive on all four sides, and always within
/// `0..`[`CANVAS_WIDTH`] x `0..`[`CANVAS_HEIGHT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Painted {
    /// Leftmost column painted.
    pub left: usize,
    /// Topmost row painted.
    pub top: usize,
    /// Rightmost column painted, inclusive.
    pub right: usize,
    /// Bottommost row painted, inclusive.
    pub bottom: usize,
}

/// Accumulates [`Painted`] as a decoder writes, so a decoder's own inner loop
/// says "I wrote here" and nothing has to re-derive it from a header whose
/// promises the data may not keep.
#[derive(Default)]
pub(crate) struct PaintedBox(Option<Painted>);

impl PaintedBox {
    /// Note that `(x, y)` was written. Both must already be on the canvas.
    pub(crate) fn mark(&mut self, x: usize, y: usize) {
        match &mut self.0 {
            None => self.0 = Some(Painted { left: x, top: y, right: x, bottom: y }),
            Some(p) => {
                p.left = p.left.min(x);
                p.top = p.top.min(y);
                p.right = p.right.max(x);
                p.bottom = p.bottom.max(y);
            }
        }
    }

    /// What was painted, or `None` if nothing was.
    pub(crate) fn finish(self) -> Option<Painted> {
        self.0
    }
}

impl Picture {
    /// The RGB of the pixel at `(x, y)`, or `None` off the canvas.
    pub fn rgb(&self, x: usize, y: usize) -> Option<Rgb> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let v = *self.pixels.get(y * self.width + x)?;
        Some(self.palette[usize::from(v) & 3])
    }
}

/// Decode one family-C record (§8.3).
///
/// `bytes` is the whole record, starting with the two-byte load address: on the
/// Commodore 64 exactly the bytes of the named file as the disk's block chain
/// gives them; on the Atari the range that "starts two bytes before the listed
/// offset" and runs for the little-endian word at that offset plus two.
///
/// # The format, as implemented
///
/// **Header, twelve bytes.** 0-1 the load address, ignored; 2-3 a
/// little-endian data size, informational; 4 the left edge in 8-pixel columns
/// **plus 3**; 5 the top row; 6 the right edge in columns **plus 3**; 7 the
/// bottom row; 8-11 four colour bytes. Data runs from byte 12 to two bytes
/// before the end of the record.
///
/// **Pixels.** Two bits each, four pixels to a byte, most significant pair
/// leftmost, and each stored pixel is two device pixels wide — so one byte
/// paints eight horizontal positions.
///
/// **Storage order is column-major in 8-pixel strips.** Bytes come in pairs
/// and a pair paints two consecutive rows of one column, first byte upper. The
/// row counter then advances by two, and **when it passes the bottom row** the
/// column advances by one 8-pixel step and the row counter returns to the top
/// row. Writing stops once the column is past the right edge; further pairs
/// are consumed and discarded.
///
/// Both limits are **inclusive**, which is the one place this differs from
/// §8.3's wording and is what every specimen says: a full-canvas *Hulk* record
/// declares left 0, top 0, right 280, bottom 158 and holds exactly
/// 36 columns x 80 pairs of bytes. 36 columns is `0, 8, .., 280` inclusive and
/// 80 pairs is rows `0, 2, .., 158` inclusive, whose last pair paints rows 158
/// and 159 — hence [`CANVAS_HEIGHT`] of 160. Read either limit exclusively and
/// each column is one pair short of the data, which shifts every column two
/// rows further down than the last: the picture comes out sheared, and on the
/// *Hulk*'s title screen that reads as a plausible-looking image with the
/// wordmark sliced in half.
///
/// **Compression**, a byte-pair run-length scheme repeated until the data is
/// exhausted: read a control byte; if bit 7 is set the repeat count is the low
/// seven bits plus one and the next two bytes are a pixel pair emitted that
/// many times; if bit 7 is clear the literal count is the byte plus one and
/// that many pixel pairs follow, each emitted once.
///
/// §8.3 also describes a variant with no literal mode, used by *The Count* and
/// *Voodoo Castle*; it is not implemented, because those two titles' pictures
/// live on Atari and Apple II media this crate cannot yet address (§12.10: the
/// per-title offset lists "are not recoverable from the database", and nothing
/// in the images supplies them).
///
/// **Colour** is [`c64_colour`] or [`atari_colour`] by `platform`; entry 0 is
/// forced to black and the fourth stored byte is never used.
///
/// # Errors
///
/// [`PictureError`] — too short for a header, a placement covering no region,
/// or a platform that does not use family C at all.
pub fn decode_family_c(bytes: &[u8], platform: SagaPlatform) -> Result<Picture, PictureError> {
    let resolve: fn(u8) -> Option<Rgb> = match platform {
        SagaPlatform::Commodore64 => c64_colour,
        SagaPlatform::Atari8Bit => atari_colour,
        SagaPlatform::AppleII => return Err(PictureError::NotFamilyC { platform }),
    };
    if bytes.len() < 14 {
        return Err(PictureError::TooShort { len: bytes.len() });
    }
    let left = (i32::from(bytes[4]) - 3) * 8;
    let top = i32::from(bytes[5]);
    let right = (i32::from(bytes[6]) - 3) * 8;
    let bottom = i32::from(bytes[7]);
    if right < left || bottom < top {
        return Err(PictureError::EmptyRegion { left, right, top, bottom });
    }

    let mut pixels = vec![0u8; CANVAS_WIDTH * CANVAS_HEIGHT];
    let mut painted = PaintedBox::default();
    let mut x = left;
    let mut y = top;
    // One pair: two bytes, the first to row `y` and the second to row `y + 1`
    // of column `x`. Off-canvas pixels are dropped rather than refused — the
    // left edge is column -1 on two of the *Hulk*'s records (`R01000` and
    // `R01020` both declare `header[4]` = 2) and the region's last row is one
    // past the declared bottom, and in every specimen the pixels that fall
    // outside are value 0.
    let mut emit = |x: &mut i32, y: &mut i32, hi: u8, lo: u8| {
        if *x > right {
            return;
        }
        for (row, byte) in [(*y, hi), (*y + 1, lo)] {
            if row < 0 || row >= CANVAS_HEIGHT as i32 {
                continue;
            }
            for pair in 0..4i32 {
                let value = (byte >> (6 - 2 * pair)) & 3;
                for half in 0..2i32 {
                    let px = *x + pair * 2 + half;
                    if (0..CANVAS_WIDTH as i32).contains(&px) {
                        pixels[row as usize * CANVAS_WIDTH + px as usize] = value;
                        painted.mark(px as usize, row as usize);
                    }
                }
            }
        }
        *y += 2;
        if *y > bottom {
            *y = top;
            *x += 8;
        }
    };

    let data = &bytes[12..bytes.len() - 2];
    let mut i = 0usize;
    while i < data.len() {
        let control = data[i];
        i += 1;
        if control & 0x80 != 0 {
            if i + 2 > data.len() {
                break;
            }
            let (hi, lo) = (data[i], data[i + 1]);
            i += 2;
            for _ in 0..u16::from(control & 0x7f) + 1 {
                emit(&mut x, &mut y, hi, lo);
            }
        } else {
            let mut left_to_read = u16::from(control) + 1;
            while left_to_read > 0 {
                if i + 2 > data.len() {
                    break;
                }
                emit(&mut x, &mut y, data[i], data[i + 1]);
                i += 2;
                left_to_read -= 1;
            }
            if left_to_read > 0 {
                break;
            }
        }
    }

    let colour_bytes = [bytes[8], bytes[9], bytes[10], bytes[11]];
    let mut palette = [(0u8, 0u8, 0u8); 4];
    let mut unrecognised_colours = Vec::new();
    for (slot, stored) in colour_bytes[..3].iter().enumerate() {
        match resolve(*stored) {
            Some(rgb) => palette[slot + 1] = rgb,
            None => {
                if !unrecognised_colours.contains(stored) {
                    unrecognised_colours.push(*stored);
                }
            }
        }
    }
    Ok(Picture {
        width: CANVAS_WIDTH,
        height: CANVAS_HEIGHT,
        pixels,
        palette,
        colour_bytes,
        unrecognised_colours,
        painted: painted.finish(),
    })
}

/// The Commodore 64 colour a stored colour byte means (§8.3), or `None` for a
/// value neither §8.3's table nor a real-machine capture settles.
///
/// # The byte is an *Atari* colour value (SQ-1491)
///
/// §8.3 calls this "a bare lookup with no arithmetic structure" and it is not
/// quite: Family C is the **shared** Commodore 64 / Atari 8-bit format, and the
/// byte is encoded the way §8.3 already says the *Atari* encodes one —
/// `hue x 16 + luminance`. The Commodore 64 release keeps its Atari sibling's
/// colour bytes and resolves each to the nearest VIC-II colour, so the low
/// nibble is brightness and the high nibble is hue. Regroup §8.3's table that
/// way and it comes out in clean hue bands, luminance-ordered within each —
/// hue 0 the grey ramp, hues 1-2 gold and brown, 3-4 orange through red, 5-6
/// purple, 7-9 blue, 11-13 green, 14-15 yellow and brown.
///
/// That is why the arms below are grouped by hue: it makes the two things a
/// reader needs visible at a glance — which values a capture has settled, and
/// where a band's boundary is still guesswork. It is **not** implemented as
/// arithmetic, because doing so needs a full 256-entry Atari palette to take
/// the nearest VIC-II colour of, and §8.3 is explicit that that table "must be
/// transcribed from an Atari palette reference; it cannot responsibly be
/// reconstructed from prose". See [`atari_colour`], which has the same gap.
///
/// # What a real machine settles, and what it does not
///
/// Ten bytes are **measured**, from the *Hulk*'s own Commodore 64 release
/// running under VICE (`machine-screenshots/c64-hulk-{splash,start,transform,
/// chamber}.png`, and see `scott_c64_picture_colours` in lanthorn's test
/// suites, which re-derives them from the frames on every run): every pixel of
/// every stored value in all four frames resolves to exactly one VIC-II
/// colour, with no exceptions. They are 14 and 142 white, 50 and 66 red, 56
/// orange, 101 and 103 purple, 135 blue, 196 and 198 green — plus 232 yellow
/// off `c64-hulk-colorbars.png`. Eight of those confirm §8.3 unchanged; **50
/// and 66 correct it** (§8.3 read both as orange), and 232 was not in it at
/// all.
///
/// Every other arm is §8.3's, **unverified**, and two of them the encoding
/// says are wrong: 0 and 224 are luminance 0, which is black rather than the
/// purple §8.3 gives them, and 1 and 7 are hue 0, which is grey rather than
/// blue. They are left as §8.3 has them because no capture settles them and a
/// plausible correction is still a guess; the doc comment is the record, and
/// the frame that would settle each is listed in the appendix item §8.3 names.
///
/// A value outside the table is surfaced rather than guessed at; see
/// [`Picture::unrecognised_colours`].
pub fn c64_colour(stored: u8) -> Option<Rgb> {
    use crate::c64_palette::PEPTO_PALETTE as P;
    const WHITE: Rgb = P[1];
    const RED: Rgb = P[2];
    const PURPLE: Rgb = P[4];
    const GREEN: Rgb = P[5];
    const BLUE: Rgb = P[6];
    const YELLOW: Rgb = P[7];
    const ORANGE: Rgb = P[8];
    // Not family A's brown (§8.3 says so explicitly).
    const BROWN: Rgb = P[9];
    const LIGHT_RED: Rgb = P[10];
    const GREY: Rgb = P[12];
    const LIGHT_GREEN: Rgb = P[13];
    const LIGHT_BLUE: Rgb = P[14];
    Some(match stored {
        // hue 0 — the grey ramp. 14 is measured; 0 (luminance 0) and 1 and 7
        // (a hueless mid-grey) are §8.3's and the encoding disputes them.
        0 => PURPLE,
        1 | 7 => BLUE,
        2..=4 | 8..=10 | 12 | 14 | 15 => WHITE,
        // hue 1 — gold. 17 and 20 are §8.3's, and dark gold reading as green
        // is the same shape of doubt as 0 and 1 above.
        16 | 24 | 26 | 30 => YELLOW,
        17 | 20 => GREEN,
        // hue 2 — orange-brown, brightening to gold.
        35 | 36 | 38 | 40 => BROWN,
        46 => YELLOW,
        // hue 3 — orange. 50 (luminance 2) is measured RED and 56 (luminance
        // 8) measured orange, so the band turns somewhere in 51-54 and §8.3's
        // reading of those four as orange is unverified.
        50 => RED,
        51..=54 | 56 | 58..=60 | 62 => ORANGE,
        // hue 4 — red. 66 is measured, which makes luminance 2-7 uniformly red
        // where §8.3 had 66 alone as orange.
        66..=71 => RED,
        77 => PURPLE,
        // hues 5 and 6 — purple. 101 and 103 are measured.
        81 | 84..=87 | 97 | 101..=103 | 105 => PURPLE,
        89 => LIGHT_RED,
        110 => LIGHT_BLUE,
        // hues 7-9 — blue, going pale at the top of each ramp. 135 and 142
        // are measured.
        116 | 135 | 148 | 151 => BLUE,
        137 | 142 => WHITE,
        157 => LIGHT_BLUE,
        // hue 10 — one lone dark value §8.3 reads as grey.
        161 => GREY,
        // hues 11-13 — green. 196 and 198 are measured.
        179 | 182 | 183 | 194..=200 | 212 | 214..=216 => GREEN,
        201 => LIGHT_GREEN,
        // hues 14 and 15 — gold and brown. 232 is measured; 224 is luminance
        // 0, the same doubt as 0 above.
        224 => PURPLE,
        230 | 232 | 237 | 238 | 252 => YELLOW,
        244 | 246 | 248 => BROWN,
        255 => WHITE,
        _ => return None,
    })
}

/// The Atari 8-bit colour a stored colour byte means, **as far as §8.3 states
/// it**, or `None`.
///
/// An Atari colour byte indexes a 256-entry hardware palette laid out as
/// hue x 16 + luminance. §8.3 gives that table's **luminance row** (hue 0,
/// indices 0-15) value by value, and enumerates the sixteen entries that are
/// hand-substituted rather than taken from the hardware — and then says the
/// base table "must be transcribed from an Atari palette reference; it cannot
/// responsibly be reconstructed from prose". This implementation has no such
/// reference, so it answers exactly the entries §8.3 does state and `None` for
/// the other 224, which surfaces the gap instead of inventing 224 colours.
///
/// That is not a limitation anything can currently reach: Atari pictures live
/// on the companion picture side at hard-coded offsets that §12.10 says are
/// "not recoverable from the database", so no Atari record is addressable yet.
pub fn atari_colour(stored: u8) -> Option<Rgb> {
    // §8.3's sixteen hand-substituted entries, which override the hardware
    // table wherever they collide with it.
    let substituted = match stored {
        14 => Some((0xE0, 0xE0, 0xE0)),
        18 | 37 | 50 | 54 | 58 | 247 => Some((0xAD, 0x5F, 0x64)),
        86 => Some((0x4B, 0x1E, 0xAD)),
        133 => Some((0x34, 0x68, 0xEE)),
        198 => Some((0x2B, 0x58, 0x00)),
        199 => Some((0x3A, 0x67, 0x00)),
        216 => Some((0x63, 0x70, 0x00)),
        228 => Some((0x94, 0x4C, 0x02)),
        248 => Some((0x8D, 0x59, 0x00)),
        255 => Some((0xBA, 0x86, 0x00)),
        _ => None,
    };
    if substituted.is_some() {
        return substituted;
    }
    // The luminance-only row: hue 0, one grey per luminance, each value used
    // for all three channels (§8.3).
    const GREYS: [u8; 16] = [
        0x00, 0x0E, 0x1D, 0x2C, 0x3B, 0x4A, 0x59, 0x68, 0x77, 0x86, 0x95, 0xA4, 0xB3, 0xC2, 0xE0,
        0xE0,
    ];
    if stored < 16 {
        let g = GREYS[usize::from(stored)];
        return Some((g, g, g));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c64_palette::PEPTO_PALETTE;

    /// Build a family-C record by hand: a 12-byte header, then a literal run
    /// of `pairs`, then the two-byte tail every record carries.
    fn record(header: [u8; 12], pairs: &[(u8, u8)]) -> Vec<u8> {
        let mut out = header.to_vec();
        assert!(pairs.len() <= 128, "one literal control byte holds at most 128 pairs");
        out.push(u8::try_from(pairs.len() - 1).unwrap());
        for (hi, lo) in pairs {
            out.push(*hi);
            out.push(*lo);
        }
        out.extend_from_slice(&[0, 0]);
        out
    }

    // A hand-built one-column, four-row strip, with the expected pixels worked
    // out by hand from §8.3's rules rather than from any specimen.
    //
    // Header: left column 3 (stored 3 + 3 = 6 → x 24), top row 4, right column
    // 3 (so one column only), bottom row 6 (so rows 4, 5, 6, 7 — two pairs,
    // because the bottom is inclusive and each pair covers two rows). Colours
    // 14 (white), 67 (red), 17 (green) and an unused fourth.
    //
    // Pair one is (0b00_01_10_11, 0b11_10_01_00): row 4 reads values
    // 0,1,2,3 left to right and row 5 reads 3,2,1,0, each value two pixels
    // wide. Pair two is (0xFF, 0x00): row 6 all value 3, row 7 all value 0.
    #[test]
    fn hand_built_strip_decodes_to_the_pixels_8_3_describes() {
        let rec = record(
            [0x00, 0x50, 0, 0, 6, 4, 6, 6, 14, 67, 17, 0],
            &[(0b00_01_10_11, 0b11_10_01_00), (0xFF, 0x00)],
        );
        let pic = decode_family_c(&rec, SagaPlatform::Commodore64).expect("decodes");
        assert_eq!((pic.width, pic.height), (CANVAS_WIDTH, CANVAS_HEIGHT));
        assert_eq!(
            pic.palette,
            [PEPTO_PALETTE[0], PEPTO_PALETTE[1], PEPTO_PALETTE[2], PEPTO_PALETTE[5]],
            "black, white, red, green"
        );
        assert!(pic.unrecognised_colours.is_empty());

        let px = |x: usize, y: usize| pic.pixels[y * CANVAS_WIDTH + x];
        // The column starts at x = (6 - 3) * 8 = 24 and is eight pixels wide.
        for (pair, want) in [0u8, 1, 2, 3].iter().enumerate() {
            assert_eq!(px(24 + pair * 2, 4), *want, "row 4 pair {pair}");
            assert_eq!(px(24 + pair * 2 + 1, 4), *want, "row 4 pair {pair}, second half");
            assert_eq!(px(24 + pair * 2, 5), 3 - *want, "row 5 pair {pair}");
        }
        for x in 24..32 {
            assert_eq!(px(x, 6), 3, "row 6 is all value 3");
            assert_eq!(px(x, 7), 0, "row 7 is all value 0");
        }
        // Nothing outside the strip was touched.
        assert_eq!(px(23, 4), 0, "one pixel left of the strip");
        assert_eq!(px(32, 4), 0, "one pixel right of the strip");
        assert_eq!(px(24, 3), 0, "the row above the strip");
        assert_eq!(px(24, 8), 0, "the row below the strip");
        assert_eq!(pic.rgb(24, 4), Some(PEPTO_PALETTE[0]), "value 0 is black");
        assert_eq!(pic.rgb(26, 4), Some(PEPTO_PALETTE[1]), "value 1 is colour byte 8");
        assert_eq!(pic.rgb(30, 4), Some(PEPTO_PALETTE[5]), "value 3 is colour byte 10");
        assert_eq!(pic.rgb(CANVAS_WIDTH, 0), None, "off the right edge");
    }

    // The column advances only after the row counter passes the INCLUSIVE
    // bottom row, so a region whose rows are 4..=6 takes two pairs per column
    // and not one. Two columns, two pairs each, and the second column's pixels
    // must land eight pixels right of the first — this is the arithmetic whose
    // exclusive reading shears every picture.
    #[test]
    fn a_column_takes_pairs_up_to_and_including_the_bottom_row() {
        let rec = record(
            [0x00, 0x50, 0, 0, 6, 4, 7, 6, 14, 67, 17, 0],
            &[(0xFF, 0xFF), (0x00, 0x00), (0x55, 0x55), (0x00, 0x00)],
        );
        let pic = decode_family_c(&rec, SagaPlatform::Commodore64).expect("decodes");
        let px = |x: usize, y: usize| pic.pixels[y * CANVAS_WIDTH + x];
        assert_eq!(px(24, 4), 3, "column one, first pair");
        assert_eq!(px(24, 6), 0, "column one, second pair");
        assert_eq!(px(32, 4), 1, "column two starts eight pixels right");
        assert_eq!(px(32, 6), 0, "column two, second pair");
    }

    // `Picture::painted` is the rectangle the record's own pixels cover, and
    // it is what an overlay is composited through (§12.11, SQ-1482). It is
    // measured from the WRITES, not from the header, so it reports the four
    // rows a two-pair column really paints rather than the three the
    // inclusive bottom row promises — and a record whose value-0 pixels are
    // indistinguishable from an untouched canvas still knows where it is.
    #[test]
    fn painted_reports_the_rectangle_the_record_covers() {
        let rec = record(
            [0x00, 0x50, 0, 0, 6, 4, 7, 6, 14, 67, 17, 0],
            &[(0xFF, 0xFF), (0x00, 0x00), (0x55, 0x55), (0x00, 0x00)],
        );
        let pic = decode_family_c(&rec, SagaPlatform::Commodore64).expect("decodes");
        assert_eq!(
            pic.painted,
            Some(Painted { left: 24, top: 4, right: 39, bottom: 7 }),
            "two eight-pixel columns from x 24, rows 4 through 7"
        );
        // Row 7 is entirely value 0 and is inside the rectangle all the same:
        // "painted black" is a pixel the record drew, and the canvas outside
        // is the same value with nothing behind it.
        assert_eq!(pic.pixels[7 * CANVAS_WIDTH + 24], 0);

        // A record whose placement covers no region is refused before this
        // ever runs (see `refusals`), so `None` is reachable only from a
        // record with no data at all.
        let empty = {
            let mut r = [0x00u8, 0x50, 0, 0, 6, 4, 6, 6, 14, 67, 17, 0].to_vec();
            r.extend_from_slice(&[0, 0]);
            r
        };
        assert_eq!(
            decode_family_c(&empty, SagaPlatform::Commodore64).expect("decodes").painted,
            None,
            "a record that paints nothing has no rectangle"
        );
    }

    // Bit 7 set is a repeat: the count is the low seven bits plus one, and the
    // ONE following pair is emitted that many times.
    #[test]
    fn a_repeat_control_byte_emits_one_pair_many_times() {
        let mut rec = vec![0x00, 0x50, 0, 0, 6, 0, 6, 5, 14, 67, 17, 0];
        rec.extend_from_slice(&[0x80 | 2, 0xFF, 0xFF]); // three copies
        rec.extend_from_slice(&[0, 0]);
        let pic = decode_family_c(&rec, SagaPlatform::Commodore64).expect("decodes");
        let px = |x: usize, y: usize| pic.pixels[y * CANVAS_WIDTH + x];
        // Rows 0..=5 is three pairs, so one column exactly.
        for y in 0..6 {
            assert_eq!(px(24, y), 3, "row {y} of the repeated column");
        }
        assert_eq!(px(24, 6), 0, "one row past the region");
    }

    // §8.3: entry 0 is forced to black whatever the file says, and the fourth
    // stored byte is never used.
    #[test]
    fn palette_entry_zero_is_black_and_the_fourth_byte_is_ignored() {
        let rec = record([0x00, 0x50, 0, 0, 3, 0, 3, 1, 14, 14, 14, 67], &[(0, 0)]);
        let pic = decode_family_c(&rec, SagaPlatform::Commodore64).expect("decodes");
        assert_eq!(pic.palette[0], (0, 0, 0), "value 0 is black however 8-11 read");
        assert_eq!(pic.palette, [PEPTO_PALETTE[0], PEPTO_PALETTE[1], PEPTO_PALETTE[1], PEPTO_PALETTE[1]]);
        assert_eq!(pic.colour_bytes, [14, 14, 14, 67], "all four are reported");
    }

    // An unrecognised colour byte is surfaced, not guessed at — and the
    // picture still decodes. 153 is the *Hulk*'s own `R01012`.
    #[test]
    fn an_unrecognised_colour_byte_is_reported_and_draws_black() {
        let rec = record([0x00, 0x50, 0, 0, 3, 0, 3, 1, 56, 153, 14, 0], &[(0, 0)]);
        let pic = decode_family_c(&rec, SagaPlatform::Commodore64).expect("decodes");
        assert_eq!(pic.unrecognised_colours, vec![153]);
        assert_eq!(pic.palette[2], (0, 0, 0), "unresolved draws as black");
        assert_eq!(pic.palette[1], PEPTO_PALETTE[8], "the recognised ones still resolve");
    }

    #[test]
    fn refusals() {
        assert_eq!(
            decode_family_c(&[0; 13], SagaPlatform::Commodore64),
            Err(PictureError::TooShort { len: 13 })
        );
        // Right edge left of the left edge.
        let rec = record([0x00, 0x50, 0, 0, 9, 0, 4, 4, 14, 67, 17, 0], &[(0, 0)]);
        assert_eq!(
            decode_family_c(&rec, SagaPlatform::Commodore64),
            Err(PictureError::EmptyRegion { left: 48, right: 8, top: 0, bottom: 4 })
        );
        // Bottom above the top.
        let rec = record([0x00, 0x50, 0, 0, 4, 9, 9, 4, 14, 67, 17, 0], &[(0, 0)]);
        assert!(matches!(
            decode_family_c(&rec, SagaPlatform::Commodore64),
            Err(PictureError::EmptyRegion { top: 9, bottom: 4, .. })
        ));
        // The Apple II is family D.
        let rec = record([0x00, 0x50, 0, 0, 3, 0, 3, 1, 14, 67, 17, 0], &[(0, 0)]);
        assert_eq!(
            decode_family_c(&rec, SagaPlatform::AppleII),
            Err(PictureError::NotFamilyC { platform: SagaPlatform::AppleII })
        );
    }

    // A truncated data run must stop, not spin: a literal control byte
    // promising more pairs than the record holds is the shape that loops
    // forever if the outer reader never advances.
    #[test]
    fn a_truncated_literal_run_terminates() {
        let mut rec = vec![0x00, 0x50, 0, 0, 3, 0, 39, 158, 14, 67, 17, 0];
        rec.extend_from_slice(&[127, 0xFF]); // 128 pairs promised, one byte given
        rec.extend_from_slice(&[0, 0]);
        let pic = decode_family_c(&rec, SagaPlatform::Commodore64).expect("decodes what it has");
        assert!(pic.pixels.iter().all(|&v| v == 0), "nothing was completed");
    }

    // SQ-1491's ground truth: the eleven bytes a real Commodore 64 running the
    // *Hulk*'s own release has been photographed resolving, with the frame that
    // settles each. `scott_c64_picture_colours` in lanthorn's suites re-derives
    // these from the captures themselves; this is the same table pinned where
    // the decoder can be read.
    #[test]
    fn the_colour_bytes_real_machine_captures_settle() {
        for (byte, index, frame) in [
            (14u8, 1usize, "start/transform/chamber, white"),
            (142, 1, "splash, white"),
            (50, 2, "chamber, red — §8.3 read this as orange"),
            (66, 2, "chamber's B01053R overlay, red — §8.3 read this as orange"),
            (56, 8, "start, orange"),
            (101, 4, "transform, purple"),
            (103, 4, "splash and start, purple"),
            (135, 6, "chamber, blue"),
            (196, 5, "transform, green"),
            (198, 5, "splash, green"),
            (232, 7, "colorbars, yellow — §8.3's table has no entry for it"),
        ] {
            assert_eq!(
                c64_colour(byte),
                Some(PEPTO_PALETTE[index]),
                "byte {byte} (Atari hue {}, luminance {}): {frame}",
                byte >> 4,
                byte & 15
            );
        }
    }

    #[test]
    fn c64_colour_table_matches_8_3() {
        assert_eq!(c64_colour(35), Some(PEPTO_PALETTE[9]), "brown, not family A's");
        assert_eq!(c64_colour(89), Some(PEPTO_PALETTE[10]), "light red, the lone value");
        assert_eq!(c64_colour(161), Some(PEPTO_PALETTE[12]), "grey, the lone value");
        assert_eq!(c64_colour(201), Some(PEPTO_PALETTE[13]), "light green, the lone value");
        assert_eq!(c64_colour(0), Some(PEPTO_PALETTE[4]), "0 is purple, not black");
        assert_eq!(c64_colour(153), None, "R01012's second byte");
        assert_eq!(c64_colour(18), None, "not a value §8.3 lists");
    }

    #[test]
    fn atari_colour_answers_only_what_8_3_states() {
        assert_eq!(atari_colour(0), Some((0, 0, 0)), "luminance 0");
        assert_eq!(atari_colour(2), Some((0x1D, 0x1D, 0x1D)), "luminance 2");
        assert_eq!(atari_colour(14), Some((0xE0, 0xE0, 0xE0)), "the substituted entry");
        assert_eq!(atari_colour(86), Some((0x4B, 0x1E, 0xAD)), "a substituted entry above the row");
        assert_eq!(atari_colour(16), None, "hue 1 needs the transcribed table");
        assert_eq!(atari_colour(135), None, "and so does most of the palette");
    }
}
