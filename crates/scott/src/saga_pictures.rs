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

/// Which of family C's two run-length schemes a release's records use (§8.3).
///
/// §8.3 states the difference in one sentence — "**The Count and Voodoo Castle
/// use a variant with no literal mode**: every control byte is a repeat count,
/// bit 7 is not masked, the count is the byte's full value, and two is
/// subtracted from the stored height before decoding" — and that is the whole
/// of the compression difference. **It is not the whole of the difference**:
/// measured on those two titles' own Atari records (SQ-1484), the variant also
/// reads both of the header's far edges as **exclusive** limits where the
/// standard scheme reads them as inclusive, which is one column narrower as
/// well as the two rows shorter §8.3 names. See [`StripLayout::resolve`].
///
/// **Never sniffed.** Which scheme a record uses is a property of the
/// *release*, not of its bytes: a no-literal record read as standard decodes
/// into something that still looks like a picture, because a control byte
/// below 0x80 is a plausible literal count and the pixels that follow are
/// plausible pixels. [`crate::SagaUs::picture_scheme`] is the only place the
/// question is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FamilyCScheme {
    /// Bit 7 set is a repeat of the count's low seven bits plus one; bit 7
    /// clear is a literal run of the byte plus one pairs. Both header edges
    /// are inclusive. The Commodore 64 releases, and *Claymorgue Castle* on
    /// the Atari.
    Standard,
    /// Every control byte is a full repeat count with no literal mode, and
    /// both header edges are exclusive. *The Count* and *Voodoo Castle*.
    NoLiteral,
}

/// Where on the canvas a record's strips land, and how many there are.
///
/// The four header edge bytes resolved once, so that the two schemes'
/// different readings of them (see [`FamilyCScheme`]) live in one place
/// instead of at every arithmetic site. `cols` and `pairs` are counts, not
/// limits: a record holds exactly `cols * pairs` byte pairs, and that product
/// is the strongest thing known about a family-C record — strong enough that
/// [`crate::saga_atari::scan_picture_side`] finds records on a raw disk side
/// with it and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StripLayout {
    /// Leftmost device-pixel column, `(header[4] - 3) * 8`. May be negative:
    /// two of the *Hulk*'s records declare `header[4]` of 2, and most Atari
    /// full-canvas records declare 2 as well.
    pub left: i32,
    /// Topmost pixel row.
    pub top: i32,
    /// How many 8-pixel columns the record paints.
    pub cols: i32,
    /// How many byte pairs each column holds; a pair paints two rows.
    pub pairs: i32,
}

impl StripLayout {
    /// Resolve the four header edge bytes under `scheme`.
    ///
    /// `left_col` and `right_col` are the stored column bytes (the edge in
    /// 8-pixel columns **plus 3**); `top` and `bottom` are stored pixel rows.
    ///
    /// # Errors
    ///
    /// [`PictureError::EmptyRegion`] when the edges describe no region.
    pub fn resolve(
        left_col: i32,
        top: i32,
        right_col: i32,
        bottom: i32,
        scheme: FamilyCScheme,
    ) -> Result<StripLayout, PictureError> {
        let left = (left_col - 3) * 8;
        let right = (right_col - 3) * 8;
        let empty = match scheme {
            // An inclusive limit equal to its own start is one column, or one
            // pair, and is a real region.
            FamilyCScheme::Standard => right < left || bottom < top,
            // An exclusive one is empty.
            FamilyCScheme::NoLiteral => right <= left || bottom <= top,
        };
        if empty {
            return Err(PictureError::EmptyRegion { left, right, top, bottom });
        }
        let (cols, pairs) = match scheme {
            FamilyCScheme::Standard => ((right - left) / 8 + 1, (bottom - top) / 2 + 1),
            FamilyCScheme::NoLiteral => ((right - left) / 8, (bottom - top) / 2),
        };
        Ok(StripLayout { left, top, cols, pairs })
    }

    /// `cols * pairs` — how many byte pairs a record with this layout holds.
    pub fn pair_count(&self) -> usize {
        (self.cols as usize) * (self.pairs as usize)
    }
}

/// What [`paint_strips`] produced.
#[derive(Debug, Clone)]
pub struct StripPaint {
    /// `CANVAS_WIDTH * CANVAS_HEIGHT` pixel values, each 0-3.
    pub pixels: Vec<u8>,
    /// How many byte pairs were emitted. Equal to
    /// [`StripLayout::pair_count`] for a complete record.
    pub emitted: usize,
    /// How many bytes of `data` were read to get there. The Atari scanner
    /// checks this against the record's own declared size.
    pub consumed: usize,
    /// The canvas rectangle the writes actually covered — [`Picture::painted`],
    /// measured here rather than derived from the header, so a record whose
    /// data runs out early reports the region it really painted.
    pub bounds: Option<Painted>,
}

/// Decode a family-C run-length stream onto the canvas (§8.3).
///
/// Stops as soon as `layout` is full — the standard scheme's own wording is
/// that "further pairs are consumed and discarded", and discarding them is
/// indistinguishable from not reading them — or when `data` runs out, which is
/// how a truncated record decodes to what it has instead of refusing.
pub fn paint_strips(data: &[u8], layout: &StripLayout, scheme: FamilyCScheme) -> StripPaint {
    let need = layout.pair_count();
    let mut pixels = vec![0u8; CANVAS_WIDTH * CANVAS_HEIGHT];
    let mut box_ = PaintedBox::default();
    let mut emitted = 0usize;
    let mut x = layout.left;
    let mut k = 0i32;
    let mut i = 0usize;

    while emitted < need && i < data.len() {
        let control = data[i];
        i += 1;
        // Read the whole unit before painting any of it, so `consumed` names a
        // unit boundary even when the region fills part way through a run.
        let run: Run = match scheme {
            FamilyCScheme::NoLiteral => {
                if i + 2 > data.len() {
                    break;
                }
                let pair = (data[i], data[i + 1]);
                i += 2;
                // "the count is the byte's full value" — and a stored zero
                // still paints one pair, which is what the specimens hold.
                Run::Repeat(usize::from(control).max(1), pair)
            }
            FamilyCScheme::Standard if control & 0x80 != 0 => {
                if i + 2 > data.len() {
                    break;
                }
                let pair = (data[i], data[i + 1]);
                i += 2;
                Run::Repeat(usize::from(control & 0x7f) + 1, pair)
            }
            FamilyCScheme::Standard => {
                let count = usize::from(control) + 1;
                if i + 2 * count > data.len() {
                    break;
                }
                let at = i;
                i += 2 * count;
                Run::Literal(at, count)
            }
        };
        let count = match run {
            Run::Repeat(n, _) => n,
            Run::Literal(_, n) => n,
        };
        for n in 0..count {
            if emitted >= need {
                break;
            }
            let (hi, lo) = match run {
                Run::Repeat(_, pair) => pair,
                Run::Literal(at, _) => (data[at + 2 * n], data[at + 2 * n + 1]),
            };
            let y0 = layout.top + 2 * k;
            for (row, byte) in [(y0, hi), (y0 + 1, lo)] {
                if row < 0 || row >= CANVAS_HEIGHT as i32 {
                    continue;
                }
                for pair in 0..4i32 {
                    let value = (byte >> (6 - 2 * pair)) & 3;
                    for half in 0..2i32 {
                        let px = x + pair * 2 + half;
                        if (0..CANVAS_WIDTH as i32).contains(&px) {
                            pixels[row as usize * CANVAS_WIDTH + px as usize] = value;
                            box_.mark(px as usize, row as usize);
                        }
                    }
                }
            }
            emitted += 1;
            k += 1;
            if k >= layout.pairs {
                k = 0;
                x += 8;
            }
        }
    }
    StripPaint { pixels, emitted, consumed: i, bounds: box_.finish() }
}

/// One decoded control unit: a pair repeated, or `n` pairs starting at an
/// offset into the data.
#[derive(Clone, Copy)]
enum Run {
    Repeat(usize, (u8, u8)),
    Literal(usize, usize),
}

/// Turn a record's four stored colour bytes into a palette (§8.3).
///
/// Entry 0 is forced to black whatever the record says, entries 1-3 are the
/// first three stored bytes through `resolve`, and the fourth stored byte is
/// never used. A byte `resolve` cannot name draws black and is reported in the
/// second half, which is §8.3's "surface it rather than invent a colour".
pub fn resolve_palette(colour_bytes: [u8; 4], resolve: fn(u8) -> Option<Rgb>) -> ([Rgb; 4], Vec<u8>) {
    let mut palette = [(0u8, 0u8, 0u8); 4];
    let mut unrecognised = Vec::new();
    for (slot, stored) in colour_bytes[..3].iter().enumerate() {
        match resolve(*stored) {
            Some(rgb) => palette[slot + 1] = rgb,
            None => {
                if !unrecognised.contains(stored) {
                    unrecognised.push(*stored);
                }
            }
        }
    }
    (palette, unrecognised)
}

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
    /// with the picture for a host to put in a diagnostic. Two of the
    /// *Hulk*'s own records need it: `R01012` stores 153 for pixel value 2 and
    /// `B01250R` stores 232 for pixel value 3, and §8.3's table has neither.
    /// **Always empty on the Atari**, whose colour bytes index hardware and
    /// are therefore all nameable; see [`atari_colour`].
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
/// *Voodoo Castle*. It is [`FamilyCScheme::NoLiteral`], and it is reached
/// through [`crate::saga_atari::decode_record`] rather than through this
/// function: those two titles' records live on an Atari companion side whose
/// header is ten bytes, not this one's twelve (see [`crate::saga_atari`]).
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
        SagaPlatform::Atari8Bit => |stored| Some(atari_colour(stored)),
        SagaPlatform::AppleII => return Err(PictureError::NotFamilyC { platform }),
    };
    if bytes.len() < 14 {
        return Err(PictureError::TooShort { len: bytes.len() });
    }
    // Off-canvas pixels are dropped rather than refused — the left edge is
    // column -1 on two of the *Hulk*'s records (`R01000` and `R01020` both
    // declare `header[4]` = 2) and the region's last row is one past the
    // declared bottom, and in every specimen the pixels that fall outside are
    // value 0.
    let layout = StripLayout::resolve(
        i32::from(bytes[4]),
        i32::from(bytes[5]),
        i32::from(bytes[6]),
        i32::from(bytes[7]),
        FamilyCScheme::Standard,
    )?;
    let data = &bytes[12..bytes.len() - 2];
    let strips = paint_strips(data, &layout, FamilyCScheme::Standard);
    let colour_bytes = [bytes[8], bytes[9], bytes[10], bytes[11]];
    let (palette, unrecognised_colours) = resolve_palette(colour_bytes, resolve);
    Ok(Picture {
        width: CANVAS_WIDTH,
        height: CANVAS_HEIGHT,
        pixels: strips.pixels,
        palette,
        colour_bytes,
        unrecognised_colours,
        painted: strips.bounds,
    })
}

/// The Commodore 64 colour a stored colour byte means (§8.3), or `None` for a
/// value §8.3's table does not list.
///
/// **These are not palette indices.** §8.3: their meaning "was recovered
/// empirically and the mapping is a bare lookup with no arithmetic structure",
/// onto thirteen colours — so this is a `match` on the recognised values and
/// nothing more, and a value outside it must be surfaced rather than guessed
/// at. Two of the *Hulk*'s own records fall outside it; see
/// [`Picture::unrecognised_colours`].
pub fn c64_colour(stored: u8) -> Option<Rgb> {
    const WHITE: Rgb = (255, 255, 255);
    const RED: Rgb = (191, 97, 72);
    const PURPLE: Rgb = (177, 89, 185);
    const GREEN: Rgb = (121, 213, 112);
    const BLUE: Rgb = (95, 72, 233);
    const YELLOW: Rgb = (247, 255, 108);
    const ORANGE: Rgb = (186, 134, 32);
    // Not family A's brown (§8.3 says so explicitly).
    const BROWN: Rgb = (131, 112, 0);
    const LIGHT_RED: Rgb = (231, 154, 132);
    const GREY: Rgb = (167, 167, 167);
    const LIGHT_GREEN: Rgb = (192, 255, 185);
    const LIGHT_BLUE: Rgb = (162, 143, 255);
    Some(match stored {
        2 | 3 | 4 | 8 | 9 | 10 | 12 | 14 | 15 | 137 | 142 | 255 => WHITE,
        35 | 36 | 38 | 40 | 244 | 246 | 248 => BROWN,
        16 | 24 | 26 | 30 | 46 | 230 | 237 | 238 | 252 => YELLOW,
        50..=54 | 56 | 58..=60 | 62 | 66 => ORANGE,
        67..=71 => RED,
        0 | 77 | 81 | 84..=87 | 97 | 101..=103 | 105 | 224 => PURPLE,
        89 => LIGHT_RED,
        1 | 7 | 116 | 135 | 148 | 151 => BLUE,
        110 | 157 => LIGHT_BLUE,
        161 => GREY,
        17 | 20 | 179 | 182 | 183 | 194..=200 | 212 | 214..=216 => GREEN,
        201 => LIGHT_GREEN,
        _ => return None,
    })
}

/// The luminance-only row of the Atari palette: hue 0, one grey per
/// luminance, each value used for all three channels (§8.3 states this row
/// value by value).
///
/// It is also the **luminance ramp of every other hue**, which is why it is a
/// named constant rather than a local: [`atari_colour`] takes a hue's
/// brightness from here and adds chroma to it.
pub const ATARI_LUMINANCE: [u8; 16] = [
    0x00, 0x0E, 0x1D, 0x2C, 0x3B, 0x4A, 0x59, 0x68, 0x77, 0x86, 0x95, 0xA4, 0xB3, 0xC2, 0xE0, 0xE0,
];

/// The Atari 8-bit colour a stored colour byte means.
///
/// # Where this comes from
///
/// **Not from §8.3**, which gives only the hue-0 row (above) and sixteen
/// hand-substituted entries, and then says the base table "must be transcribed
/// from an Atari palette reference; it cannot responsibly be reconstructed from
/// prose". This is that reference, taken from public hardware documentation
/// rather than from any interpreter:
///
/// - **Register layout** — a GTIA colour register holds the hue in bits 7-4
///   and the luminance in bits 3-1, bit 0 unused, and **hue 0 is chroma off**,
///   a grey at the register's luminance (*Atari 400/800 Hardware Manual*,
///   Atari Inc. 1982, part C016555, the `COLPF0`-`COLPF3`/`COLBK` register
///   descriptions). The 256-entry hue x 16 + luminance arrangement §8.3
///   describes is the conventional way to *tabulate* that byte, and it is what
///   this function indexes.
/// - **The fifteen hues** are fifteen phases of the NTSC colour subcarrier,
///   evenly spaced 360/15 = 24 degrees apart, which the same manual gives as
///   the sequence gold, orange, red-orange, pink, purple, purple-blue, blue,
///   blue, light blue, turquoise, green-blue, green, yellow-green,
///   orange-green, light orange for hues 1 to 15.
/// - **Chroma to RGB** is the standard NTSC YIQ matrix (FCC / SMPTE 170M).
///
/// [`PHASE_DEGREES`] is the burst offset that puts hue 1 on the manual's
/// "gold"; it is the one number here fixed by reading the sequence back rather
/// than quoted, and every other hue then lands on its documented name.
///
/// § 8.3's sixteen hand-substituted entries override the result wherever they
/// collide with it, so a release that relies on one still gets it.
///
/// **Total, unlike [`c64_colour`].** A Commodore 64 stored colour byte is not
/// a palette index at all and §8.3's lookup genuinely has holes; an Atari one
/// indexes hardware, and every one of the 256 has a colour.
pub fn atari_colour(stored: u8) -> Rgb {
    // §8.3's sixteen hand-substituted entries, which override the hardware
    // table wherever they collide with it.
    match stored {
        14 => return (0xE0, 0xE0, 0xE0),
        18 | 37 | 50 | 54 | 58 | 247 => return (0xAD, 0x5F, 0x64),
        86 => return (0x4B, 0x1E, 0xAD),
        133 => return (0x34, 0x68, 0xEE),
        198 => return (0x2B, 0x58, 0x00),
        199 => return (0x3A, 0x67, 0x00),
        216 => return (0x63, 0x70, 0x00),
        228 => return (0x94, 0x4C, 0x02),
        248 => return (0x8D, 0x59, 0x00),
        255 => return (0xBA, 0x86, 0x00),
        _ => {}
    }
    let hue = stored >> 4;
    let y = f64::from(ATARI_LUMINANCE[usize::from(stored & 0x0F)]) / 255.0;
    if hue == 0 {
        let g = ATARI_LUMINANCE[usize::from(stored & 0x0F)];
        return (g, g, g);
    }
    let theta = (PHASE_DEGREES + f64::from(hue - 1) * 24.0).to_radians();
    let i = SATURATION * theta.cos();
    let q = SATURATION * theta.sin();
    let channel = |v: f64| -> u8 { (v * 255.0).clamp(0.0, 255.0).round() as u8 };
    (
        channel(y + 0.956 * i + 0.621 * q),
        channel(y - 0.272 * i - 0.647 * q),
        channel(y - 1.106 * i + 1.703 * q),
    )
}

/// The colour-burst phase, in degrees, at which GTIA hue 1 sits.
///
/// The one number in [`atari_colour`] that is not quoted from a document.
/// Published derivations of this palette disagree about the burst offset by
/// tens of degrees, and the offset is what decides which *name* each hue gets;
/// -30 is the value at which the fifteen hues read back as the *Atari 400/800
/// Hardware Manual*'s own sequence, hue 1 gold through hue 15 light orange,
/// and `hue_names_read_back_in_the_manuals_order` is that check written down.
pub const PHASE_DEGREES: f64 = -30.0;

/// Chroma amplitude relative to full luminance, for [`atari_colour`].
pub const SATURATION: f64 = 0.30;

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(pic.palette, [(0, 0, 0), (255, 255, 255), (191, 97, 72), (121, 213, 112)]);
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
        assert_eq!(pic.rgb(24, 4), Some((0, 0, 0)), "value 0 is black");
        assert_eq!(pic.rgb(26, 4), Some((255, 255, 255)), "value 1 is colour byte 8");
        assert_eq!(pic.rgb(30, 4), Some((121, 213, 112)), "value 3 is colour byte 10");
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
        assert_eq!(pic.palette, [(0, 0, 0), (255, 255, 255), (255, 255, 255), (255, 255, 255)]);
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
        assert_eq!(pic.palette[1], (186, 134, 32), "the recognised ones still resolve");
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

    #[test]
    fn c64_colour_table_matches_8_3() {
        assert_eq!(c64_colour(14), Some((255, 255, 255)), "white");
        assert_eq!(c64_colour(35), Some((131, 112, 0)), "brown, not family A's");
        assert_eq!(c64_colour(89), Some((231, 154, 132)), "light red, the lone value");
        assert_eq!(c64_colour(161), Some((167, 167, 167)), "grey, the lone value");
        assert_eq!(c64_colour(201), Some((192, 255, 185)), "light green, the lone value");
        assert_eq!(c64_colour(0), Some((177, 89, 185)), "0 is purple, not black");
        assert_eq!(c64_colour(153), None, "R01012's second byte");
        assert_eq!(c64_colour(232), None, "B01250R's third byte");
    }

    #[test]
    fn atari_colour_keeps_8_3s_grey_row_and_substitutions() {
        assert_eq!(atari_colour(0), (0, 0, 0), "luminance 0");
        assert_eq!(atari_colour(2), (0x1D, 0x1D, 0x1D), "luminance 2");
        assert_eq!(atari_colour(0x0C), (0xB3, 0xB3, 0xB3), "luminance 12, hue 0 is grey");
        assert_eq!(atari_colour(14), (0xE0, 0xE0, 0xE0), "the substituted entry");
        assert_eq!(atari_colour(86), (0x4B, 0x1E, 0xAD), "a substituted entry above the row");
        assert_eq!(atari_colour(255), (0xBA, 0x86, 0x00), "the last substituted entry");
    }

    /// The hue names the *Atari 400/800 Hardware Manual* gives, read back off
    /// [`atari_colour`] — the check that fixes [`PHASE_DEGREES`].
    ///
    /// Each hue is classified only coarsely (which channel dominates, and by
    /// how much), because a name like "turquoise" is not a triple; what the
    /// case pins is the ORDER — warm at hue 1, through blue in the middle, to
    /// green, and back to warm at hue 15 — which is what a wrong burst offset
    /// rotates.
    #[test]
    fn hue_names_read_back_in_the_manuals_order() {
        let warm = |c: u8| {
            let (r, _, b) = atari_colour(c);
            r > b + 20
        };
        let cool = |c: u8| {
            let (r, _, b) = atari_colour(c);
            b > r + 20
        };
        let greenish = |c: u8| {
            let (r, g, b) = atari_colour(c);
            g > r + 10 && g > b + 10
        };
        // Hue 0 is chroma off, whatever the luminance.
        for lum in 0..16u8 {
            let (r, g, b) = atari_colour(lum);
            if lum == 14 {
                continue; // §8.3 substitutes this one.
            }
            assert_eq!((r, g), (r, b), "hue 0 luminance {lum} is grey: {r} {g} {b}");
        }
        // 1 gold, 2 orange, 3 red-orange: warm.
        for hue in 1..=3u8 {
            assert!(warm(hue * 16 + 8), "hue {hue} is warm");
        }
        // 7, 8, 9: blue, blue, light blue.
        for hue in 7..=9u8 {
            assert!(cool(hue * 16 + 8), "hue {hue} is blue");
        }
        // 11, 12, 13: green-blue, green, yellow-green.
        for hue in 11..=13u8 {
            assert!(greenish(hue * 16 + 8), "hue {hue} is green");
        }
        // 15 light orange: back round to warm.
        assert!(warm(15 * 16 + 8), "hue 15 is warm again");
        // Luminance still runs the brightness: a bright hue outshines a dark
        // one of the same hue on every channel.
        let (dr, dg, db) = atari_colour(0x32);
        let (br, bg, bb) = atari_colour(0x3C);
        assert!(br > dr && bg > dg && bb > db, "luminance 12 outshines luminance 2");
    }

    // The two schemes read the far edges differently, which is the half of
    // §8.3's variant §8.3 does not state (SQ-1484). Same header bytes, one
    // column and one pair apart.
    #[test]
    fn the_two_schemes_resolve_the_same_edges_differently() {
        let std = StripLayout::resolve(3, 0, 8, 20, FamilyCScheme::Standard).expect("region");
        assert_eq!((std.left, std.top, std.cols, std.pairs), (0, 0, 6, 11));
        let nl = StripLayout::resolve(3, 0, 8, 20, FamilyCScheme::NoLiteral).expect("region");
        assert_eq!((nl.left, nl.top, nl.cols, nl.pairs), (0, 0, 5, 10));
        assert_eq!(std.pair_count(), 66);
        assert_eq!(nl.pair_count(), 50);
        // A one-column, one-pair region is real under the inclusive reading
        // and empty under the exclusive one.
        assert!(StripLayout::resolve(3, 0, 3, 0, FamilyCScheme::Standard).is_ok());
        assert!(StripLayout::resolve(3, 0, 3, 0, FamilyCScheme::NoLiteral).is_err());
    }

    // The no-literal variant, on a record built by hand with the pixels
    // worked out from §8.3's rules (SQ-1484). Header edges 3, 0, 5, 8: under
    // the exclusive reading that is (5-3) = 2 columns of (8-0)/2 = 4 pairs,
    // so eight pairs exactly, at x 0..16 and rows 0..8.
    //
    // Every control byte is a full repeat count and bit 7 is NOT masked, so
    // 0x83 repeats its pair 131 times and not 4 — which is the whole
    // difference, and what the same bytes read as the standard scheme get
    // wrong.
    #[test]
    fn the_no_literal_variant_reads_every_control_byte_as_a_full_count() {
        let layout = StripLayout::resolve(3, 0, 5, 8, FamilyCScheme::NoLiteral).expect("region");
        assert_eq!(layout.pair_count(), 8, "two columns of four pairs");
        // Column one: four copies of (0b11_00_00_00, 0b00_00_00_11) — value 3
        // in the leftmost stored pixel of every even row and the rightmost of
        // every odd row. Column two: four copies of (0x00, 0xFF).
        let data = [4u8, 0b11_00_00_00, 0b00_00_00_11, 4, 0x00, 0xFF];
        let painted = paint_strips(&data, &layout, FamilyCScheme::NoLiteral);
        assert_eq!(painted.emitted, 8, "the region filled exactly");
        assert_eq!(painted.consumed, 6, "and took both units to do it");
        let px = |x: usize, y: usize| painted.pixels[y * CANVAS_WIDTH + x];
        for y in [0usize, 2, 4, 6] {
            assert_eq!(px(0, y), 3, "column one, row {y}, first stored pixel");
            assert_eq!(px(1, y), 3, "…is two device pixels wide");
            assert_eq!(px(2, y), 0, "…and the next stored pixel is value 0");
            assert_eq!(px(6, y + 1), 3, "column one, row {}, last stored pixel", y + 1);
            assert_eq!(px(7, y + 1), 3);
            assert_eq!(px(0, y + 1), 0);
        }
        for y in 0..8 {
            let want = if y % 2 == 0 { 0 } else { 3 };
            for x in 8..16 {
                assert_eq!(px(x, y), want, "column two, row {y}, x {x}");
            }
        }
        assert_eq!(px(16, 0), 0, "nothing right of the second column");
        assert_eq!(px(0, 8), 0, "nothing below the region");

        // The SAME bytes under the standard scheme: 4 is a literal count of
        // five pairs, so the first unit swallows ten bytes it does not have
        // and the record decodes to nothing at all.
        let std = StripLayout::resolve(3, 0, 5, 8, FamilyCScheme::Standard).expect("region");
        let wrong = paint_strips(&data, &std, FamilyCScheme::Standard);
        assert_eq!(wrong.emitted, 0, "read as literals the run outruns the data");
    }

    // A stored count of zero still paints one pair: §8.3 says the count is
    // "the byte's full value", and a value of zero that painted nothing would
    // leave the region one pair short of the data for the rest of the record.
    #[test]
    fn a_no_literal_count_of_zero_paints_one_pair() {
        let layout = StripLayout::resolve(3, 0, 4, 4, FamilyCScheme::NoLiteral).expect("region");
        assert_eq!(layout.pair_count(), 2, "one column of two pairs");
        let painted = paint_strips(&[0u8, 0xFF, 0xFF, 0, 0x00, 0x00], &layout, FamilyCScheme::NoLiteral);
        assert_eq!(painted.emitted, 2);
        let px = |x: usize, y: usize| painted.pixels[y * CANVAS_WIDTH + x];
        assert_eq!(px(0, 0), 3, "the zero-count unit painted its pair");
        assert_eq!(px(0, 1), 3);
        assert_eq!(px(0, 2), 0, "and the second unit painted the next");
    }
}
