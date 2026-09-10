//! Picture family **D** — the Apple II artwork of the US S.A.G.A. releases
//! (SQ-1476), as shipped on the seven two-sided *Scott Adams Graphic
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
//! stream of absolute coordinates** — move, draw, area — over the
//! machine's 280 x 192 hi-res canvas, with no page, no interleave, no
//! compression and no artifact model. §8.4's description does fit the three
//! *scrambled* releases' `PAK.*` files, which really do open with its
//! four-byte header (`00 00 28 A0` — offset 0, offset 0, 40 byte columns, 160
//! rows), so the section is right about a format this crate cannot currently
//! reach and wrong about the one it can: the scrambled releases keep their
//! ROOM pictures on a side A that is not a DOS 3.3 disk at all, at the
//! per-title offsets §12.10 says are "not recoverable from the database"
//! (SQ-1490). Appendix A item 26 records the measurement.
//!
//! # The format, as measured
//!
//! **Canvas** [`CANVAS_WIDTH`] x [`CANVAS_HEIGHT`] — the Apple II hi-res
//! screen, 280 x 192, one bit of ink per pixel.
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
//! **Tokens.** A byte with **bit 7 set** opens a three-byte token: bits 7-5
//! are the command, **bit 0 is bit 8 of the horizontal coordinate** (280 does
//! not fit in a byte), and the two bytes after it are the low eight bits of
//! *x* and then *y*.
//!
//! | command | effect |
//! |---|---|
//! | `0x80` | **move** — the current point becomes (*x*, *y*); no ink |
//! | `0xA0` | **draw** — a line from the current point to (*x*, *y*), which becomes the current point |
//! | `0xC0` | **draw**, as `0xA0` (see below) |
//! | `0xE0` | **area** — names a point inside a region to be coloured; the current point does not move. **Not painted here; see below** |
//!
//! A byte with **bit 7 clear** is a one-byte token that **ends the current
//! path**: the next drawing command sets the current point without inking.
//! Records end with one of these (`0x00` on nearly every specimen).
//!
//! # What is undetermined, and is said here rather than guessed at
//!
//! **The difference between the two drawing commands.** `0xA0` and `0xC0`
//! both plainly draw — the *Pirate Adventure* darkness card's lettering is
//! `0xC0` throughout while its symbol is `0xA0` throughout, and
//! *Adventureland*'s darkness card draws its lettering with `0xA0` — so a pen
//! or colour distinction is the likely reading and nothing in the corpus
//! falsifies either colour. This decoder draws both in [`INK`].
//!
//! **What a one-byte token SAYS**, beyond ending the path. A colour would fit:
//! they cluster in short runs immediately before a drawing command, and 92
//! distinct values occur. Nothing measurable settles it.
//!
//! **And therefore the `0xE0` areas are not painted.** Read as a
//! flood fill in the one ink this decoder has, they are catastrophic rather
//! than merely wrong: **172 of the corpus's 314 pictures wash out**, because a
//! fill plainly paints in a colour the outline is not — *Adventureland*'s
//! darkness card opens with an `0xE0` before a single line has been drawn, so
//! its region is the whole empty canvas. A fill that paints the whole picture
//! is not a picture, and the alternative to painting it wrongly is not
//! painting it: the line art the drawing commands describe is a strict subset
//! of the artwork and is legible on every specimen, which is what
//! [`decode_family_d`] returns. `0xE0` is recognised, consumes its operands,
//! and inks nothing. SQ-1489 carries the colour model.
//!
//! (`0xE0` is an *area* rather than certainly a *fill* for the same reason:
//! read as a line it draws long diagonals across otherwise clean artwork, and
//! read as a seed it lands inside the closed outlines the drawing commands
//! have just built — but whether the region is flooded, hatched or merely
//! recoloured is exactly the undetermined part.)
//!
//! # How the reading was settled
//!
//! Not from the specification, which describes another format, but from the
//! specimens, three ways.
//!
//! - **The ninth bit of *x*.** Parsed as `(command, x, y)` triples with an
//!   eight-bit *x*, the four titles' 314 picture files yield a scatter of
//!   every byte value in the command position. Read with bit 0 as *x*'s ninth
//!   bit, the command position holds **only** `0x80`, `0xA0`, `0xC0` and
//!   `0xE0` across all **71,899** three-byte tokens, and **not one of their
//!   coordinates lands off the 280 x 192 canvas**. `crates/scott/tests/`'s
//!   `apple_pictures_specimens` runs that census on the real disks.
//! - **The one-byte token.** Trying one, two and three bytes for a bit-7-clear
//!   byte (there are 6,272 of them): one byte gives zero off-canvas
//!   coordinates, two gives 6.75% and three gives 9.89%.
//! - **What the pictures are.** `R0100` decodes to the words `IT'S TOO DARK!`
//!   — §8.6's reserved index 0, the darkness picture — `R0198` to an
//!   `INVENTORY` card (§8.6's reserved 98), `R0199` to the Adventure
//!   International globe-and-`ai` logo (§8.6's reserved 99), and `B01255` to
//!   the word `Adventureland`. A wrong interleave or a wrong bit order does
//!   not produce legible lettering.
//!
//! The rule that a one-byte token ends the path came from the same place:
//! without it the two `0xC0` tokens that follow a one-byte run on *Pirate
//! Adventure*'s darkness card draw long diagonals across the lettering, and
//! with it that card is clean.

use crate::saga_pictures::{Picture, Rgb};
use crate::saga_us::SagaPlatform;

/// The family-D canvas width in pixels — the Apple II hi-res screen.
pub const CANVAS_WIDTH: usize = 280;

/// The family-D canvas height in pixels — the Apple II hi-res screen.
///
/// The whole 192-row page, not the 160 rows a mixed-mode screen shows above
/// four text lines: measured across the four plain releases' 314 pictures, 29
/// of them ink at or below row 160 and the deepest reaches row 170.
pub const CANVAS_HEIGHT: usize = 192;

/// The colour ink is drawn in.
///
/// One colour, because no opcode in the stream is known to select one — see
/// the module doc on what is undetermined. White on black is what a hi-res
/// line drawing with no colour information is.
pub const INK: Rgb = (255, 255, 255);

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

/// Decode one family-D picture file (the format the module doc measures).
///
/// `file` is the whole DOS 3.3 binary file as the disk's track/sector list
/// gives it, **including** the four-byte prologue: bytes 0-1 the load address
/// (`$7000` on every specimen, ignored here) and bytes 2-3 a little-endian
/// data length. The opcode stream is that many bytes from offset 4, bounded
/// by the end of the slice so a short read decodes what it has instead of
/// refusing.
///
/// The answer is the same [`Picture`] family C returns, over this family's own
/// canvas: `pixels` are 0 for background and 1 for ink, `palette` is black and
/// [`INK`], and `colour_bytes` is all zero because a family-D file stores
/// none.
///
/// # Errors
///
/// [`AppleError`] — a file too short to carry a prologue, a prologue declaring
/// an empty stream, or a platform that does not use family D at all.
pub fn decode_family_d(file: &[u8], platform: SagaPlatform) -> Result<Picture, AppleError> {
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

    let mut pixels = vec![0u8; CANVAS_WIDTH * CANVAS_HEIGHT];
    let mut painted = crate::saga_pictures::PaintedBox::default();
    // The pen starts up: nothing has said where a first line would come from,
    // and every specimen opens with a move in any case.
    let mut cur = (0i32, 0i32);
    let mut pen_up = true;

    let mut i = 0usize;
    while i < data.len() {
        let b = data[i];
        if b & 0x80 == 0 {
            // A one-byte token: end the current path.
            i += 1;
            pen_up = true;
            continue;
        }
        if i + 3 > data.len() {
            break;
        }
        let x = i32::from(data[i + 1]) | (i32::from(b & 1) << 8);
        let y = i32::from(data[i + 2]);
        i += 3;
        match b & 0xE0 {
            0x80 => {
                cur = (x, y);
                pen_up = false;
            }
            0xA0 | 0xC0 => {
                if pen_up {
                    pen_up = false;
                } else {
                    line(&mut pixels, &mut painted, cur, (x, y));
                }
                cur = (x, y);
            }
            // 0xE0, the only value left: an area, which inks nothing here
            // (see the module doc) and leaves the pen where it was — which is
            // what keeps the `0xC0` after one from drawing a stroke across
            // the picture.
            _ => {}
        }
    }

    Ok(Picture {
        width: CANVAS_WIDTH,
        height: CANVAS_HEIGHT,
        pixels,
        palette: [(0, 0, 0), INK, (0, 0, 0), (0, 0, 0)],
        colour_bytes: [0; 4],
        unrecognised_colours: Vec::new(),
        painted: painted.finish(),
    })
}

/// Ink one pixel, dropping anything off the canvas.
///
/// Defensive rather than needed: not one of the corpus's 71,899 coordinates
/// is off the canvas (see the module doc's census), and a decoder that
/// panicked on the first one that was would be a poor way to find that out.
fn plot(pixels: &mut [u8], painted: &mut crate::saga_pictures::PaintedBox, x: i32, y: i32) {
    if (0..CANVAS_WIDTH as i32).contains(&x) && (0..CANVAS_HEIGHT as i32).contains(&y) {
        pixels[y as usize * CANVAS_WIDTH + x as usize] = 1;
        painted.mark(x as usize, y as usize);
    }
}

/// A Bresenham line with both endpoints inked.
fn line(
    pixels: &mut [u8],
    painted: &mut crate::saga_pictures::PaintedBox,
    from: (i32, i32),
    to: (i32, i32),
) {
    let (mut x, mut y) = from;
    let dx = (to.0 - x).abs();
    let dy = -(to.1 - y).abs();
    let sx = if x < to.0 { 1 } else { -1 };
    let sy = if y < to.1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        plot(pixels, painted, x, y);
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

    fn decode(stream: &[u8]) -> Picture {
        decode_family_d(&file(stream), SagaPlatform::AppleII).expect("decodes")
    }

    fn ink(pic: &Picture, x: usize, y: usize) -> bool {
        pic.pixels[y * CANVAS_WIDTH + x] == 1
    }

    // Move to (4, 6), draw to (9, 6): a horizontal run of six inked pixels,
    // both endpoints included, and nothing either side of it. Worked out by
    // hand from the module doc's table, not from a specimen.
    #[test]
    fn a_move_and_a_draw_ink_the_run_between_them() {
        let pic = decode(&[0x80, 4, 6, 0xA0, 9, 6]);
        assert_eq!((pic.width, pic.height), (CANVAS_WIDTH, CANVAS_HEIGHT));
        for x in 4..=9 {
            assert!(ink(&pic, x, 6), "x {x} of the run");
        }
        assert!(!ink(&pic, 3, 6), "one pixel before the run");
        assert!(!ink(&pic, 10, 6), "one pixel after the run");
        assert!(!ink(&pic, 4, 5), "the row above");
        assert_eq!(pic.rgb(4, 6), Some(INK));
        assert_eq!(pic.rgb(3, 6), Some((0, 0, 0)));
        assert_eq!(pic.palette[0], (0, 0, 0), "value 0 is the background");
        assert_eq!(pic.colour_bytes, [0; 4], "family D stores no colour bytes");
        assert!(pic.unrecognised_colours.is_empty());
    }

    // A move inks nothing on its own, and the SECOND move is where the next
    // line starts from — two moves in a row are not a line.
    #[test]
    fn a_move_inks_nothing() {
        let pic = decode(&[0x80, 4, 6, 0x80, 40, 6]);
        assert!(pic.pixels.iter().all(|&v| v == 0), "two moves ink nothing");
    }

    // Bit 0 of the command byte is bit 8 of x, which is the only way a
    // 280-wide canvas fits in a byte-oriented stream: 0xA1 with a low byte of
    // 0x17 is x = 279, the last column.
    #[test]
    fn bit_zero_of_the_command_is_the_ninth_bit_of_x() {
        let pic = decode(&[0x81, 0x15, 9, 0xA1, 0x17, 9]);
        for x in 277..=279 {
            assert!(ink(&pic, x, 9), "x {x}");
        }
        assert!(!ink(&pic, 276, 9), "one pixel left of the run");
        // And without the ninth bit the same pair would sit at x 21..=23.
        let low = decode(&[0x80, 0x15, 9, 0xA0, 0x17, 9]);
        assert!(ink(&low, 21, 9) && ink(&low, 23, 9) && !ink(&low, 277, 9));
    }

    // A byte with bit 7 clear ends the path: the draw after it sets the
    // current point without inking, so the long stroke that would otherwise
    // cross the picture is never drawn. This is the rule that removes the two
    // spurious diagonals from *Pirate Adventure*'s darkness card.
    #[test]
    fn a_one_byte_token_ends_the_path() {
        let joined = decode(&[0x80, 4, 6, 0xA0, 9, 6, 0xA0, 9, 20]);
        assert!(ink(&joined, 9, 13), "without the break the pen draws on");

        let broken = decode(&[0x80, 4, 6, 0xA0, 9, 6, 0x00, 0xA0, 9, 20]);
        assert!(ink(&broken, 4, 6), "the first run is still drawn");
        assert!(!ink(&broken, 9, 13), "the break stops the second stroke");
        // …and the pen is at the new point, so the stroke AFTER that draws.
        let resumed = decode(&[0x80, 4, 6, 0x00, 0xA0, 9, 20, 0xA0, 9, 24]);
        assert!(resumed.pixels.contains(&1), "the third token draws");
        assert!(ink(&resumed, 9, 22), "from (9,20) to (9,24)");
        assert!(!ink(&resumed, 6, 12), "and not from (4,6)");
    }

    // 0xC0 draws exactly as 0xA0 does; the difference between them is
    // undetermined (see the module doc) and neither is a move.
    #[test]
    fn the_second_draw_command_draws_too() {
        let a = decode(&[0x80, 4, 6, 0xA0, 9, 6]);
        let c = decode(&[0x80, 4, 6, 0xC0, 9, 6]);
        assert_eq!(a.pixels, c.pixels);
    }

    // An area token inks nothing at all — the module doc says why — and, the
    // load-bearing half, it does not move the pen: the draw after one still
    // starts where the drawing left off.
    #[test]
    fn an_area_token_inks_nothing_and_leaves_the_pen_alone() {
        // A box from (10,10) to (20,20), then an area inside it.
        let stream = [
            0x80, 10, 10, 0xA0, 20, 10, 0xA0, 20, 20, 0xA0, 10, 20, 0xA0, 10, 10, //
            0xE0, 15, 15, //
        ];
        let pic = decode(&stream);
        assert!(!ink(&pic, 15, 15), "the area's own point is not inked");
        assert!(!ink(&pic, 11, 19), "nor anything inside the box");
        assert!(ink(&pic, 15, 10), "the box itself is still drawn");
        // Exactly the box's perimeter, and not a pixel more.
        assert_eq!(pic.pixels.iter().filter(|&&v| v == 1).count(), 40);

        // The pen is still at (10,10) — the box's last corner — so a draw
        // after the area runs from there, not from the area's point.
        let mut with_stroke = stream.to_vec();
        with_stroke.extend_from_slice(&[0xA0, 10, 30]);
        let after = decode(&with_stroke);
        assert!(ink(&after, 10, 25), "the stroke ran from the corner");
        assert!(!ink(&after, 15, 22), "and not from the area's point");
    }

    // An area token off the canvas is as harmless as one on it, and neither
    // hangs.
    #[test]
    fn an_area_token_off_the_canvas_is_harmless() {
        let off = decode(&[0xE1, 0xFF, 200]);
        assert!(off.pixels.iter().all(|&v| v == 0));
        let at_origin = decode(&[0xE0, 0, 0]);
        assert!(at_origin.pixels.iter().all(|&v| v == 0), "and does not wash the canvas");
    }

    // A coordinate past the canvas is clipped, not refused: eight of the
    // corpus's pictures carry one.
    #[test]
    fn an_off_canvas_coordinate_is_clipped() {
        let pic = decode(&[0x80, 20, 180, 0xA0, 20, 253]);
        assert!(ink(&pic, 20, 191), "the last row on the canvas");
        assert_eq!(pic.pixels.len(), CANVAS_WIDTH * CANVAS_HEIGHT, "and no growth");
    }

    // A token cut off by the end of the stream stops the decode rather than
    // reading past it or spinning.
    #[test]
    fn a_truncated_token_terminates() {
        let pic = decode(&[0x80, 4, 6, 0xA0, 9, 6, 0xA0, 9]);
        assert!(ink(&pic, 4, 6), "what was complete is drawn");
        assert!(!ink(&pic, 9, 8), "the truncated token is not");
    }

    // The prologue's length bounds the stream, and a stream shorter than the
    // prologue claims decodes what is actually there.
    #[test]
    fn the_prologue_length_bounds_the_stream() {
        let mut f = file(&[0x80, 4, 6, 0xA0, 9, 6]);
        // Say the stream is three bytes long: only the move survives.
        f[2] = 3;
        f[3] = 0;
        let pic = decode_family_d(&f, SagaPlatform::AppleII).expect("decodes");
        assert!(pic.pixels.iter().all(|&v| v == 0), "the draw was past the declared end");

        // …and a declared length past the end of the file reads what is there.
        let mut over = file(&[0x80, 4, 6, 0xA0, 9, 6]);
        over[2] = 0xFF;
        over[3] = 0xFF;
        let pic = decode_family_d(&over, SagaPlatform::AppleII).expect("decodes");
        assert!(ink(&pic, 6, 6), "the run is still drawn");
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
