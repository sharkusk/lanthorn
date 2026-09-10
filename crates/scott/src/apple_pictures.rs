//! Picture family **D** — the Apple II artwork of the US S.A.G.A. releases
//! (SQ-1476, SQ-1489), as shipped on the seven two-sided *Scott Adams Graphic
//! Adventure* disks of §10.6.
//!
//! # This is a LINE DRAWING, and the specification says it is a bitmap
//!
//! `docs/internals/scott-dialects-spec.md` §8.4 describes family D as a
//! simulated 8192-byte high-resolution page, filled by a three-way address
//! interleave from byte pairs and resolved to colour by an artifact model.
//! **Not one byte of the four plain releases' artwork is stored that way.**
//! Every picture file on *Adventureland*'s, *Pirate Adventure*'s, *Mission
//! Impossible*'s and *Strange Odyssey*'s companion sides is an **opcode
//! stream** — move, line, brush, area, and three attribute registers — played
//! over the machine's own 280 x 192 hi-res page. §8.4's description does fit
//! the three *scrambled* releases, whose records really do open with its
//! four-byte header (`00 00 28 A0`), and which
//! [`decode_family_d_scrambled`] reads; so the section is right about one of
//! its two sub-variants and wrong about the other. Appendix A items 26, 38 and
//! 39 record the measurements.
//!
//! §8.4 *is* right about one thing this module needs: the **artifact colour
//! model** it states for resolving a hi-res page to six colours, which
//! [`decode_family_d`] applies unchanged.
//!
//! # The format, as measured
//!
//! **Canvas** [`CANVAS_WIDTH`] x [`CANVAS_HEIGHT`] — the Apple II hi-res
//! screen, 280 x 192. Internally the decoder keeps the real thing: 40 bytes a
//! row, **seven pixels in the low seven bits of a byte, least significant bit
//! leftmost, and bit 7 a per-byte palette select** (§8.4).
//!
//! **Container.** One picture is one DOS 3.3 binary file on the release's
//! companion side, so the bytes begin with that file type's four-byte
//! prologue: a little-endian load address (`$7000` on every specimen) and a
//! little-endian data length. The opcode stream is that many bytes from
//! offset 4. Which file is which picture is
//! [`crate::saga_us::room_picture_file_name`] and
//! [`crate::saga_us::parse_apple_picture_file_name`]; walking the disk is the
//! host's business, exactly as it is for family C.
//!
//! **The ground is WHITE.** A room picture is drawn over a page filled with
//! `$FF`, not over a blank one — which is why so many of these pictures open
//! by flooding the canvas with a dark paint and why reading the ground as
//! black inverts them.
//!
//! **Tokens.** A byte with **bit 7 set** opens a three-byte token: bits 7-5
//! are the command, **bit 0 is bit 8 of the horizontal coordinate** (280 does
//! not fit in a byte), and the two bytes after it are the low eight bits of
//! *x* and then *y*.
//!
//! | command | effect |
//! |---|---|
//! | `0x80` | **move** — the current point becomes (*x*, *y*); no ink |
//! | `0xA0` | **line** — a line from the current point to (*x*, *y*) in the current [`HCOLOR`](HCOLOR_MASKS), which becomes the current point |
//! | `0xC0` | **brush** — stamp the current 14 x 16 [brush](BRUSHES) with its top-left at (*x*, *y*), in the current paint. Does **not** move the line pen |
//! | `0xE0` | **area** — flood the lit region containing (*x*, *y*) with the current paint. Does **not** move the line pen |
//!
//! A byte with **bit 7 clear** is an attribute token whose top three bits pair
//! with the drawing command it feeds, and whose low four bits are its operand:
//!
//! | token | effect |
//! |---|---|
//! | `0x00`-`0x1F` | **end of picture** — nothing after it is read |
//! | `0x20`\|*c* | the line colour: Applesoft **HCOLOR** *c*, [`HCOLOR_MASKS`] |
//! | `0x40`\|*n* | the brush: one of the eight [`BRUSHES`] |
//! | `0x60`, then one more byte *v* | the paint: [`PAINTS`]\[*v*\], a pair of [`PATTERNS`] indices |
//!
//! `0x60` is the only **two-byte** token, and its operand is a raw value with
//! no bit-7 rule of its own.
//!
//! **The paint is a pair, not a colour.** `PAINTS[v]` names one pattern for
//! **even** rows and another for **odd**, and each pattern is four bytes
//! chosen by the screen byte's column modulo four — so a paint can be a solid
//! colour, a two-colour row dither, or a diagonal hatch. Both `0xC0` and
//! `0xE0` paint through it; `0xA0` does not, and takes the Applesoft colour
//! mask instead.
//!
//! **Attribute defaults, at the start of every picture:** HCOLOR 4 (black in
//! the high-bit palette), brush 5, paint 0, and the pen at (140, 96).
//!
//! # How the reading was settled
//!
//! Not from the specification, which describes another format, but from the
//! specimens — the picture streams, and then the releases' own 6502 renderer,
//! which is on the same disks and is as much a specimen as the artwork is.
//!
//! - **The ninth bit of *x*.** Parsed as `(command, x, y)` triples with an
//!   eight-bit *x*, the four titles' 314 picture files yield a scatter of
//!   every byte value in the command position. Read with bit 0 as *x*'s ninth
//!   bit, the command position holds **only** `0x80`, `0xA0`, `0xC0` and
//!   `0xE0` across all **71,899** three-byte tokens, and **not one of their
//!   coordinates lands off the 280 x 192 canvas**.
//! - **The attribute tokens are four commands, not ninety-two values.**
//!   Reading `0x60` as a two-byte token leaves only **19** distinct
//!   bit-7-clear opcodes in the whole corpus — `0x00` exactly 314 times (once
//!   per file, always the last token, always at the declared end of the
//!   stream), `0x20`-`0x27`, `0x40`-`0x47`, `0x53` twice, and `0x60` itself
//!   1,977 times — where reading every such byte as its own token leaves 92.
//!   And **every** one of those 1,977 is followed by another bit-7-clear
//!   byte, which is what a mandatory operand looks like.
//! - **The operand ranges match the tables exactly.** The `0x60` operand runs
//!   `0x00`-`0x6B` across the corpus and [`PAINTS`] has exactly `0x6C` = 108
//!   entries, ending where [`PATTERNS`] begins. That is the strongest single
//!   check that the operand is an index into that table and nothing else.
//! - **The releases' own renderer.** `M3` on each plain release's boot side is
//!   byte-identical across all four titles and holds both tables and the token
//!   loop; the loop dispatches on the top three bits, masks the operand with
//!   `AND #$0F`, returns on the `0x00` class, hands `0x20`'s operand to
//!   Applesoft's HCOLOR entry point at `$F6EC`, plays `0x80` through
//!   Applesoft's HPOSN (`$F411`) and `0xA0` through its HPLOT-TO (`$F53A`),
//!   and clears the page to `$FF` before a room picture. `FPBASIC` on the same
//!   disk carries the Applesoft image whose `$F6F6` colour table is
//!   [`HCOLOR_MASKS`]. Reading a game's own binary is measurement of a
//!   specimen; no interpreter was consulted (`docs/internals/clean-room.md`).
//! - **What the pictures are.** `R0100` decodes to white lettering reading
//!   `IT'S TOO DARK!` on black — §8.6's reserved index 0 — `R0198` to an
//!   `INVENTORY` card (§8.6's reserved 98), `R0199` to the Adventure
//!   International globe-and-`ai` logo in green, blue and orange (§8.6's
//!   reserved 99), and `B01255` to the word `Adventureland` in green. A wrong
//!   ground, a wrong palette or a wrong token length does not produce those.
//!
//! # What is still undetermined
//!
//! **The flood fill's exact edge rule.** The release's own filler is a
//! scanline walk whose stop test looks at the pixel to the *left* of the one
//! being tested as well as the pixel itself; this decoder spreads over lit
//! pixels and stops at unlit ones, which is that rule's plain meaning and is
//! what makes the corpus legible. A region reached only through a
//! single-pixel gap may therefore differ from the machine by a few pixels.
//! Appendix A item 38 records it.

use crate::saga_pictures::{Painted, PaintedBox, Rgb};
use crate::saga_us::SagaPlatform;

/// The family-D canvas width in pixels — the Apple II hi-res screen.
pub const CANVAS_WIDTH: usize = 280;

/// The family-D canvas height in pixels — the Apple II hi-res screen.
///
/// The whole 192-row page, not the 160 rows a mixed-mode screen shows above
/// four text lines: measured across the four plain releases' 314 pictures, 29
/// of them ink at or below row 160 and the deepest reaches row 170.
pub const CANVAS_HEIGHT: usize = 192;

/// Bytes per hi-res row: seven pixels each, 40 x 7 = [`CANVAS_WIDTH`].
const COLUMNS: usize = CANVAS_WIDTH / 7;

/// The six colours an Apple II hi-res screen presents, in the order
/// [`HiResPicture::pixels`] indexes them: black, purple, green, blue, orange,
/// white.
///
/// §8.4's own values. The first four are the high-bit-clear set (black,
/// purple, green, white) and the set used when a byte's bit 7 is set swaps
/// purple and green for blue and orange.
pub const PALETTE: [Rgb; 6] = [
    (0x00, 0x00, 0x00), // 0 black
    (0xD5, 0x3E, 0xF9), // 1 purple
    (0x64, 0xD4, 0x40), // 2 green
    (0x45, 0x8F, 0xF7), // 3 blue
    (0xD7, 0x76, 0x2C), // 4 orange
    (0xFF, 0xFF, 0xFF), // 5 white
];

/// Applesoft's eight HCOLOR masks, the table at `$F6F6` of the Applesoft image
/// the releases ship as `FPBASIC` — black, green, violet, white, and the same
/// four again in the high-bit palette (black, orange, blue, white).
///
/// A `0x20`-class token's operand selects one of these, and `0xA0` plots
/// through it exactly as Applesoft's HPLOT does: the pixel's bit and the
/// byte's bit 7 both take their value from the mask. This is the published
/// Apple II high-resolution colour model — see the *Applesoft BASIC
/// Programmer's Reference Manual*'s HCOLOR table and the *Apple II Reference
/// Manual*'s description of the high-resolution screen — and the disk's own
/// copy of the ROM is where these eight bytes were read.
pub const HCOLOR_MASKS: [u8; 8] = [0x00, 0x2A, 0x55, 0x7F, 0x80, 0xAA, 0xD5, 0xFF];

/// [`HCOLOR_MASKS`] as Applesoft rotates them on an **odd** byte column.
///
/// Seven pixels to a byte is an odd number, so a mask that lights every second
/// pixel would change hue at every byte boundary; Applesoft's position routine
/// therefore exclusive-ORs the mask with `0x7F` on odd columns unless it is
/// one of the four solid values (black, white, and their high-bit twins),
/// which is what keeps a green line green across the screen. Measured off the
/// same `FPBASIC` image; pinned as a table here because the arithmetic that
/// produces it is a `CMP`/`BPL` on a doubled accumulator and the eight answers
/// are clearer than the derivation.
pub const HCOLOR_MASKS_ODD_COLUMN: [u8; 8] = [0x00, 0x55, 0x2A, 0x7F, 0x80, 0xD5, 0xAA, 0xFF];

/// The thirty fill patterns, each four bytes chosen by the screen byte's
/// column modulo four.
///
/// Entries 0-7 are the eight solid Apple II colours with their parity
/// maintained across byte columns (so entry 1 lights the even pixels of the
/// screen and entry 2 the odd ones); 8 onward are hatches and dithers. A
/// [`PAINTS`] entry names two of these, one for even rows and one for odd.
///
/// **Re-derivable from a specimen**: the 120 bytes at `$9054` of the `M3` file
/// on any plain release's boot side, which is byte-identical on all four. The
/// table's extent is not a guess — `$9054` is the pointer the paint routine
/// loads, and the byte after entry 29 is the first instruction of the next
/// routine.
pub const PATTERNS: [[u8; 4]; 30] = [
    [0x00, 0x00, 0x00, 0x00], //  0
    [0x55, 0x2A, 0x55, 0x2A], //  1
    [0x2A, 0x55, 0x2A, 0x55], //  2
    [0x7F, 0x7F, 0x7F, 0x7F], //  3
    [0x80, 0x80, 0x80, 0x80], //  4
    [0xD5, 0xAA, 0xD5, 0xAA], //  5
    [0xAA, 0xD5, 0xAA, 0xD5], //  6
    [0xFF, 0xFF, 0xFF, 0xFF], //  7
    [0x33, 0x66, 0x4C, 0x19], //  8
    [0xB3, 0xE6, 0xCC, 0x99], //  9
    [0x4C, 0x19, 0x33, 0x66], // 10
    [0xCC, 0x99, 0xB3, 0xE6], // 11
    [0x11, 0x22, 0x44, 0x08], // 12
    [0x91, 0xA2, 0xC4, 0x88], // 13
    [0x44, 0x08, 0x11, 0x22], // 14
    [0xC4, 0x88, 0x91, 0xA2], // 15
    [0x22, 0x44, 0x08, 0x11], // 16
    [0xA2, 0xC4, 0x88, 0x91], // 17
    [0x08, 0x11, 0x22, 0x44], // 18
    [0x88, 0x91, 0xA2, 0xC4], // 19
    [0xC9, 0xA4, 0x92, 0x89], // 20
    [0x24, 0x12, 0x49, 0x24], // 21
    [0x77, 0x6E, 0x5D, 0x3B], // 22
    [0xF7, 0xEE, 0xDD, 0xBB], // 23
    [0x5D, 0x3B, 0x77, 0x6E], // 24
    [0xDD, 0xBB, 0xF7, 0xEE], // 25
    [0x6E, 0x5D, 0x3B, 0x77], // 26
    [0xEE, 0xDD, 0xBB, 0xF7], // 27
    [0x3B, 0x77, 0x6E, 0x5D], // 28
    [0xBB, 0xF7, 0xEE, 0xDD], // 29
];

/// The 108 paints a `0x60` token can select: (pattern for **even** rows,
/// pattern for **odd** rows), both indices into [`PATTERNS`].
///
/// Paint `0x50` is (0, 0), solid black in the low palette; `0x35` is (4, 4),
/// solid black in the high one; `0x4D`, `0x65`, `0x57`, `0x46`, `0x3C` and
/// `0x34` are the solid white, purple, green, blue, orange and high-palette
/// white; the rest are two-row dithers and hatches.
///
/// **Re-derivable from a specimen**: the 216 bytes at `$8F7C` of `M3`, read as
/// pairs. The extent is checked twice over — the table ends exactly where
/// [`PATTERNS`] begins, and the largest operand anywhere in the four titles'
/// 1,977 paint tokens is `0x6B`, the last entry.
pub const PAINTS: [(u8, u8); 108] = [
    (3, 7),
    (22, 7),
    (26, 29),
    (28, 23),
    (8, 11),
    (0, 27), // 0x00
    (0, 4),
    (3, 27),
    (3, 6),
    (26, 6),
    (0, 6),
    (0, 17), // 0x06
    (2, 6),
    (28, 19),
    (16, 19),
    (16, 7),
    (2, 27),
    (2, 7), // 0x0C
    (2, 23),
    (2, 9),
    (26, 4),
    (16, 4),
    (2, 5),
    (18, 23), // 0x12
    (26, 7),
    (3, 23),
    (22, 25),
    (3, 5),
    (3, 13),
    (26, 13), // 0x18
    (26, 5),
    (16, 5),
    (0, 13),
    (0, 23),
    (8, 5),
    (22, 5), // 0x1E
    (1, 5),
    (22, 11),
    (1, 7),
    (1, 23),
    (1, 9),
    (1, 4), // 0x24
    (22, 4),
    (12, 15),
    (1, 27),
    (1, 17),
    (12, 23),
    (12, 4), // 0x2A
    (22, 19),
    (1, 6),
    (22, 6),
    (12, 17),
    (7, 7),
    (4, 4), // 0x30
    (7, 27),
    (27, 29),
    (7, 17),
    (6, 7),
    (23, 6),
    (6, 27), // 0x36
    (6, 6),
    (4, 6),
    (17, 19),
    (4, 17),
    (23, 7),
    (23, 11), // 0x3C
    (23, 25),
    (5, 7),
    (23, 5),
    (7, 13),
    (5, 5),
    (5, 13), // 0x42
    (13, 15),
    (4, 13),
    (23, 4),
    (5, 27),
    (5, 6),
    (3, 3), // 0x48
    (22, 3),
    (3, 12),
    (0, 0),
    (8, 26),
    (2, 22),
    (26, 28), // 0x4E
    (3, 16),
    (2, 3),
    (2, 26),
    (2, 2),
    (18, 28),
    (0, 26), // 0x54
    (18, 26),
    (16, 18),
    (0, 16),
    (3, 26),
    (22, 26),
    (22, 18), // 0x5A
    (1, 2),
    (22, 24),
    (1, 3),
    (1, 26),
    (1, 22),
    (1, 1), // 0x60
    (1, 0),
    (22, 0),
    (22, 12),
    (22, 14),
    (12, 14),
    (0, 12), // 0x66
];

/// The eight brushes a `0x40` token can select, each a **14 x 16** stencil.
///
/// Byte *b* of an entry covers row `(b & 7) + if b >= 16 { 8 } else { 0 }` and
/// the seven pixels starting at `7 * ((b >> 3) & 1)` of the brush box, least
/// significant bit leftmost — the four eight-byte blits the release's own
/// routine performs, in its own order. Brushes 0-5 are discs of growing
/// radius, 6 and 7 are speckled spatter.
///
/// **Re-derivable from a specimen**: the 256 bytes at `$9500` of `M3`.
pub const BRUSHES: [[u8; 32]; 8] = [
    [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ],
    [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ],
    [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x60, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
        0x03, 0x60, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x01, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ],
    [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x60, 0x70, 0x70, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x07,
        0x07, 0x70, 0x70, 0x60, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x07, 0x03, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ],
    [
        0x00, 0x00, 0x00, 0x60, 0x78, 0x78, 0x7C, 0x7C, 0x00, 0x00, 0x00, 0x03, 0x0F, 0x0F, 0x1F,
        0x1F, 0x7C, 0x7C, 0x78, 0x78, 0x60, 0x00, 0x00, 0x00, 0x1F, 0x1F, 0x0F, 0x0F, 0x03, 0x00,
        0x00, 0x00,
    ],
    [
        0x00, 0x60, 0x7C, 0x7E, 0x7E, 0x7E, 0x7F, 0x7F, 0x00, 0x03, 0x1F, 0x3F, 0x3F, 0x3F, 0x7F,
        0x7F, 0x7F, 0x7F, 0x7E, 0x7E, 0x7E, 0x7C, 0x60, 0x00, 0x7F, 0x7F, 0x3F, 0x3F, 0x3F, 0x1F,
        0x03, 0x00,
    ],
    [
        0x00, 0x00, 0x00, 0x00, 0x40, 0x08, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00,
        0x09, 0x28, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x01, 0x00, 0x00,
        0x00, 0x00,
    ],
    [
        0x00, 0x20, 0x08, 0x24, 0x40, 0x12, 0x68, 0x60, 0x00, 0x04, 0x01, 0x14, 0x00, 0x2B, 0x03,
        0x27, 0x62, 0x48, 0x22, 0x28, 0x00, 0x10, 0x40, 0x00, 0x17, 0x09, 0x22, 0x15, 0x00, 0x0A,
        0x00, 0x00,
    ],
];

/// One decoded family-D picture: [`CANVAS_WIDTH`] x [`CANVAS_HEIGHT`] pixels,
/// each an index into [`PALETTE`].
///
/// Not [`crate::saga_pictures::Picture`], which carries four palette entries
/// and four stored colour bytes: family D stores no colour bytes at all and
/// presents six colours, so it answers its own shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiResPicture {
    /// Always [`CANVAS_WIDTH`].
    pub width: usize,
    /// Always [`CANVAS_HEIGHT`].
    pub height: usize,
    /// `width * height` pixel values, each 0-5, row-major from the top-left.
    /// Index [`PALETTE`] with one to get a colour.
    pub pixels: Vec<u8>,
    /// The canvas rectangle this record's own drawing covers — the same fact
    /// [`crate::saga_pictures::Picture::painted`] carries, and needed for the
    /// same reason: an object picture is a sub-image that must be composited
    /// over the rectangle it drew and nowhere else (§8.6, SQ-1482).
    ///
    /// **Every pixel a line, a brush or a fill wrote**, not every pixel that
    /// is not the ground: family D's ground is WHITE, and a record that
    /// flooded its canvas white is indistinguishable from one that drew
    /// nothing there. Bounds are inclusive; `None` for a record that wrote
    /// nothing at all.
    pub painted: Option<Painted>,
}

impl HiResPicture {
    /// The RGB of the pixel at `(x, y)`, or `None` off the canvas.
    #[must_use]
    pub fn rgb(&self, x: usize, y: usize) -> Option<Rgb> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let v = *self.pixels.get(y * self.width + x)?;
        PALETTE.get(usize::from(v)).copied()
    }
}

/// Why a file is not a family-D picture ("name it and refuse it", §11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppleError {
    /// Fewer bytes than the DOS 3.3 binary prologue, so there is no length
    /// field to read, let alone a picture.
    TooShort {
        /// The length that was offered.
        len: usize,
    },
    /// The prologue declares a stream of no bytes, or the file holds none
    /// after it.
    EmptyStream,
    /// The platform does not use family D. The Commodore 64 and Atari 8-bit
    /// releases are family C (§8.3), a four-colour strip bitmap that shares
    /// nothing with this.
    NotAppleII {
        /// The platform asked for.
        platform: SagaPlatform,
    },
}

impl std::fmt::Display for AppleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppleError::TooShort { len } => {
                write!(f, "family-D file is {len} bytes, too short for a four-byte prologue")
            }
            AppleError::EmptyStream => write!(f, "family-D file declares an empty opcode stream"),
            AppleError::NotAppleII { platform } => {
                write!(f, "{} releases do not use picture family D", platform.label())
            }
        }
    }
}

impl std::error::Error for AppleError {}

/// A simulated Apple II high-resolution page: 40 bytes a row, 192 rows.
///
/// Kept as the machine keeps it — seven pixels in the low seven bits of a
/// byte and a palette select in bit 7 — because every one of this format's
/// primitives writes whole bytes through a mask and the colour a pixel ends up
/// with depends on its neighbours' bits, not on a per-pixel value.
struct Page {
    bytes: Vec<u8>,
    /// Which pixels the record's own primitives wrote — see
    /// [`HiResPicture::painted`].
    painted: PaintedBox,
}

impl Page {
    /// A page cleared to white, which is what the release's own loader does
    /// (`LDA #$FF`, then Applesoft's page-fill entry) before a room picture.
    fn white() -> Self {
        Page { bytes: vec![0xFF; COLUMNS * CANVAS_HEIGHT], painted: PaintedBox::default() }
    }

    /// Write `colour`'s bit for this pixel, and `colour`'s bit 7 as the byte's
    /// palette select — Applesoft's plot, and this format's paint, both.
    fn poke(&mut self, x: i32, y: i32, colour: u8) {
        let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else { return };
        if x >= CANVAS_WIDTH || y >= CANVAS_HEIGHT {
            return;
        }
        let mask = (1u8 << (x % 7)) | 0x80;
        let at = y * COLUMNS + x / 7;
        self.bytes[at] = (self.bytes[at] & !mask) | (colour & mask);
        self.painted.mark(x, y);
    }

    /// Is the pixel lit? Off the canvas counts as unlit, which stops a flood
    /// fill at the edge.
    fn lit(&self, x: i32, y: i32) -> bool {
        let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else { return false };
        if x >= CANVAS_WIDTH || y >= CANVAS_HEIGHT {
            return false;
        }
        self.bytes[y * COLUMNS + x / 7] & (1 << (x % 7)) != 0
    }

    /// Plot one pixel of an `0xA0` line in Applesoft HCOLOR `hcolor`.
    fn plot_line(&mut self, x: i32, y: i32, hcolor: usize) {
        let col = if x >= 0 { (x as usize) / 7 } else { return };
        let mask = if col % 2 == 0 { HCOLOR_MASKS[hcolor] } else { HCOLOR_MASKS_ODD_COLUMN[hcolor] };
        self.poke(x, y, mask);
    }

    /// Paint one pixel through a paint pair: the even-row pattern on even
    /// rows, the odd-row one on odd, each indexed by the byte column modulo
    /// four.
    fn paint(&mut self, x: i32, y: i32, paint: (u8, u8)) {
        let col = if x >= 0 { (x as usize) / 7 } else { return };
        let Ok(row) = usize::try_from(y) else { return };
        let which = if row % 2 == 0 { paint.0 } else { paint.1 };
        let Some(pattern) = PATTERNS.get(usize::from(which)) else { return };
        self.poke(x, y, pattern[col % 4]);
    }

    /// A Bresenham line with both endpoints plotted, as Applesoft's HPLOT-TO
    /// draws one.
    fn line(&mut self, from: (i32, i32), to: (i32, i32), hcolor: usize) {
        let (mut x, mut y) = from;
        let dx = (to.0 - x).abs();
        let dy = -(to.1 - y).abs();
        let sx = if x < to.0 { 1 } else { -1 };
        let sy = if y < to.1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.plot_line(x, y, hcolor);
            if x == to.0 && y == to.1 {
                return;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Stamp brush `n` with its top-left at `(x, y)`, painted through `paint`.
    fn brush(&mut self, x: i32, y: i32, n: usize, paint: (u8, u8)) {
        let stencil = &BRUSHES[n];
        for row in 0..16i32 {
            for half in 0..2i32 {
                let at = if row < 8 { 0 } else { 16 } + 8 * half + (row % 8);
                let byte = stencil[at as usize];
                for bit in 0..7i32 {
                    if byte & (1 << bit) != 0 {
                        self.paint(x + 7 * half + bit, y + row, paint);
                    }
                }
            }
        }
    }

    /// Flood the lit region containing `(x, y)` with `paint`.
    ///
    /// Lit, not unlit: the ground is white and the artwork's outlines are
    /// drawn dark, so a region is a run of *set* pixels. A separate visited
    /// map is needed because a paint may leave the pixels it covers unlit —
    /// solid black is a paint like any other.
    fn fill(&mut self, x: i32, y: i32, paint: (u8, u8)) {
        if !self.lit(x, y) {
            return;
        }
        let mut seen = vec![false; CANVAS_WIDTH * CANVAS_HEIGHT];
        let mut queue = std::collections::VecDeque::new();
        let index = |x: i32, y: i32| (y as usize) * CANVAS_WIDTH + (x as usize);
        seen[index(x, y)] = true;
        queue.push_back((x, y));
        while let Some((x, y)) = queue.pop_front() {
            self.paint(x, y, paint);
            for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                if self.lit(nx, ny) && !seen[index(nx, ny)] {
                    seen[index(nx, ny)] = true;
                    queue.push_back((nx, ny));
                }
            }
        }
    }

    /// Resolve the page to one [`PALETTE`] index per pixel by §8.4's artifact
    /// model.
    fn resolve(&self) -> Vec<u8> {
        // §8.4: the selector for a three-bit neighbourhood and a parity, as a
        // position in the high-bit-clear set (black, purple, green, white).
        const SELECTOR: [[u8; 2]; 8] = {
            let mut table = [[0u8; 2]; 8];
            let mut i = 0;
            while i < 8 {
                let mut j = 0;
                while j < 2 {
                    table[i][j] = if i & 2 != 0 {
                        if i & 1 != 0 || i & 4 != 0 {
                            3
                        } else if j == 1 {
                            2
                        } else {
                            1
                        }
                    } else if i & 1 != 0 && i & 4 != 0 {
                        if j == 1 {
                            1
                        } else {
                            2
                        }
                    } else {
                        0
                    };
                    j += 1;
                }
                i += 1;
            }
            table
        };
        // The selector's four positions as PALETTE indices, high bit clear
        // then set.
        const LOW: [u8; 4] = [0, 1, 2, 5];
        const HIGH: [u8; 4] = [0, 3, 4, 5];

        let mut out = vec![0u8; CANVAS_WIDTH * CANVAS_HEIGHT];
        for y in 0..CANVAS_HEIGHT {
            let row = &self.bytes[y * COLUMNS..(y + 1) * COLUMNS];
            for col in 0..COLUMNS {
                let prev = if col > 0 { u32::from(row[col - 1] & 0x7F) } else { 0 };
                let cur = row[col];
                let next = if col + 1 < COLUMNS { u32::from(row[col + 1] & 0x7F) } else { 0 };
                let window = prev | (u32::from(cur & 0x7F) << 7) | (next << 14);
                let set = if cur & 0x80 != 0 { HIGH } else { LOW };
                for bit in 0..7 {
                    let i = ((window >> (bit + 6)) & 7) as usize;
                    let parity = (bit ^ col) & 1;
                    out[y * CANVAS_WIDTH + 7 * col + bit] = set[SELECTOR[i][parity] as usize];
                }
            }
        }
        out
    }
}

/// Every **scrambled** family-D picture on one side-A image, as byte ranges
/// into it, in the order the disk holds them (SQ-1490).
///
/// `image` is the flat sector image `blorb::medium::apple_raw_sectors` hands
/// back — a side A with no filesystem on it, so there is no catalogue to walk
/// and the records are found by their own header.
///
/// # How they are found, and why a scan rather than a table
///
/// §8.4 says a family-D loader needs "a hard-coded per-title list of (usage,
/// index, offset, length)". It does not, on these three disks: every record
/// opens with §8.4's own four-byte header, the three titles' headers are
/// **identical** (`00 00 28 A0` — no offset, 40 byte columns, 160 rows), every
/// record starts on a **sector boundary**, and they run in order from **track
/// 1 sector 0**. So the *n*-th header is picture *n*, and the record is the
/// bytes from it up to the next one. Measured: 36 records on *Voodoo Castle*,
/// 26 on *The Count*, 35 on *Claymorgue Castle*, at `0x01000` onward on all
/// three. They are nearly always packed tight — five to twenty-two sectors to
/// the next header — but not always: *Claymorgue Castle* leaves forty-one
/// after its title card, which is why a record's length is BOUNDED rather than
/// simply taken from the gap.
///
/// **And the ordinal is the picture index**, which four checks settle. Record
/// 0 is §8.6's reserved darkness card on all three (the words `IT'S TOO
/// DARK!`); records 1 upward are the rooms in room order; the record numbered
/// with each release's LAST room is that release's death card — *Voodoo
/// Castle*'s room 25 "lot of TROUBLE!", *The Count*'s room 22 "LOT OF
/// TROUBLE!", *Claymorgue Castle*'s room 32 "real mess!" — which is the check
/// that no spurious header anywhere before it has shifted the numbering; and
/// *Claymorgue Castle*'s room 29, "dragon's lair", is a green dragon.
///
/// The records past the highest room number are the release's object and title
/// artwork — ten on *Voodoo Castle*, three on *The Count*, two on
/// *Claymorgue Castle*, including in each case the Adventure International
/// title card. **What §8.6 indices those carry is not established here**;
/// this function numbers every record by its ordinal, which is what the disk's
/// own order says and is right for every index a room can ask for.
///
/// The last record is bounded at [`SCRAMBLED_MAX_RECORD`] bytes rather than
/// run to the end of the image, which on these disks is fifty kilobytes of
/// nothing.
#[must_use]
pub fn scan_scrambled_pictures(image: &[u8]) -> Vec<std::ops::Range<usize>> {
    /// §8.4's header as all three titles write it: no offset, 40 byte columns
    /// (280 pixels), 160 rows.
    const HEADER: [u8; 4] = [0x00, 0x00, 0x28, 0xA0];
    /// A sector, and the granularity a record starts on.
    const SECTOR: usize = 256;
    /// Track 1 sector 0. Track 0 is the boot track on all three.
    const FIRST: usize = 16 * SECTOR;

    let mut starts = Vec::new();
    let mut at = FIRST;
    while at + HEADER.len() <= image.len() {
        if image[at..at + HEADER.len()] == HEADER {
            starts.push(at);
        }
        at += SECTOR;
    }
    starts
        .iter()
        .enumerate()
        .map(|(i, &start)| {
            let end = starts.get(i + 1).copied().unwrap_or(usize::MAX);
            start..end.min(start.saturating_add(SCRAMBLED_MAX_RECORD)).min(image.len())
        })
        .collect()
}

/// The most bytes a scrambled record can need: one byte-pair token per output
/// pair over the whole 40 x 160 picture, plus its four-byte header.
///
/// The run-length scheme cannot expand — a literal pair costs two bytes and
/// writes two — so this bounds the last record, whose end no following header
/// marks.
pub const SCRAMBLED_MAX_RECORD: usize = 4 + 40 * 160;

/// Decode one **scrambled** family-D picture — §8.4's own sub-variant, the one
/// the specification is right about (SQ-1490).
///
/// `record` is a range [`scan_scrambled_pictures`] found: §8.4's four-byte
/// header — horizontal byte-column offset, vertical row offset, width in byte
/// columns, height in rows, the last two **absolute limits** — followed by the
/// compressed stream. There is no DOS 3.3 prologue, because these records are
/// not files.
///
/// **Compression**, §8.4's second scheme: read a byte; if it is **zero** it is
/// an escape and the next two bytes are the repeat count and the first data
/// byte; if it is non-zero it is itself the first data byte with a count of
/// one. Either way one more byte follows as the second data byte, and a count
/// of zero means one. The pair is written that many times.
///
/// **Placement** is the plain sub-variant's: the pair goes to rows *y* and
/// *y* + 1 of the current byte column, *y* advances by two, and when it
/// reaches the stored height the column advances and *y* resets to the
/// vertical offset. Decoding ends when the column reaches the stored width.
///
/// **§8.4's per-release row table is not needed and does not exist.** The
/// section says the row address "is not computed but read from a 0x182-byte
/// table taken off the game disk", at `M2` file offset `0x174B`. Measured on
/// all three: the 384 bytes there are **byte-identical across the three
/// titles** and are exactly the standard Apple II high-resolution interleave —
/// `1024 * (y mod 8) + 128 * ((y / 8) mod 8) + 40 * (y / 64)` — for every one
/// of the 192 rows. It is a lookup table for an address computation, not a
/// descrambling of anything, so a decoder that computes the address is reading
/// the same picture. (The two bytes past `0x181` are the only part that
/// differs between the three, and they are not part of the table.)
///
/// The answer is [`CANVAS_HEIGHT`]-independent: a picture is `7 * width` by
/// `height` pixels, which on every specimen is §8.4's nominal 280 x 160 —
/// **not** the plain sub-variant's whole 192-row page.
///
/// # Errors
///
/// [`AppleError`] — a record too short to carry a header, a header describing
/// no pixels, or a platform that does not use family D.
pub fn decode_family_d_scrambled(
    record: &[u8],
    platform: SagaPlatform,
) -> Result<HiResPicture, AppleError> {
    if !matches!(platform, SagaPlatform::AppleII) {
        return Err(AppleError::NotAppleII { platform });
    }
    if record.len() < 4 {
        return Err(AppleError::TooShort { len: record.len() });
    }
    let (hoff, voff) = (usize::from(record[0]), usize::from(record[1]));
    let width = usize::from(record[2]).min(COLUMNS);
    let height = usize::from(record[3]).min(CANVAS_HEIGHT);
    if width == 0 || height <= voff || hoff >= width {
        return Err(AppleError::EmptyStream);
    }

    // Black, and it never shows: every byte inside the declared box is
    // written, and the answer is cropped to that box.
    let mut page =
        Page { bytes: vec![0x00; COLUMNS * CANVAS_HEIGHT], painted: PaintedBox::default() };
    let mut col = hoff;
    let mut y = voff;
    let mut i = 4usize;
    'stream: while i < record.len() {
        let b = record[i];
        i += 1;
        let (mut count, first) = if b == 0 {
            let (Some(&count), Some(&first)) = (record.get(i), record.get(i + 1)) else { break };
            i += 2;
            (usize::from(count), first)
        } else {
            (1, b)
        };
        let Some(&second) = record.get(i) else { break };
        i += 1;
        if count == 0 {
            count = 1;
        }
        for _ in 0..count {
            if col >= width {
                break 'stream;
            }
            if y < CANVAS_HEIGHT {
                page.bytes[y * COLUMNS + col] = first;
            }
            if y + 1 < CANVAS_HEIGHT {
                page.bytes[(y + 1) * COLUMNS + col] = second;
            }
            y += 2;
            if y >= height {
                col += 1;
                y = voff;
            }
        }
    }

    let resolved = page.resolve();
    let out_w = width * 7;
    let mut pixels = Vec::with_capacity(out_w * height);
    for row in 0..height {
        pixels.extend_from_slice(&resolved[row * CANVAS_WIDTH..row * CANVAS_WIDTH + out_w]);
    }
    // Every byte of the declared box was written, so that box IS what this
    // record painted — pixel-exact, and cheaper than tracking it a byte at a
    // time through the run-length loop.
    let painted = Painted { left: hoff * 7, top: voff, right: out_w - 1, bottom: height - 1 };
    Ok(HiResPicture { width: out_w, height, pixels, painted: Some(painted) })
}

/// Decode one **plain** family-D picture file — the opcode stream the module
/// doc measures, which is what the four releases with an ordinary DOS 3.3 side
/// A carry.
///
/// [`decode_family_d`] is the entry point that picks between this and
/// [`decode_family_d_scrambled`]; call this one when the record's
/// sub-variant is already known.
///
/// `file` is the whole DOS 3.3 binary file as the disk's track/sector list
/// gives it, **including** the four-byte prologue: bytes 0-1 the load address
/// (`$7000` on every specimen, ignored here) and bytes 2-3 a little-endian
/// data length. The opcode stream is that many bytes from offset 4, bounded
/// by the end of the slice so a short read decodes what it has instead of
/// refusing.
///
/// The picture is drawn as a **room** picture: over a white page, with the
/// stream's coordinates taken as absolute. An object picture — a `Bnnnnn`
/// file — opens with a move-shaped anchor the release's overlay path reads as
/// an origin and this decoder plays as an ordinary move, which places the
/// artwork exactly where it was authored; only compositing it onto a room
/// picture would need the anchor read as an anchor.
///
/// # Errors
///
/// [`AppleError`] — a file too short to carry a prologue, a prologue declaring
/// an empty stream, or a platform that does not use family D at all.
pub fn decode_family_d_plain(
    file: &[u8],
    platform: SagaPlatform,
) -> Result<HiResPicture, AppleError> {
    if !matches!(platform, SagaPlatform::AppleII) {
        return Err(AppleError::NotAppleII { platform });
    }
    if file.len() < 4 {
        return Err(AppleError::TooShort { len: file.len() });
    }
    let declared = usize::from(u16::from_le_bytes([file[2], file[3]]));
    let end = 4usize.saturating_add(declared).min(file.len());
    let data = &file[4..end];
    if data.is_empty() {
        return Err(AppleError::EmptyStream);
    }

    let mut page = Page::white();
    // The attribute defaults the release's own loader installs before every
    // picture: HCOLOR 4, brush 5, paint 0, pen at (140, 96).
    let mut hcolor = 4usize;
    let mut brush = 5usize;
    let mut paint = PAINTS[0];
    let mut cur = (140i32, 96i32);

    let mut i = 0usize;
    while i < data.len() {
        let b = data[i];
        if b & 0x80 == 0 {
            match b >> 5 {
                // 0x00-0x1F: end of picture. Measured: exactly one per file,
                // always the last token, always at the declared end.
                0 => break,
                1 => {
                    hcolor = usize::from(b & 0x0F) & 7;
                    i += 1;
                }
                2 => {
                    brush = usize::from(b & 0x0F) & 7;
                    i += 1;
                }
                // 0x60: the only two-byte token. An operand past the table is
                // left as it is rather than wrapping into a colour nobody
                // chose; none occurs in the corpus.
                _ => {
                    if let Some(&v) = data.get(i + 1) {
                        if let Some(&p) = PAINTS.get(usize::from(v)) {
                            paint = p;
                        }
                    }
                    i += 2;
                }
            }
            continue;
        }
        if i + 3 > data.len() {
            break;
        }
        let x = i32::from(data[i + 1]) | (i32::from(b & 1) << 8);
        let y = i32::from(data[i + 2]);
        i += 3;
        match b & 0xE0 {
            0x80 => cur = (x, y),
            0xA0 => {
                page.line(cur, (x, y), hcolor);
                cur = (x, y);
            }
            // A brush stamp and an area fill both leave the line pen where it
            // was: the release plays them through its own routines and never
            // touches Applesoft's cursor.
            0xC0 => page.brush(x, y, brush, paint),
            _ => page.fill(x, y, paint),
        }
    }

    Ok(HiResPicture {
        width: CANVAS_WIDTH,
        height: CANVAS_HEIGHT,
        pixels: page.resolve(),
        painted: page.painted.finish(),
    })
}

/// Decode one family-D picture, whichever sub-variant it is (SQ-1490).
///
/// The host that walks a disk knows which release it opened but not, in
/// general, which of the two shapes a given record has — so the record says.
/// The two are told apart by their first bytes, and the discriminator is
/// measured on the whole corpus rather than assumed:
///
/// * A **plain** record is a DOS 3.3 binary FILE, so it opens with that file
///   type's four-byte prologue, and the load address in its first two bytes is
///   `$7000` — bytes `00 70` — on **all 314** of the four plain releases'
///   picture files.
/// * A **scrambled** record is not a file at all but a run of sectors, and
///   opens with §8.4's own four-byte header, which on all 97 records of the
///   three scrambled releases is `00 00 28 A0`. Its second byte is a vertical
///   row offset and can never be `0x70`, because that is a load address's high
///   byte and this header has no load address in it.
///
/// So: bytes 0-1 of `00 70` select [`decode_family_d_plain`], a plausible
/// §8.4 header selects [`decode_family_d_scrambled`], and anything else falls
/// to the plain decoder, whose refusals (§11) are the right ones to report for
/// a record that is neither.
///
/// # Errors
///
/// [`AppleError`], from whichever decoder the record selected.
pub fn decode_family_d(record: &[u8], platform: SagaPlatform) -> Result<HiResPicture, AppleError> {
    if !matches!(platform, SagaPlatform::AppleII) {
        return Err(AppleError::NotAppleII { platform });
    }
    if record.len() < 4 {
        return Err(AppleError::TooShort { len: record.len() });
    }
    let plain_prologue = record[0] == 0x00 && record[1] == 0x70;
    let scrambled_header = usize::from(record[0]) < COLUMNS
        && usize::from(record[1]) < CANVAS_HEIGHT
        && (1..=COLUMNS).contains(&usize::from(record[2]))
        && (1..=CANVAS_HEIGHT).contains(&usize::from(record[3]));
    if !plain_prologue && scrambled_header {
        decode_family_d_scrambled(record, platform)
    } else {
        decode_family_d_plain(record, platform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture file: the four-byte prologue (load address `$7000` and the
    /// stream's length) and then the stream.
    fn file(stream: &[u8]) -> Vec<u8> {
        let mut out = vec![0x00, 0x70];
        out.extend_from_slice(&u16::try_from(stream.len()).unwrap().to_le_bytes());
        out.extend_from_slice(stream);
        out
    }

    fn decode(stream: &[u8]) -> HiResPicture {
        decode_family_d_plain(&file(stream), SagaPlatform::AppleII).expect("decodes")
    }

    fn at(pic: &HiResPicture, x: usize, y: usize) -> Rgb {
        pic.rgb(x, y).expect("on the canvas")
    }

    const BLACK: Rgb = PALETTE[0];
    const PURPLE: Rgb = PALETTE[1];
    const GREEN: Rgb = PALETTE[2];
    const BLUE: Rgb = PALETTE[3];
    const ORANGE: Rgb = PALETTE[4];
    const WHITE: Rgb = PALETTE[5];

    // The ground is white, not black. An empty stream that only ends the
    // picture leaves the whole canvas white — which is the fact that inverts
    // every picture in this family if it is got wrong.
    #[test]
    fn the_ground_is_white() {
        let pic = decode(&[0x00]);
        assert_eq!((pic.width, pic.height), (CANVAS_WIDTH, CANVAS_HEIGHT));
        assert!(pic.pixels.iter().all(|&v| v == 5), "every pixel is PALETTE's white");
        assert_eq!(at(&pic, 0, 0), WHITE);
        assert_eq!(at(&pic, CANVAS_WIDTH - 1, CANVAS_HEIGHT - 1), WHITE);
        assert_eq!(pic.rgb(CANVAS_WIDTH, 0), None, "and nothing off it");
    }

    // A `0x20`-class token sets the line colour, and a line takes it. HCOLOR
    // 4 is the default and is black in the high-bit palette, so the FIRST
    // stroke of a picture that never sets a colour is invisible on white —
    // which is why the default has to be right rather than convenient.
    #[test]
    fn a_line_takes_the_hcolor_the_attribute_token_selected() {
        // HCOLOR 7 is white2: on a white ground it changes nothing but the
        // palette bit, so pin it against HCOLOR 0 (black1), which erases.
        let black = decode(&[0x20, 0x80, 10, 20, 0xA0, 40, 20]);
        for x in 10..=40 {
            assert_eq!(at(&black, x, 20), BLACK, "x {x} of a black1 line");
        }
        assert_eq!(at(&black, 9, 20), WHITE, "one pixel before it");
        assert_eq!(at(&black, 41, 20), WHITE, "one pixel after it");
        assert_eq!(at(&black, 10, 19), WHITE, "the row above");

        // …and the default is HCOLOR 4, which is black too, so a picture that
        // sets no colour at all still draws a black line.
        let default = decode(&[0x80, 10, 20, 0xA0, 40, 20]);
        for x in 12..=38 {
            assert_eq!(at(&default, x, 20), BLACK, "x {x} of the default line");
        }
    }

    // The six colours, each drawn as a long horizontal line over the black
    // ground its own two-colour mask leaves behind. This is the whole palette
    // pinned by a pixel apiece, in the Applesoft HCOLOR order the `0x20`
    // operand uses: 1 green, 2 violet, 5 orange, 6 blue.
    #[test]
    fn every_hcolor_resolves_to_the_colour_the_hardware_gives_it() {
        for (hcolor, want, what) in [
            (0u8, BLACK, "black1"),
            (1, GREEN, "green"),
            (2, PURPLE, "violet"),
            (3, WHITE, "white1"),
            (4, BLACK, "black2"),
            (5, ORANGE, "orange"),
            (6, BLUE, "blue"),
            (7, WHITE, "white2"),
        ] {
            // Clear the row to black first, then draw the colour over it, so
            // the white ground cannot flatter a colour into white.
            let pic = decode(&[
                0x20, 0x80, 0, 30, 0xA1, 0x17, 30, // black1 right across row 30
                0x20 | hcolor, 0x80, 20, 30, 0xA0, 200, 30,
            ]);
            for x in [40usize, 41, 100, 101, 160, 161] {
                assert_eq!(at(&pic, x, 30), want, "HCOLOR {hcolor} ({what}) at x {x}");
            }
        }
    }

    // A coloured line keeps its hue across a byte boundary, which is what
    // Applesoft's odd-column rotation exists for: seven pixels to a byte is
    // odd, so an unrotated mask would flip green to violet every seven pixels.
    #[test]
    fn a_coloured_line_does_not_change_hue_at_a_byte_boundary() {
        let pic = decode(&[
            0x20, 0x80, 0, 40, 0xA0, 200, 40, // black1 across row 40
            0x21, 0x80, 0, 40, 0xA0, 200, 40, // then green over it
        ]);
        // Byte columns 0, 1 and 2 are x 0-6, 7-13 and 14-20.
        for x in [2usize, 4, 8, 10, 16, 18] {
            assert_eq!(at(&pic, x, 40), GREEN, "x {x} stays green");
        }
        assert_eq!(HCOLOR_MASKS[1], 0x2A, "green's mask");
        assert_eq!(HCOLOR_MASKS_ODD_COLUMN[1], 0x55, "and the same mask on an odd column");
    }

    // Bit 0 of the command byte is bit 8 of x, which is the only way a
    // 280-wide canvas fits in a byte-oriented stream: 0xA1 with a low byte of
    // 0x17 is x = 279, the last column.
    #[test]
    fn bit_zero_of_the_command_is_the_ninth_bit_of_x() {
        let pic = decode(&[0x20, 0x81, 0x15, 9, 0xA1, 0x17, 9]);
        for x in 277..=279 {
            assert_eq!(at(&pic, x, 9), BLACK, "x {x}");
        }
        assert_eq!(at(&pic, 276, 9), WHITE, "one pixel left of the run");
        // And without the ninth bit the same pair would sit at x 21..=23.
        let low = decode(&[0x20, 0x80, 0x15, 9, 0xA0, 0x17, 9]);
        assert_eq!(at(&low, 22, 9), BLACK);
        assert_eq!(at(&low, 278, 9), WHITE);
    }

    // A move inks nothing on its own, and the SECOND move is where the next
    // line starts from — two moves in a row are not a line.
    #[test]
    fn a_move_inks_nothing() {
        let pic = decode(&[0x20, 0x80, 4, 6, 0x80, 40, 6]);
        assert!(pic.pixels.iter().all(|&v| v == 5), "two moves leave the canvas white");
    }

    // `0x60` is a TWO-byte token: its operand is the next raw byte, and the
    // byte after that is the next opcode. Read it as one byte and the operand
    // is played as an opcode, which is the mis-framing this pins against.
    #[test]
    fn the_paint_token_takes_an_operand() {
        // Paint 0x50 is (0, 0), solid black. Fill the whole white canvas with
        // it: one seed anywhere reaches every pixel.
        let pic = decode(&[0x60, 0x50, 0xE0, 100, 100]);
        assert!(pic.pixels.iter().all(|&v| v == 0), "paint 0x50 is solid black");

        // …and the operand is NOT executed: 0x50 read as an opcode would be a
        // brush token, leaving the fill to run in paint 0 (white).
        let unread = decode(&[0x60, 0x4D, 0xE0, 100, 100]);
        assert_eq!(at(&unread, 100, 100), WHITE, "paint 0x4D is solid white1");
        assert_eq!(PAINTS[0x50], (0, 0));
        assert_eq!(PAINTS[0x4D], (3, 3));
    }

    // A paint is a PAIR: one pattern for even rows and one for odd. Paint
    // 0x06 is (0, 4) — black in both palettes, so it reads as flat black —
    // while 0x4C is (5, 6), blue on even rows and orange on odd, which is the
    // two-row dither the artwork uses for a muted ground.
    #[test]
    fn a_paint_is_one_pattern_for_even_rows_and_another_for_odd() {
        let dither = decode(&[0x60, 0x4C, 0xE0, 100, 100]);
        for y in [40usize, 42, 100] {
            assert_eq!(at(&dither, 100, y), BLUE, "row {y} is the even pattern");
        }
        for y in [41usize, 43, 101] {
            assert_eq!(at(&dither, 100, y), ORANGE, "row {y} is the odd pattern");
        }
        assert_eq!(PAINTS[0x4C], (5, 6), "blue over orange");

        let flat = decode(&[0x60, 0x06, 0xE0, 100, 100]);
        assert!(flat.pixels.iter().all(|&v| v == 0), "0x06 is black on both row parities");
    }

    // A fill spreads over LIT pixels and stops at unlit ones — the ground is
    // white and the outlines are dark, so a region is a run of set pixels.
    #[test]
    fn a_fill_stops_at_the_lines_that_bound_it() {
        // A black box from (10,10) to (30,30), then a green fill inside it.
        let stream = [
            0x20, // HCOLOR 0, black1
            0x80, 10, 10, 0xA0, 30, 10, 0xA0, 30, 30, 0xA0, 10, 30, 0xA0, 10, 10, //
            0x60, 0x57, // paint 0x57 = (2, 2), solid green
            0xE0, 20, 20,
        ];
        let pic = decode(&stream);
        assert_eq!(at(&pic, 20, 20), GREEN, "inside the box");
        assert_eq!(at(&pic, 11, 29), GREEN, "and right up to the wall");
        assert_eq!(at(&pic, 20, 10), BLACK, "the wall itself is not painted");
        assert_eq!(at(&pic, 40, 20), WHITE, "and nothing outside it");
        assert_eq!(PAINTS[0x57], (2, 2));

        // A seed ON an unlit pixel fills nothing at all: there is no region.
        let mut on_the_wall = stream;
        on_the_wall[18] = 20;
        on_the_wall[19] = 10;
        let pic = decode(&on_the_wall);
        assert_eq!(at(&pic, 20, 20), WHITE, "a seed on the wall paints nothing");
    }

    // `0xC0` stamps a brush and does NOT move the line pen: the line after
    // one still runs from where the drawing left off, which is what keeps a
    // stroke from being flung across the picture.
    #[test]
    fn a_brush_stamps_a_blob_and_leaves_the_line_pen_alone() {
        // Black the whole canvas first (paint 0x50), then stamp in solid
        // white (paint 0x34) so the blob is what stands out.
        let ground = [0x60, 0x50, 0xE0, 100, 100];

        let mut disc = ground.to_vec();
        disc.extend_from_slice(&[0x45, 0x60, 0x34, 0xC0, 100, 100]);
        let disc = decode(&disc);
        let white = disc.pixels.iter().filter(|&&v| v == 5).count();
        assert!((120..=220).contains(&white), "brush 5 covers {white} white pixels, not a disc");
        assert_eq!(at(&disc, 107, 107), WHITE, "its middle");
        assert_eq!(at(&disc, 100, 92), BLACK, "and not eight rows above it");

        // Brush 0 is a single pixel at (x+7, y+7). One lit pixel with unlit
        // neighbours is a colour fringe rather than white — the artifact model
        // working — so this pins that it is lit at all, and that its
        // neighbours are not.
        let mut dot = ground.to_vec();
        dot.extend_from_slice(&[0x40, 0x60, 0x34, 0xC0, 100, 100]);
        let dot = decode(&dot);
        assert_ne!(at(&dot, 107, 107), BLACK, "brush 0's one pixel is lit");
        assert_eq!(at(&dot, 105, 107), BLACK, "and nothing two pixels away");
        assert_eq!(at(&dot, 107, 106), BLACK, "nor the row above");

        // The pen: move, line, brush, line. Without the brush the second line
        // runs from (60,50); with it, it still does.
        let with = decode(&[0x20, 0x80, 20, 50, 0xA0, 60, 50, 0x40, 0xC0, 200, 150, 0xA0, 60, 90]);
        // A one-pixel-wide dark line on a white ground reads as a colour
        // fringe rather than black — two lit neighbours either side of an
        // unlit pixel is exactly the artifact §8.4's model reproduces — so
        // this asks whether the ground was disturbed, not what hue it took.
        assert_ne!(at(&with, 60, 70), WHITE, "the stroke ran from the line pen");
        assert_eq!(at(&with, 130, 120), WHITE, "and not from the brush's point");
    }

    // A byte with bit 7 clear and a top-three-bit class of zero ends the
    // picture. Measured: 314 files, 314 such bytes, every one of them the
    // last token and at the declared end of its stream.
    #[test]
    fn a_class_zero_byte_ends_the_picture() {
        let pic = decode(&[0x20, 0x80, 4, 6, 0xA0, 40, 6, 0x00, 0x80, 4, 40, 0xA0, 40, 40]);
        assert_eq!(at(&pic, 20, 6), BLACK, "what came before the end token is drawn");
        assert_eq!(at(&pic, 20, 40), WHITE, "and what came after it is not");
    }

    // An area token off the canvas is as harmless as one on it, and neither
    // hangs.
    #[test]
    fn an_area_token_off_the_canvas_is_harmless() {
        let off = decode(&[0x60, 0x50, 0xE1, 0xFF, 200]);
        assert!(off.pixels.iter().all(|&v| v == 5), "nothing was painted");
    }

    // A coordinate past the canvas is clipped, not refused: eight of the
    // corpus's pictures carry one.
    #[test]
    fn an_off_canvas_coordinate_is_clipped() {
        let pic = decode(&[0x20, 0x80, 20, 180, 0xA0, 20, 253]);
        assert_ne!(at(&pic, 20, CANVAS_HEIGHT - 1), WHITE, "the last row on the canvas");
        assert_eq!(pic.pixels.len(), CANVAS_WIDTH * CANVAS_HEIGHT, "and no growth");
    }

    // A token cut off by the end of the stream stops the decode rather than
    // reading past it or spinning.
    #[test]
    fn a_truncated_token_terminates() {
        let pic = decode(&[0x20, 0x80, 4, 6, 0xA0, 40, 6, 0xA0, 40]);
        assert_eq!(at(&pic, 20, 6), BLACK, "what was complete is drawn");
        assert_eq!(at(&pic, 40, 8), WHITE, "the truncated token is not");
    }

    // The prologue's length bounds the stream, and a stream shorter than the
    // prologue claims decodes what is actually there.
    #[test]
    fn the_prologue_length_bounds_the_stream() {
        let mut f = file(&[0x20, 0x80, 4, 6, 0xA0, 40, 6]);
        // Say the stream is four bytes long: only the colour and the move.
        f[2] = 4;
        f[3] = 0;
        let pic = decode_family_d(&f, SagaPlatform::AppleII).expect("decodes");
        assert!(pic.pixels.iter().all(|&v| v == 5), "the line was past the declared end");

        // …and a declared length past the end of the file reads what is there.
        let mut over = file(&[0x20, 0x80, 4, 6, 0xA0, 40, 6]);
        over[2] = 0xFF;
        over[3] = 0xFF;
        let pic = decode_family_d(&over, SagaPlatform::AppleII).expect("decodes");
        assert_eq!(at(&pic, 20, 6), BLACK, "the run is still drawn");
    }

    // The three tables' shapes, which are what the corpus's operand ranges
    // are checked against: 30 patterns of four bytes, 108 paints, 8 brushes of
    // 32 bytes, and every paint naming a pattern that exists.
    #[test]
    fn the_tables_have_the_shape_the_operands_need() {
        assert_eq!(PATTERNS.len(), 30);
        assert_eq!(PAINTS.len(), 0x6C, "the largest paint operand in the corpus is 0x6B");
        assert_eq!(BRUSHES.len(), 8, "a 0x40 operand is three bits wide in practice");
        for (i, &(even, odd)) in PAINTS.iter().enumerate() {
            assert!(usize::from(even) < PATTERNS.len(), "paint {i:#04X} even row");
            assert!(usize::from(odd) < PATTERNS.len(), "paint {i:#04X} odd row");
        }
        // Patterns 0-7 are the eight Applesoft colours with their parity kept
        // across byte columns, which is the mask and its odd-column twin.
        for c in 0..8usize {
            let pattern = PATTERNS[[0, 2, 1, 3, 4, 6, 5, 7][c]];
            assert_eq!(pattern[0], HCOLOR_MASKS[c], "HCOLOR {c} on an even column");
            assert_eq!(pattern[1], HCOLOR_MASKS_ODD_COLUMN[c], "HCOLOR {c} on an odd column");
        }
    }

    #[test]
    fn refusals() {
        assert_eq!(
            decode_family_d(&[0x00, 0x70, 4], SagaPlatform::AppleII),
            Err(AppleError::TooShort { len: 3 })
        );
        assert_eq!(
            decode_family_d(&[0x00, 0x70, 0, 0], SagaPlatform::AppleII),
            Err(AppleError::EmptyStream)
        );
        for platform in [SagaPlatform::Commodore64, SagaPlatform::Atari8Bit] {
            assert_eq!(
                decode_family_d(&file(&[0x80, 4, 6]), platform),
                Err(AppleError::NotAppleII { platform }),
                "{platform:?} is family C"
            );
        }
    }
}
