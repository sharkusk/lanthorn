//! Picture family **E** — the MS-DOS *Questprobe* CGA bitmaps (spec §8.5),
//! and the release their database identifies as (§10.7).
//!
//! Two MS-DOS releases are known, both supplied as zips of loose files: *The
//! Hulk* (`START.EXE`, `HULK.BAT`, `ADVENT.DAT` and sixty-eight `.PAK`
//! pictures) and *Questprobe featuring the Human Torch and the Thing*
//! (`FANTFOUR.EXE`, `FANTFOUR.TXT`, `SPL53P.DAT` and sixty-four of them).
//! Their **pictures are one format**; their databases are not, and only the
//! *Hulk*'s is one this crate reads — §10.7: the *Hulk* ships the plain
//! reference text format of §2, and *Fantastic Four*'s encoding "is unknown to
//! this document".
//!
//! # What this module is for
//!
//! [`decode_family_e`] turns one `.PAK` file into the same
//! [`Picture`] family C decodes to, so a host
//! draws either without knowing which it holds. Everything else here is the
//! naming and identity a host needs to ask for the right file:
//! [`parse_picture_file_name`] takes a `.PAK` name apart,
//! [`picture_file_name`] spells a room picture's, and [`identify`] says which
//! release a parsed database is so [`DosRelease::room_picture`] can apply
//! §12.11's *Hulk* remap.
//!
//! # Why the release has to be identified at all
//!
//! Because the MS-DOS *Hulk*'s database is the **reference text format**, not
//! §12's binary one, so [`crate::detect_saga_us`] does not answer for it and
//! [`crate::Database::saga_us`] is `None` — and §12.11's two runtime picture
//! rules (the darkness image, and the *Hulk*'s five remapped room pairs) are
//! properties of the RELEASE rather than of the encoding. The release's own
//! picture set says so out loud: it ships `R0100` (the darkness image) and
//! room pictures for 1-4, 9, 12, 15, 16, 19 and 20 — and **for no other room**,
//! which is exactly §12.11's ten remapped rooms and nothing else.
//!
//! # Family E in one paragraph
//!
//! Four colours, two bits per pixel, most significant pair leftmost, on the
//! same 280-pixel canvas as family C — but stored **row-major with the CGA
//! two-bank interleave**, all the even rows and then all the odd ones, and
//! compressed a byte at a time rather than in pairs. Nothing in it shares any
//! arithmetic with family C, which is what makes the two encodings of the
//! *Hulk*'s artwork a real oracle for each other (§10.1, and
//! `crates/scott/tests/saga_dos_specimens.rs` is that comparison).

use crate::database::Database;
use crate::saga_pictures::{Picture, PictureError, Rgb, CANVAS_HEIGHT, CANVAS_WIDTH};
use crate::saga_us::{hulk_room_picture, PictureFile, PictureUsage};

/// The five bytes every family-E `.PAK` file this catalogue knows begins with,
/// with the two that vary written as `None`.
///
/// **Measured, not stated**: §8.5 gives fixed header POSITIONS and no
/// signature at all. Every one of the *Hulk*'s sixty-eight files opens
/// `FD 07 19 08 6A` and every one of *Fantastic Four*'s sixty-four opens
/// `FD 07 29 04 6A`, so bytes 0, 1 and 4 are the release-independent part and
/// bytes 2-3 are not. [`looks_like_family_e`] tests the three, which is enough
/// to tell a picture from a `.EXE`, a `.BAT` or a database in the same
/// archive, and is the only thing this module refuses on.
pub const MAGIC: [Option<u8>; 5] = [Some(0xFD), Some(0x07), None, None, Some(0x6A)];

/// The fixed CGA palette family E draws through (§8.5), value 0 through 3.
///
/// **Not stored anywhere in the file.** §8.5: "palette 1 at high intensity,
/// with no intensity or background selection" — so unlike family C, where
/// every record carries four colour bytes and two of the *Hulk*'s are outside
/// the documented table, there is nothing here to fail to resolve.
pub const PALETTE: [Rgb; 4] =
    [(0, 0, 0), (0, 255, 255), (255, 0, 255), (255, 255, 255)];

/// The lowest file length that can hold a family-E header: the last field
/// [`decode_family_e`] reads is the width byte at `0x13`, and the compressed
/// data begins at `0x17` (§8.5).
const HEADER_LEN: usize = 0x17;

/// The CGA row pitch, in bytes, the raw start and end offsets of the header
/// are expressed in (§8.5: "the height is (raw end − raw start) ÷ 80").
const CGA_ROW_PITCH: usize = 80;

/// Does `record` look like a family-E picture file?
///
/// The three fixed bytes of [`MAGIC`], nothing more — a name is not a
/// classification, and a host walking a zip full of `.EXE`s, `.BAT`s and a
/// database needs to ask the CONTENT which entries are artwork. See [`MAGIC`]
/// for where the three bytes come from and why the other two are not tested.
pub fn looks_like_family_e(record: &[u8]) -> bool {
    record.len() >= HEADER_LEN
        && MAGIC
            .iter()
            .enumerate()
            .all(|(i, want)| want.is_none_or(|b| record[i] == b))
}

/// Decode one family-E `.PAK` file (spec §8.5).
///
/// `record` is the whole file, header included, exactly as the archive stores
/// it — there is no load address to skip and no trailer to trim; the trailing
/// slack every file carries (1 to 109 bytes, the DOS tool's padding to a
/// 128-byte boundary) is simply past the declared chunk and never read.
///
/// # The format, as implemented
///
/// **Header, at fixed file positions**, every word little-endian: `0x00`-`0x01`
/// and `0x04` the signature ([`MAGIC`]); `0x05`-`0x06` the size of the
/// compressed graphics chunk, which bounds decoding; `0x0D` the *lined* flag,
/// `0xFF` meaning unlined and anything else lined; `0x0F`-`0x10` the raw start
/// offset and `0x11`-`0x12` the raw end offset, both into a CGA bank;
/// `0x13` the width in 4-pixel units. The first compressed byte is at `0x17`.
///
/// **Derived placement.** The horizontal pixel offset is
/// `(raw start mod 80) × 4 − 24`; the vertical pixel offset is
/// `raw start ÷ 40` rounded down to an even number; the width in pixels is the
/// byte at `0x13` times four.
///
/// **Rows per pass** is `(raw end − raw start) ÷ 80` **plus one**, and that
/// `+ 1` is the one place this differs from §8.5's wording — see
/// [the note below](#the-height-limit-is-inclusive).
///
/// **Pixels.** Two bits each, four to a byte, most significant pair leftmost.
/// The *lined* flag is the pixel aspect and nothing else: unlined means each
/// stored pixel is two device pixels wide (so a byte paints eight positions,
/// and a 280-pixel row is 35 bytes), lined means one and one (a byte paints
/// four, and a 280-pixel row is 70). Both are used within one release — the
/// *Hulk* ships 36 unlined pictures and 32 lined ones.
///
/// **Storage order is row-major with the CGA two-bank interleave.** Bytes run
/// left to right along a row; when the horizontal position reaches the right
/// edge it resets and the vertical position advances by **two**. When a pass
/// has painted its rows, the vertical position resets to *vertical offset + 1*
/// and a second pass paints the odd rows. Decoding ends when the second pass
/// completes, when the chunk runs out, or when the canvas is left behind.
///
/// **Compression** is single-byte, not pairs: read a control byte; bit 7 set
/// means the count is the low seven bits **plus one** and one data byte
/// follows, emitted that many times; bit 7 clear means the count is the byte
/// **plus one** and that many data bytes follow, each emitted once.
///
/// # The height limit is INCLUSIVE
///
/// `(raw end − raw start) ÷ 80` is 78 for a full-canvas *Hulk* picture, and a
/// pass paints **79** rows, not 78 — so a picture is 158 device rows and the
/// stored end offset is the last row's own start rather than one past it
/// (`6316 − 70 = 6246 = 6 + 78 × 80`). Read the limit exclusively and the odd
/// pass begins one row's worth of data early, which leaves every even row
/// exactly right and every odd row wrong: the *Hulk*'s title screen still
/// decodes to a legible `QUESTPROBE`/`HULK` wordmark with the artwork torn
/// into horizontal streaks, so the picture alone cannot settle it. The
/// Commodore 64 twin of the same picture (family C, §8.3) can, and does: with
/// the inclusive reading, twenty-five of the *Hulk*'s pictures match their
/// family-C twins **pixel for pixel over all 280 × 158**, and with the
/// exclusive one none matches at all. This is the same correction §8.3 needed
/// in the other direction (Appendix A item 17).
///
/// # Colour
///
/// [`PALETTE`], fixed. The returned [`Picture`]'s `colour_bytes` are therefore
/// all zero and its `unrecognised_colours` always empty: family E stores no
/// colour bytes for those fields to report, and neither field means anything
/// here. (They are family C's, and [`Picture`] is one type so that a host can
/// draw either without a second code path.)
///
/// # Errors
///
/// [`PictureError`] — [`TooShort`](PictureError::TooShort) for a file that
/// cannot hold the header, [`NotFamilyE`](PictureError::NotFamilyE) for one
/// whose signature bytes say it is something else, and
/// [`EmptyRegion`](PictureError::EmptyRegion) for a header describing no
/// pixels at all. A declared chunk longer than the file is **clamped rather
/// than refused**; see the note beside the clamp.
pub fn decode_family_e(record: &[u8]) -> Result<Picture, PictureError> {
    if record.len() < HEADER_LEN {
        return Err(PictureError::TooShort { len: record.len() });
    }
    if !looks_like_family_e(record) {
        let mut magic = [0u8; 5];
        magic.copy_from_slice(&record[..5]);
        return Err(PictureError::NotFamilyE { magic });
    }
    let word = |at: usize| usize::from(u16::from_le_bytes([record[at], record[at + 1]]));
    let chunk = word(0x05);
    let lined = record[0x0D] != 0xFF;
    let raw_start = word(0x0F);
    let raw_end = word(0x11);
    let width = usize::from(record[0x13]) * 4;

    // The declared chunk is an upper BOUND on decoding, not a promise about
    // the file's length: *Fantastic Four*'s `R010.PAK` declares 4,597 bytes
    // and carries 4,585, and decodes to a complete picture from what is
    // there. Every file in both releases is padded to a 128-byte boundary and
    // most have slack the other way, so this is the tool's arithmetic rather
    // than damage — clamp, and let the row limit end the picture.
    let chunk = chunk.min(record.len() - HEADER_LEN);
    if width == 0 || raw_end < raw_start {
        // §8.5's fields are unsigned and a right edge left of the left edge is
        // spelled as an end offset below the start; report it in the same
        // shape §8.3's placement refusal uses, in pixels.
        return Err(PictureError::EmptyRegion {
            left: 0,
            right: width as i32,
            top: raw_start as i32,
            bottom: raw_end as i32,
        });
    }
    // §8.5's two derivations. The 24 is the canvas's own inset into the
    // 320-pixel CGA screen: a full-canvas picture starts at byte column 6,
    // which is pixel 24, and answers offset 0.
    let x_offset = (raw_start % CGA_ROW_PITCH) as i32 * 4 - 24;
    let y_offset = (raw_start / 40) as i32 & !1;
    // Inclusive; see "The height limit is INCLUSIVE" above.
    let rows_per_pass = (raw_end - raw_start) / CGA_ROW_PITCH + 1;
    let step = if lined { 1 } else { 2 };

    let mut pixels = vec![0u8; CANVAS_WIDTH * CANVAS_HEIGHT];
    let mut x = x_offset;
    let mut y = y_offset;
    let mut row = 0usize;
    let mut pass = 0usize;
    let mut done = false;
    // One stored byte: four pixels, most significant pair leftmost, each
    // `step` device pixels wide. Off-canvas pixels are dropped rather than
    // refused — *Fantastic Four*'s `B011.PAK` declares a right edge of 288 on
    // a 280-pixel canvas, and the *Hulk*'s tallest overlays end on the last
    // row a 158-row picture has.
    let mut emit = |x: &mut i32, y: &mut i32, row: &mut usize, pass: &mut usize, byte: u8| {
        for pair in 0..4i32 {
            let value = (byte >> (6 - 2 * pair)) & 3;
            for half in 0..step {
                let px = *x + half;
                if (0..CANVAS_WIDTH as i32).contains(&px)
                    && (0..CANVAS_HEIGHT as i32).contains(y)
                {
                    pixels[*y as usize * CANVAS_WIDTH + px as usize] = value;
                }
            }
            *x += step;
        }
        if *x < x_offset + width as i32 {
            return false;
        }
        *x = x_offset;
        *y += 2;
        *row += 1;
        if *row < rows_per_pass {
            return false;
        }
        *pass += 1;
        if *pass >= 2 {
            return true;
        }
        *y = y_offset + 1;
        *row = 0;
        false
    };

    let data = &record[HEADER_LEN..HEADER_LEN + chunk];
    let mut i = 0usize;
    while i < data.len() && !done {
        let control = data[i];
        i += 1;
        if control & 0x80 != 0 {
            let Some(&byte) = data.get(i) else { break };
            i += 1;
            for _ in 0..u16::from(control & 0x7f) + 1 {
                if emit(&mut x, &mut y, &mut row, &mut pass, byte) {
                    done = true;
                    break;
                }
            }
        } else {
            for _ in 0..u16::from(control) + 1 {
                let Some(&byte) = data.get(i) else { break };
                i += 1;
                if emit(&mut x, &mut y, &mut row, &mut pass, byte) {
                    done = true;
                    break;
                }
            }
        }
    }

    Ok(Picture {
        width: CANVAS_WIDTH,
        height: CANVAS_HEIGHT,
        pixels,
        palette: PALETTE,
        // Family E stores no colour bytes; see "Colour" above.
        colour_bytes: [0; 4],
        unrecognised_colours: Vec::new(),
    })
}

// ── Picture file names (§8.5, §8.6, §10.7) ───────────────────────────────────

/// Take an MS-DOS picture file's name apart (§8.5's naming rule as the two
/// known releases actually spell it), or `None` if it is not one.
///
/// The `.PAK` extension is optional and case-insensitive, and so are the
/// letters; what is left must be one of **two** conventions, which is the
/// whole of the difficulty here:
///
/// | release | room | object in room | object in inventory |
/// |---|---|---|---|
/// | *The Hulk* | `R01nn` | `B01nnnR` | `B01nnnI` |
/// | *Fantastic Four* | `Rnnn` / `Snnn` | `Bnnn` | — |
///
/// §8.5 describes the first as "`R01nn`, `B01nnR`, `B01nnI`, with a two-digit
/// index at name positions 3-4 for a room name and 4-5 for an object name".
/// The room half is right. The object half reads the correct index on this
/// release and only by luck: the *Hulk*'s object names carry **three** digits
/// after the `01`, of which the first is always `0` because no index reaches
/// 100 — so positions 4-5 and positions 3-5 give the same answer here and
/// would not on the Commodore 64 twin of the same set, which names its
/// `B01250R`. This function reads three digits, which is both readings where
/// they agree and the right one where they do not.
///
/// §10.7 records the second convention as *Fantastic Four*'s and warns that
/// §8.6's trailing-letter rule "mis-reads them": its `B` names carry no
/// trailing `R` or `I` at all, so every one of them is taken as an object
/// drawn in a room, and its twenty-one `S` names take §8.6's stated default of
/// a room picture ("a leading `S` carries no usage and defaults to a room
/// picture"). The two conventions cannot be confused for each other, because
/// they are different lengths: 5 or 7 characters against 4.
pub fn parse_picture_file_name(name: &str) -> Option<PictureFile> {
    let stem = match name.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("pak") => stem,
        _ => name,
    };
    let b = stem.as_bytes();
    let lead = b.first()?.to_ascii_uppercase();
    let digits = |from: usize, to: usize| -> Option<u16> {
        let s = std::str::from_utf8(b.get(from..to)?).ok()?;
        if !s.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    };
    match (lead, b.len()) {
        // *The Hulk*: `R01nn`, two digits after the series number.
        (b'R', 5) if &b[1..3] == b"01" => {
            Some(PictureFile { usage: PictureUsage::Room, index: digits(3, 5)? })
        }
        // *The Hulk*: `B01nnnR` / `B01nnnI`, three digits and a usage letter.
        (b'B', 7) if &b[1..3] == b"01" => {
            let usage = match b[6].to_ascii_uppercase() {
                b'R' => PictureUsage::ObjectInRoom,
                b'I' => PictureUsage::ObjectInInventory,
                _ => return None,
            };
            Some(PictureFile { usage, index: digits(3, 6)? })
        }
        // *Fantastic Four*: `Rnnn` / `Snnn` / `Bnnn`, three digits, no series
        // number and no usage letter (§10.7).
        (b'R' | b'S', 4) => Some(PictureFile { usage: PictureUsage::Room, index: digits(1, 4)? }),
        (b'B', 4) => {
            Some(PictureFile { usage: PictureUsage::ObjectInRoom, index: digits(1, 4)? })
        }
        _ => None,
    }
}

/// Is `name` an MS-DOS picture file name? [`parse_picture_file_name`] without
/// the parts, for a host walking a zip's entries.
///
/// **A name is not a classification** — pair it with [`looks_like_family_e`]
/// over the entry's bytes before believing it, exactly as a release floppy's
/// files are classified by content. Nothing stops an archive holding a
/// `README` an unlucky rule would match.
pub fn is_picture_file_name(name: &str) -> bool {
    parse_picture_file_name(name).is_some()
}

/// The file name a **room** picture with index `n` is stored under in the
/// MS-DOS *Hulk* release (§8.5) — `R01nn.PAK` — or `None` for an index no
/// two-digit field can spell.
///
/// The inverse of [`parse_picture_file_name`] for the room usage, and the
/// lookup a host needs once [`DosRelease::room_picture`] has given it a
/// picture index. Deliberately only the *Hulk*'s convention: it is the only
/// MS-DOS release whose database this crate reads (§10.7), so it is the only
/// one a running game ever asks this question about.
pub fn picture_file_name(n: usize) -> Option<String> {
    (n <= 99).then(|| format!("R01{n:02}.PAK"))
}

// ── Release identity (§10.7, §12.11) ─────────────────────────────────────────

/// One MS-DOS release this crate can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DosRelease {
    /// The box title, platform folded in the way
    /// [`SagaUs::display_title`](crate::SagaUs::display_title) folds it — the
    /// *same game* ships on the Commodore 64 as a S.A.G.A. binary database,
    /// and a story list showing both needs to tell them apart.
    pub title: &'static str,
    /// Does this release remap room pictures the way §12.11 says the *Hulk*
    /// does? See [`Self::room_picture`].
    pub remaps_hulk_rooms: bool,
}

impl DosRelease {
    /// The picture index a room's view is drawn from.
    ///
    /// §12.11's *Hulk* rule, reached through the one table that states it
    /// ([`hulk_room_picture`]) rather than a second copy: rooms 5 and 6 draw
    /// picture 3, 7 and 8 draw 4, 10 and 11 draw 9, 13 and 14 draw 2, and 17
    /// and 18 draw 16. Every other room draws its own number.
    ///
    /// **Checked against this release's own files, not taken on trust.** The
    /// MS-DOS *Hulk* ships `R0100` through `R0104`, `R0109`, `R0112`,
    /// `R0115`, `R0116`, `R0119` and `R0120` and no other room picture below
    /// 81 — so the ten rooms with no file of their own are exactly the ten
    /// this remaps, and the five targets are all present.
    pub fn room_picture(&self, room: usize) -> usize {
        if self.remaps_hulk_rooms {
            hulk_room_picture(room)
        } else {
            room
        }
    }
}

/// The MS-DOS *Hulk*, as §10.7's `ADVENT.DAT` describes it.
const HULK: DosRelease = DosRelease { title: "The Hulk (MS-DOS)", remaps_hulk_rooms: true };

/// Which MS-DOS release `db` is, or `None` for a database that is not one.
///
/// Identified by the **reference-format header** §10.7 prints in full —
/// "items 54, actions 261, words 128, rooms 20, max carried 10, start room 1,
/// treasures 17, word length 4, lamp 150, messages 99, treasure room 16" —
/// which is eleven numbers agreeing at once, and is the same shape of test
/// [`crate::zx_mysterious::identify_z80`] uses ("identified by the header
/// counts its own loader reads"). Not a checksum: the numbers are what §10.7
/// records and what anyone can re-read off the first twelve lines of the file,
/// where a checksum would be a magic constant nobody could check.
///
/// **Why a table exists at all.** The MS-DOS *Hulk*'s database is the plain
/// reference text format, so it carries no version, no adventure number the
/// text parser reads, and nothing else that names the game; without this it
/// appears in a story list under whatever the archive happens to be called.
/// It is one entry because one MS-DOS database is readable (§10.7:
/// *Fantastic Four*'s "is neither a memory image nor the reference text
/// format", and this document does not describe it).
pub fn identify(db: &Database) -> Option<DosRelease> {
    // **A database that identifies itself is not this.** The Commodore 64
    // release of the very same game decodes to the very same tables — same
    // items, same actions, same rooms, same counts — and answers this shape
    // exactly; it is §12's binary database and says so
    // ([`Database::saga_us`](crate::Database::saga_us)), which is a stronger
    // fact than eleven counts agreeing and must win. Without this the
    // Commodore disk's row is titled "The Hulk (MS-DOS)".
    if db.saga_us.is_some() {
        return None;
    }
    let shape = (
        db.items.len(),
        db.actions.len(),
        db.verbs.len(),
        db.rooms.len(),
        db.max_carry,
        db.start_room,
        db.num_treasures,
        db.word_length,
        db.light_time,
        db.messages.len(),
        db.treasure_room,
    );
    // Each table entry is a header count plus one, because the reference
    // format's counts are highest INDEX and every table carries an entry 0
    // (§2.2) — 54 items is items 0..=54.
    (shape == (55, 262, 129, 21, 10, 1, 17, 4, 150, 100, 16)).then_some(HULK)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A family-E file built by hand from §8.5, small enough that every pixel
    /// can be worked out on paper.
    ///
    /// Two device rows of eight pixels, unlined, so one stored byte covers a
    /// whole row: raw start 6 puts the left edge at `(6 mod 80) × 4 − 24` = 0
    /// and the top at `6 ÷ 40` = 0; raw end 6 makes `(6 − 6) ÷ 80 + 1` = one
    /// row per pass, and two passes paint rows 0 and 1; the width byte 2 is
    /// `2 × 4` = 8 pixels, which at two device pixels per stored pixel is one
    /// byte. So the chunk is two control-plus-data pairs, one row each.
    fn hand_built(lined: bool, rows_field: (u16, u16), width_units: u8, chunk: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8; HEADER_LEN];
        v[0] = 0xFD;
        v[1] = 0x07;
        v[4] = 0x6A;
        v[5..7].copy_from_slice(&(chunk.len() as u16).to_le_bytes());
        v[0x0D] = if lined { 0x00 } else { 0xFF };
        v[0x0F..0x11].copy_from_slice(&rows_field.0.to_le_bytes());
        v[0x11..0x13].copy_from_slice(&rows_field.1.to_le_bytes());
        v[0x13] = width_units;
        v.extend_from_slice(chunk);
        v
    }

    /// `0b00_01_10_11` is pixel values 0, 1, 2, 3 left to right (§8.5's "most
    /// significant pair leftmost"), and unlined draws each two device pixels
    /// wide — so one byte is `0 0 1 1 2 2 3 3`. Row 0 gets that byte and row 1
    /// gets `0b11_10_01_00`, its mirror, which is the interleave's second pass.
    #[test]
    fn two_bits_a_pixel_most_significant_pair_leftmost_unlined() {
        // Control 0x80 = "repeat the next byte once"; two of them, one per pass.
        let file = hand_built(false, (6, 6), 2, &[0x80, 0b00_01_10_11, 0x80, 0b11_10_01_00]);
        let pic = decode_family_e(&file).expect("a hand-built family-E file decodes");
        assert_eq!(pic.pixels[0..8], [0, 0, 1, 1, 2, 2, 3, 3], "row 0, the even pass");
        assert_eq!(
            pic.pixels[CANVAS_WIDTH..CANVAS_WIDTH + 8],
            [3, 3, 2, 2, 1, 1, 0, 0],
            "row 1, the odd pass"
        );
        assert!(
            pic.pixels[CANVAS_WIDTH * 2..].iter().all(|&v| v == 0),
            "nothing below the two rows the header declares"
        );
        assert_eq!(pic.palette, PALETTE, "the fixed CGA palette, black cyan magenta white");
        assert!(pic.unrecognised_colours.is_empty(), "family E stores no colour bytes");
    }

    /// The lined flag is the pixel ASPECT and nothing else: the same byte now
    /// covers four device pixels rather than eight, so the row is half as wide
    /// and the width byte has to be halved with it.
    #[test]
    fn lined_draws_one_device_pixel_per_stored_pixel() {
        let file = hand_built(true, (6, 6), 1, &[0x80, 0b00_01_10_11, 0x80, 0b11_10_01_00]);
        let pic = decode_family_e(&file).expect("decodes");
        assert_eq!(pic.pixels[0..4], [0, 1, 2, 3], "four pixels, one device pixel each");
        assert_eq!(pic.pixels[4..8], [0, 0, 0, 0], "and nothing past the four-pixel row");
        assert_eq!(pic.pixels[CANVAS_WIDTH..CANVAS_WIDTH + 4], [3, 2, 1, 0], "the odd pass");
    }

    /// §8.5's compression: bit 7 set is a repeat of ONE following byte, bit 7
    /// clear is that many literal bytes each emitted once — and both counts are
    /// the stored number plus one, so nothing encodes a zero-length run.
    #[test]
    fn the_two_compression_modes_are_repeat_one_byte_and_n_literals() {
        // Width 4 units is 16 device pixels, which unlined is TWO bytes a row.
        // The even pass is 0x81 = "the next byte twice"; the odd pass is
        // 0x01 = "two literals follow".
        let chunk = [0x81, 0b01_01_01_01, 0x01, 0b11_11_11_11, 0b10_10_10_10];
        let pic = decode_family_e(&hand_built(false, (6, 6), 4, &chunk)).expect("decodes");
        assert_eq!(pic.pixels[0..16], [1; 16], "the repeat filled the whole even row");
        assert_eq!(
            pic.pixels[CANVAS_WIDTH..CANVAS_WIDTH + 16],
            [3, 3, 3, 3, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2],
            "the two literals of the odd row, each byte eight device pixels wide"
        );
        assert!(
            pic.pixels[CANVAS_WIDTH * 2..].iter().all(|&v| v == 0),
            "and the second pass ended the picture"
        );
    }

    /// The row limit is INCLUSIVE (see [`decode_family_e`]): an end offset one
    /// CGA row past the start is TWO rows per pass, not one, so the picture is
    /// four device rows and not two.
    #[test]
    fn the_row_limit_is_inclusive_so_a_one_row_span_paints_two_rows_a_pass() {
        let chunk = [0x83, 0xFF]; // one byte, repeated four times: four rows.
        let pic = decode_family_e(&hand_built(false, (6, 86), 2, &chunk)).expect("decodes");
        for row in 0..4 {
            assert_eq!(
                &pic.pixels[row * CANVAS_WIDTH..row * CANVAS_WIDTH + 8],
                &[3u8; 8],
                "row {row} is painted"
            );
        }
        assert!(
            pic.pixels[4 * CANVAS_WIDTH..].iter().all(|&v| v == 0),
            "and the fifth row is not — (86 − 6) / 80 + 1 = 2 rows a pass, two passes"
        );
    }

    /// Placement: the header's raw start offset is where on the canvas the
    /// picture goes, and both derivations are §8.5's.
    #[test]
    fn the_raw_start_offset_places_the_picture_on_the_canvas() {
        // Byte column 26 of a CGA row is pixel 104, minus the 24-pixel inset
        // is x = 80; 1626 / 40 = 40 (already even), so y = 40.
        let pic = decode_family_e(&hand_built(false, (1626, 1626), 2, &[0x80, 0xFF]))
            .expect("decodes");
        assert_eq!(&pic.pixels[40 * CANVAS_WIDTH + 80..40 * CANVAS_WIDTH + 88], &[3u8; 8]);
        assert_eq!(pic.pixels[40 * CANVAS_WIDTH + 79], 0, "and nothing left of it");
    }

    /// An ODD raw start rounds the vertical offset DOWN to an even row (§8.5),
    /// because the even pass paints even rows by construction.
    #[test]
    fn an_odd_vertical_offset_rounds_down_to_even() {
        // 1666 / 40 = 41, which rounds down to 40; and 1666 mod 80 = 66, so
        // the left edge is 66 x 4 - 24 = 240.
        let pic = decode_family_e(&hand_built(false, (1666, 1666), 2, &[0x80, 0xFF]))
            .expect("decodes");
        assert_eq!(pic.pixels[40 * CANVAS_WIDTH + 240], 3, "row 40, not row 41");
        assert_eq!(pic.pixels[41 * CANVAS_WIDTH + 240], 0);
    }

    /// §11's "name it and refuse it", three ways.
    #[test]
    fn the_three_refusals_are_named_rather_than_guessed_at() {
        assert_eq!(decode_family_e(&[]), Err(PictureError::TooShort { len: 0 }));
        assert_eq!(
            decode_family_e(&[0u8; HEADER_LEN - 1]),
            Err(PictureError::TooShort { len: HEADER_LEN - 1 })
        );
        // A `.EXE` in the same archive: right length, wrong signature.
        let mut exe = vec![0u8; HEADER_LEN + 4];
        exe[0] = b'M';
        exe[1] = b'Z';
        assert_eq!(
            decode_family_e(&exe),
            Err(PictureError::NotFamilyE { magic: [b'M', b'Z', 0, 0, 0] })
        );
        // A header describing no pixels.
        assert!(matches!(
            decode_family_e(&hand_built(false, (6, 6), 0, &[0x80, 0xFF])),
            Err(PictureError::EmptyRegion { .. })
        ));
        assert!(matches!(
            decode_family_e(&hand_built(false, (600, 6), 2, &[0x80, 0xFF])),
            Err(PictureError::EmptyRegion { .. })
        ));
    }

    /// A chunk that stops mid-picture decodes what it has rather than failing:
    /// the declared chunk is honoured, so the rest of the canvas is simply
    /// value 0. (A chunk running PAST the file is the refusal above; this is a
    /// well-formed file whose artwork does not fill its own declared rows.)
    #[test]
    fn a_chunk_shorter_than_its_rows_leaves_the_rest_of_the_canvas_black() {
        let pic = decode_family_e(&hand_built(false, (6, 806), 2, &[0x80, 0xFF]))
            .expect("decodes");
        assert_eq!(&pic.pixels[0..8], &[3u8; 8], "the one row the chunk carries");
        assert!(pic.pixels[CANVAS_WIDTH..].iter().all(|&v| v == 0), "and nothing else");
    }

    /// Both naming conventions, and the fact that they cannot be confused.
    #[test]
    fn both_release_naming_conventions_are_read() {
        let room = |index| Some(PictureFile { usage: PictureUsage::Room, index });
        let in_room = |index| Some(PictureFile { usage: PictureUsage::ObjectInRoom, index });
        let carried =
            |index| Some(PictureFile { usage: PictureUsage::ObjectInInventory, index });

        // *The Hulk*: two-digit rooms, three-digit objects with a usage letter.
        assert_eq!(parse_picture_file_name("R0100.PAK"), room(0), "the darkness image");
        assert_eq!(parse_picture_file_name("R0199.PAK"), room(99), "the title screen");
        assert_eq!(parse_picture_file_name("R0112.PAK"), room(12));
        assert_eq!(parse_picture_file_name("B01039R.PAK"), in_room(39));
        assert_eq!(parse_picture_file_name("B01004I.PAK"), carried(4));
        // The extension is optional and the case is not load-bearing.
        assert_eq!(parse_picture_file_name("r0112"), room(12));
        assert_eq!(parse_picture_file_name("b01039r.pak"), in_room(39));

        // *Fantastic Four*: three digits, no series number, no usage letter.
        assert_eq!(parse_picture_file_name("R001.PAK"), room(1));
        assert_eq!(parse_picture_file_name("S000.PAK"), room(0), "§8.6's S default");
        assert_eq!(parse_picture_file_name("S020.PAK"), room(20));
        assert_eq!(parse_picture_file_name("B002.PAK"), in_room(2));

        // The OBJECT convention is shared with §8.3's Commodore 64 rule, three
        // digits and all, so the twin disk's `B01250R` reads here and reads
        // right — which is the point of reading three digits rather than
        // §8.5's stated two.
        assert_eq!(parse_picture_file_name("B01250R"), in_room(250));

        // Not picture names: the rest of either archive, and the Commodore 64
        // twin's ROOM names, which are the one convention that really does
        // differ (§8.3's `R01nnn` is six characters, not five).
        for name in [
            "ADVENT.DAT",
            "START.EXE",
            "HULK.BAT",
            "SPL53P.DAT",
            "FANTFOUR.TXT",
            "R01000",
            "",
            "R.PAK",
            "R010A.PAK",
            "B01039X.PAK",
        ] {
            assert_eq!(parse_picture_file_name(name), None, "{name} is not a DOS picture");
        }
    }

    /// [`picture_file_name`] is [`parse_picture_file_name`]'s inverse for the
    /// room usage, over every index a two-digit field can spell.
    #[test]
    fn picture_file_name_round_trips_every_room_index() {
        for n in 0..=99usize {
            let name = picture_file_name(n).expect("two digits spell it");
            assert_eq!(
                parse_picture_file_name(&name),
                Some(PictureFile { usage: PictureUsage::Room, index: n as u16 }),
                "{name}"
            );
        }
        assert_eq!(picture_file_name(100), None, "no two-digit field spells 100");
    }

    /// The remap is §12.11's, reached through the one table that states it.
    #[test]
    fn the_hulk_release_remaps_the_five_room_pairs_and_nothing_else() {
        for (room, want) in
            [(5, 3), (6, 3), (7, 4), (8, 4), (10, 9), (11, 9), (13, 2), (14, 2), (17, 16), (18, 16)]
        {
            assert_eq!(HULK.room_picture(room), want, "room {room}");
        }
        for room in [1usize, 2, 3, 4, 9, 12, 15, 16, 19, 20] {
            assert_eq!(HULK.room_picture(room), room, "room {room} draws its own picture");
        }
        let plain = DosRelease { title: "x", remaps_hulk_rooms: false };
        for room in 1..=20usize {
            assert_eq!(plain.room_picture(room), room, "a release with no remap");
        }
    }

    /// The Commodore 64 release of the SAME GAME decodes to the same eleven
    /// counts, so the header shape alone cannot tell the two apart — the
    /// §12 binary database's own record can, and does.
    ///
    /// Hand-built rather than read off `QUESTPR1.D64`: what is under test is
    /// the precedence, and a `Database` carrying a `saga_us` record is all it
    /// takes to state it. The real disk is the case in `picker.rs`
    /// (`ms_dos_questprobe_rows_name_the_release_its_pictures_and_its_zip`)
    /// and in `crates/app/tests/suites/saga_us_disks.rs`.
    #[test]
    fn a_database_that_identifies_itself_as_a_saga_binary_release_is_not_this() {
        // A minimal reference-format database, grown to the *Hulk*'s counts.
        // Every table is `pub` and the shape is all this reads, so the
        // contents do not matter.
        let text = concat!(
            "0 0 0 0 0 0 0 0 3 0 0 0\n",
            "0 0 0 0 0 0 0 0\n",
            "\"NORTH\" \"NORTH\"\n",
            "0 0 0 0 0 0 \"A room.\"\n",
            "\"A message.\"\n",
            "\"An item.\" 0\n",
        );
        let mut db = Database::parse(text.as_bytes()).expect("the minimal database parses");
        db.items.resize(55, db.items[0].clone());
        db.actions.resize(262, db.actions[0].clone());
        db.verbs.resize(129, String::new());
        db.rooms.resize(21, db.rooms[0].clone());
        db.messages.resize(100, String::new());
        db.max_carry = 10;
        db.start_room = 1;
        db.num_treasures = 17;
        db.word_length = 4;
        db.light_time = 150;
        db.treasure_room = 16;
        assert_eq!(
            identify(&db).map(|r| r.title),
            Some("The Hulk (MS-DOS)"),
            "the reference-format encoding is the one this table is for"
        );
        db.saga_us = Some(crate::SagaUs {
            version: 127,
            adventure: 1,
            platform: crate::SagaPlatform::Commodore64,
        });
        assert_eq!(identify(&db), None, "the Commodore 64 release titles itself");
    }

    /// [`looks_like_family_e`] tests three bytes and ignores the two that vary
    /// between the two releases.
    #[test]
    fn the_signature_is_three_bytes_and_the_release_stamp_is_not_one_of_them() {
        let mut hulk = vec![0u8; HEADER_LEN];
        hulk[0] = 0xFD;
        hulk[1] = 0x07;
        hulk[2] = 0x19;
        hulk[3] = 0x08;
        hulk[4] = 0x6A;
        assert!(looks_like_family_e(&hulk));
        let mut ff = hulk.clone();
        ff[2] = 0x29;
        ff[3] = 0x04;
        assert!(looks_like_family_e(&ff), "a different release stamp is still family E");
        for at in [0usize, 1, 4] {
            let mut wrong = hulk.clone();
            wrong[at] ^= 0xFF;
            assert!(!looks_like_family_e(&wrong), "byte {at} is part of the signature");
        }
        assert!(!looks_like_family_e(&hulk[..HEADER_LEN - 1]), "too short to have a header");
    }
}
