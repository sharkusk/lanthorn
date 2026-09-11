//! Amiga bitmap fonts, as Infocom shipped them on the release floppies (SQ-0916).
//!
//! Four releases carry one. It is an ordinary AmigaDOS **disk font**, which looks
//! like an executable and is not: the file opens `$000003F3` (`HUNK_HEADER`) and its
//! single CODE hunk begins `70 00 4E 75` — `moveq #0,d0 / rts`, the stub that makes
//! running one as a program a harmless no-op. The font proper starts behind it.
//!
//! | release | file | geometry |
//! |---|---|---|
//! | *Arthur* | `char.data` | 10×10 nominal, **proportional** (widths 2–8), chars 32–127 |
//! | *Journey* | `Char.data` | 8×8 fixed, chars 32–255 — Z-machine font 3, not letters |
//! | *Beyond Zork* | `Graphic.Data` | byte-identical to Journey's, under another name |
//! | *Shogun*, *Zork Zero* | — | none; they take the system topaz |
//!
//! # Layout
//!
//! From `<diskfont/diskfont.h>` and `<graphics/text.h>`: the four-byte stub, then
//! `DiskFontHeader` — a 14-byte `Node`, `dfh_FileID` (`0x0F80`, the signature this
//! parser checks), `dfh_Revision`, `dfh_Segment`, and a 32-byte name — then
//! `TextFont`, whose own 20-byte `Message` precedes `tf_YSize`. Glyph rows are one
//! long bitmap per row, `tf_Modulo` bytes wide, all glyphs side by side;
//! `tf_CharLoc` is an array of `(bit offset, bit width)` pairs, one per code.
//!
//! # The bitmap width is NOT the advance (SQ-1009)
//!
//! `tf_CharLoc` says how many bits of the strike belong to a glyph. What the PEN
//! does is two other arrays: `tf_CharKern` is added BEFORE the glyph is drawn (the
//! left side bearing) and `tf_CharSpace` after it, so one character advances by
//! `kern + space` and the ink sits `kern` in from the pen. Reading the strike width
//! as the advance is wrong for every glyph in a proportional face and catastrophic
//! for the SPACE, whose strike is **zero bits wide** — draw with it and the words
//! run together, which is exactly what happened the first time this face was
//! rasterised.
//!
//! Arthur's own table settles a question three notes on SQ-1009 could not: summed
//! over `THE CHURCHYARD` it gives **80 px**, against the 83-px highlight box
//! measured on `machine-screenshots/amiga-arthur-hint.png`, and 4.70 px/char on
//! prose against the 4.5 measured in `machine-screenshots/info.txt`. Both agree at
//! **1:1** and are off by a factor of two at 2:1 — so those captures are native
//! 320x200 frames, independently of the palette argument.
//!
//! A fixed-pitch face carries neither array (both pointers are zero on Journey's
//! and Beyond Zork's font-3 sets), and there the advance is `tf_XSize`.
//!
//! `dfh_Name` is EMPTY on all of these, which is why they are files beside the game
//! rather than entries in `FONTS:` — the interpreter loads the segment directly
//! instead of asking `diskfont.library` for a font by name.
//!
//! # Bit order
//!
//! Rows come back **MSB-leftmost**, exactly as the disk stores them, so a row can be
//! read against a hex dump without mental arithmetic — one byte per row for a glyph
//! up to 8px wide (bearing included), more for a wider one; see
//! [`crate::bitmap_font::Glyph::row_bytes`] and [`crate::bitmap_font::row_bit`]
//! (SQ-1038). `app`'s renderer flips a DIFFERENT, unrelated font (`font8x8`, which
//! numbers columns from the low bit) with [`u8::reverse_bits`] — that trick only
//! applies to a single byte, so it is not the right tool for a row of these.

use crate::bitmap_font::{BitmapFont, Glyph};

/// `dfh_FileID`, the signature that separates a disk font from any other hunk file.
const DFH_ID: u16 = 0x0F80;
/// `FPF_PROPORTIONAL` in `tf_Flags`.
const FPF_PROPORTIONAL: u8 = 0x20;
/// Offset of `TextFont` from the start of the CODE hunk: the 4-byte stub, then
/// `DiskFontHeader` up to and including its 32-byte `dfh_Name`.
const TF: usize = 0x3A;

/// Parse a disk font, or `None` when the bytes are not one.
///
/// Every field is bounds-checked against the buffer and the geometry has to
/// agree with itself, so this can be pointed at any file on a volume — which is
/// how it is used, since the name varies per release and `Graphic.Data` sounds
/// like artwork.
pub fn parse(raw: &[u8]) -> Option<BitmapFont> {
    let be16 = |o: usize| -> Option<u16> {
        Some(u16::from_be_bytes([*raw.get(o)?, *raw.get(o + 1)?]))
    };
    let be32 = |o: usize| -> Option<u32> {
        Some(u32::from_be_bytes([
            *raw.get(o)?,
            *raw.get(o + 1)?,
            *raw.get(o + 2)?,
            *raw.get(o + 3)?,
        ]))
    };
    // HUNK_HEADER, then an EMPTY resident-library list — a font has none, and
    // requiring that is what keeps this off ordinary executables cheaply.
    if be32(0)? != 0x0000_03F3 || be32(4)? != 0 {
        return None;
    }
    // table size, first, last, one length per hunk, then HUNK_CODE and its size.
    let hunks = usize::try_from(be32(16)?.checked_sub(be32(12)?)?.checked_add(1)?).ok()?;
    let body = 20usize.checked_add(hunks.checked_mul(4)?)?.checked_add(8)?;

    if be16(body.checked_add(0x12)?)? != DFH_ID {
        return None;
    }
    // A hunk file's pointers are stored as offsets from the hunk's own start and
    // relocated at load time, so "dereference" here is one addition.
    text_font_at(raw, body.checked_add(TF)?, &|p| body.checked_add(p))
}

/// One `TextFont` struct at `tf`, wherever it came from and however its pointers
/// are spelled.
///
/// # The disk font and the ROM font are the SAME struct
///
/// `<graphics/text.h>`'s `TextFont` is fifty-two bytes and identical in a
/// `FONTS:` file, in a game's `char.data`, and in Kickstart ROM. The only thing
/// that differs is what `tf_CharData`, `tf_CharLoc`, `tf_CharSpace` and
/// `tf_CharKern` MEAN: a hunk file stores them as offsets from the hunk it will
/// be loaded at, a ROM image stores them already absolute in the address space
/// it is mapped into. So `deref` — pointer value in, offset into `raw` out — is
/// the whole difference, and there is exactly one parser (SQ-1053). SQ-1011
/// shipped inert TWICE over a rule that existed in two places; a second copy of
/// this would be that defect one layer down.
fn text_font_at(
    raw: &[u8],
    tf: usize,
    deref: &dyn Fn(usize) -> Option<usize>,
) -> Option<BitmapFont> {
    let be16 = |o: usize| -> Option<u16> {
        Some(u16::from_be_bytes([*raw.get(o)?, *raw.get(o + 1)?]))
    };
    let be32 = |o: usize| -> Option<u32> {
        Some(u32::from_be_bytes([
            *raw.get(o)?,
            *raw.get(o + 1)?,
            *raw.get(o + 2)?,
            *raw.get(o + 3)?,
        ]))
    };
    // A field of the struct itself is at a fixed offset from `tf`; only the four
    // POINTERS below go through `deref`.
    let field = |o: usize| tf.checked_add(o);

    let height = be16(field(20)?)?;
    let flags = *raw.get(field(23)?)?;
    let width = be16(field(24)?)?;
    let baseline = be16(field(26)?)?;
    // `tf_BoldSmear`, between the baseline and `tf_Accessors` — how far the face
    // smears (and how much wider it advances) when a run asks for bold (SQ-1009).
    let bold_smear = be16(field(28)?)?;
    let (lo, hi) = (*raw.get(field(32)?)?, *raw.get(field(33)?)?);
    let chardata = usize::try_from(be32(field(34)?)?).ok()?;
    let modulo = usize::from(be16(field(38)?)?);
    let charloc = usize::try_from(be32(field(40)?)?).ok()?;
    // Zero means "this font has no such array", which is how a fixed-pitch face is
    // stored — not an offset of zero.
    let charspace = usize::try_from(be32(field(44)?)?).ok().filter(|&p| p != 0);
    let charkern = usize::try_from(be32(field(48)?)?).ok().filter(|&p| p != 0);
    // Bounds that only a decoding error could exceed. NOTE the nominal width is
    // NOT capped at 8: Arthur's font is 10 wide nominally while every glyph in
    // it is 8 or narrower, and capping here rejected it outright. The width that
    // has to fit a byte is the per-GLYPH one, checked below.
    if hi < lo || height == 0 || height > 32 || width == 0 || width > 32 || modulo == 0 {
        return None;
    }

    let mut glyphs = Vec::with_capacity(usize::from(hi - lo) + 1);
    for i in 0..=usize::from(hi - lo) {
        let entry = charloc.checked_add(i.checked_mul(4)?)?;
        let off = usize::from(be16(deref(entry)?)?);
        let gw = usize::from(be16(deref(entry + 2)?)?);
        // A signed word each, and both default to the fixed-pitch behaviour when
        // the font omits them: no bearing, and one nominal cell of advance.
        let word = |p: usize| -> Option<i16> {
            Some(be16(deref(p.checked_add(i.checked_mul(2)?)?)?)? as i16)
        };
        let kern = match charkern {
            Some(p) => word(p)?,
            None => 0,
        };
        let space = match charspace {
            Some(p) => word(p)?,
            None => i16::try_from(width).ok()?,
        };
        // The bearing is carried in the BITMAP, the way `mac_font` carries the
        // Macintosh's, so a consumer needs only the advance and the rows. A
        // negative bearing cannot be represented that way and is dropped rather
        // than clipping the glyph's left edge; none of the shipped faces has one.
        let bearing = usize::try_from(kern.max(0)).ok()?;
        // The row has to hold bearing + ink, not just ink (SQ-1038) — this used to
        // cap at 8 and silently DROP the bearing past it rather than decline, which
        // flushed a glyph left instead of refusing it; now it refuses, the same
        // discipline `mac_font` applies.
        let span = bearing.checked_add(gw)?;
        if span > crate::bitmap_font::MAX_ROW_WIDTH {
            return None;
        }
        let row_bytes = span.div_ceil(8).max(1);
        let mut rows = Vec::with_capacity(usize::from(height) * row_bytes);
        for y in 0..usize::from(height) {
            let base = chardata.checked_add(y.checked_mul(modulo)?)?;
            let mut row = vec![0u8; row_bytes];
            for x in 0..gw {
                let bit = off.checked_add(x)?;
                if *raw.get(deref(base.checked_add(bit / 8)?)?)? & (0x80 >> (bit % 8)) != 0 {
                    let col = x + bearing;
                    row[col / 8] |= 0x80 >> (col % 8);
                }
            }
            rows.extend_from_slice(&row);
        }
        glyphs.push(Glyph { width: u8::try_from(kern.saturating_add(space).max(0)).ok()?, rows });
    }
Some(BitmapFont {
        width: u8::try_from(width).ok()?,
        height: u8::try_from(height).ok()?,
        baseline: u8::try_from(baseline).ok()?,
        bold_smear: u8::try_from(bold_smear).unwrap_or(0),
        proportional: flags & FPF_PROPORTIONAL != 0
            || BitmapFont::measure_proportional(&glyphs),
        lo,
        glyphs,
    })
}

// ── Kickstart ROM ───────────────────────────────────────────────────────────

/// `FPF_ROMFONT` in `tf_Flags` — "this font is in ROM". Set on both faces in
/// Kickstart 1.2 and on nothing that came off a disk, so it is the cheap first
/// question the scan below asks of every offset.
const FPF_ROMFONT: u8 = 0x01;
/// `sizeof(struct TextFont)`.
const TEXT_FONT_LEN: usize = 52;
/// The 68000 address one past the last byte of a Kickstart ROM. Every size of
/// Kickstart is mapped so that it ENDS here — 256 KiB at `$FC0000`, 512 KiB at
/// `$F80000` — which is what makes the base derivable from the image's length
/// instead of pinned per revision.
const ROM_TOP: u32 = 0x0100_0000;
/// The image lengths a Kickstart dump comes in. Stated as a list rather than
/// "any power of two" so a stray file under `~/.lanthorn/` cannot be scanned as
/// a ROM by accident, and so [`rom_base`] never invents an address for a length
/// no Amiga ever mapped.
const ROM_SIZES: [usize; 3] = [256 * 1024, 512 * 1024, 1024 * 1024];
/// How many faces one ROM may yield. Kickstart 1.2 has two; the cap is what keeps
/// a hostile image from turning a structural coincidence into unbounded work.
const MAX_ROM_FACES: usize = 16;

/// The 68000 address a Kickstart image of `len` bytes is mapped at, or `None`
/// for a length no Kickstart comes in.
///
/// This is the whole reason a revision number is never needed: 1.2/1.3 are 256
/// KiB at `$FC0000` and 2.0+ are 512 KiB at `$F80000`, and both are simply
/// `ROM_TOP` minus their own size.
pub fn rom_base(len: usize) -> Option<u32> {
    ROM_SIZES.contains(&len).then(|| ROM_TOP - len as u32)
}

/// Every typeface a Kickstart ROM image carries, named `<face>/<size>`.
///
/// # Why a ROM at all
///
/// The Amiga's Version 6 interpreter drew prose in **topaz 8**, and topaz 8 is
/// on no floppy: a Workbench disk's `FONTS:` drawer carries `topaz/11` and six
/// proportional display faces, and the 8x8 the machine actually painted with
/// lives in Kickstart. So the machine's own face is recoverable only from a ROM
/// the player supplies, exactly as Geneva is recoverable only from a Macintosh
/// System file (SQ-1037, SQ-1053).
///
/// # How a face is FOUND, since the address moves
///
/// Nothing here is pinned to a revision. The image is identified as a Kickstart
/// by its length (see [`rom_base`]) and by the `JMP` at offset 2 — `$4EF9`
/// followed by a long address that lands inside the ROM's own mapped range,
/// which is the first instruction every Kickstart begins with. Then every even
/// offset is tested for a `TextFont`-SHAPED record:
///
/// * `tf_Flags` carries `FPF_ROMFONT`;
/// * `tf_YSize` and `tf_XSize` are sane and `0 < tf_Baseline < tf_YSize`;
/// * `tf_LoChar <= tf_HiChar`, `tf_Modulo` is non-zero;
/// * `tf_CharData` and `tf_CharLoc` are addresses whose whole arrays lie inside
///   the image;
/// * and one word of the 20-byte `tf_Message` preamble resolves to a
///   NUL-terminated `<name>.font` string inside the image.
///
/// That last one is both the NAME and the strongest filter. A ROM image does not
/// initialise the preamble's link fields — they are fixed up at boot — so the
/// slot the name sits in is not something to assume, and it is looked for rather
/// than indexed. On `Kick12.rom` (262,144 bytes, base `$FC0000`) the whole test
/// yields exactly two records and no false positives: `topaz/8` (8x8, the face
/// the interpreter drew with) and `topaz/9` (10x9), both naming `topaz.font`.
///
/// Nothing here CHOOSES. Which face a machine wants is one question asked in one
/// place, the host's `native_font::fit` and the cascade around it, and the
/// `<face>/<size>` spelling is exactly what [`drawer_of`] already reads off a
/// `FONTS:` path so a ROM face and a disk face are ranked by the same rule.
pub fn faces_in_rom(raw: &[u8]) -> Vec<(String, BitmapFont)> {
    let Some(base) = rom_base(raw.len()).map(|b| b as usize) else { return Vec::new() };
    let be16 = |o: usize| -> Option<u16> {
        Some(u16::from_be_bytes([*raw.get(o)?, *raw.get(o + 1)?]))
    };
    let be32 = |o: usize| -> Option<u32> {
        Some(u32::from_be_bytes([
            *raw.get(o)?,
            *raw.get(o + 1)?,
            *raw.get(o + 2)?,
            *raw.get(o + 3)?,
        ]))
    };
    // An address inside the mapped ROM, as an offset into `raw`.
    let deref = |p: usize| p.checked_sub(base).filter(|&o| o < raw.len());
    // Every Kickstart opens with `JMP <somewhere in ROM>`. Two bytes of opcode and
    // an in-range target is a weak claim on its own and a decisive one alongside
    // the length rule, and it is what keeps this scan off a file that merely
    // happens to be 256 KiB.
    if be16(2) != Some(0x4EF9) || be32(4).map(|a| a as usize).and_then(deref).is_none() {
        return Vec::new();
    }

    // A NUL-terminated printable `<stem>.font` at `p`, as the stem alone.
    let font_name = |p: usize| -> Option<String> {
        let start = deref(p)?;
        let end = raw[start..].iter().position(|&b| b == 0)? + start;
        let s = std::str::from_utf8(&raw[start..end]).ok()?;
        if s.len() > 32 || !s.bytes().all(|b| (0x20..0x7f).contains(&b)) {
            return None;
        }
        let stem = s.strip_suffix(".font")?;
        (!stem.is_empty() && !stem.contains('/')).then(|| stem.to_string())
    };

    let mut out: Vec<(String, BitmapFont)> = Vec::new();
    let mut tf = 0usize;
    while tf + TEXT_FONT_LEN <= raw.len() && out.len() < MAX_ROM_FACES {
        let here = tf;
        tf += 2;
        if raw[here + 23] & FPF_ROMFONT == 0 {
            continue;
        }
        let (Some(ysize), Some(xsize), Some(baseline)) =
            (be16(here + 20), be16(here + 24), be16(here + 26))
        else {
            continue;
        };
        if ysize == 0 || ysize > 32 || xsize == 0 || xsize > 32 || baseline == 0 || baseline >= ysize
        {
            continue;
        }
        let (lo, hi) = (raw[here + 32], raw[here + 33]);
        let modulo = usize::from(be16(here + 38).unwrap_or(0));
        if lo > hi || modulo == 0 {
            continue;
        }
        // Both tables whole, not merely their first byte: a record whose strike
        // runs off the end of the image is not a font, and refusing it here keeps
        // the parser from walking a plausible-looking coincidence.
        let spans = |p: Option<u32>, len: usize| {
            p.map(|p| p as usize)
                .and_then(deref)
                .and_then(|o| o.checked_add(len))
                .is_some_and(|end| end <= raw.len())
        };
        if !spans(be32(here + 34), usize::from(ysize) * modulo)
            || !spans(be32(here + 40), 4 * (usize::from(hi - lo) + 1))
        {
            continue;
        }
        let Some(name) = (0..=16)
            .step_by(2)
            .filter_map(|o| be32(here + o))
            .find_map(|p| font_name(p as usize))
        else {
            continue;
        };
        if let Some(font) = text_font_at(raw, here, &deref) {
            out.push((format!("{name}/{ysize}"), font));
        }
    }
    out
}

/// The font a mounted volume carries, if it carries one.
///
/// `files` is whatever the medium reported, as `(path, bytes)` — the same door
/// [`crate::infocom_sound::from_volume`] takes. **Nothing is matched on the
/// filename**: the two names in use are `char.data` and `Char.data`, one release
/// calls the identical file `Graphic.Data`, and case varies by volume, so every file
/// is offered to [`parse`] and the signature decides. A volume with no
/// font costs one failed signature check per file.
///
/// When more than one parses, the widest-covering wins, so a text font outranks the
/// 95-glyph font-3 set on a hypothetical disk carrying both.
pub fn from_volume<'a, I>(files: I) -> Option<BitmapFont>
where
    I: IntoIterator<Item = (&'a str, &'a [u8])>,
{
    files
        .into_iter()
        .filter_map(|(_, bytes)| parse(bytes))
        .max_by_key(|f| f.glyphs.len())
}

/// The `FONTS:` drawer an AmigaDOS font path names — `topaz` out of
/// `fonts/topaz/11`, whatever the case and whatever leads up to it.
///
/// A disk font is `<dir>/<name>/<size>`, and the DRAWER is the typeface's real
/// name: the size is a separate directory entry and `dfh_Name` is empty on every
/// Infocom face measured (see the module docs), so a caller looking for a face by
/// name has nothing else to look at. `None` for a path with no such shape, which
/// is every loose `char.data` beside a game.
///
/// Matched case-insensitively by the caller, since a volume's own case varies —
/// this returns the drawer exactly as the path spells it.
pub fn drawer_of(path: &str) -> Option<&str> {
    let (parent, size) = path.rsplit_once('/')?;
    if size.is_empty() || !size.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let drawer = parent.rsplit('/').next().unwrap_or(parent);
    (!drawer.is_empty()).then_some(drawer)
}

/// Every font a volume carries, each with the file it was found in.
///
/// [`from_volume`]'s companion: that one CHOOSES, this one REPORTS, for a
/// surface showing a person what a medium holds. The name is kept because it is
/// the only handle a reader has on an Amiga face — nothing is matched on it (see
/// above), so it is a label rather than a key.
pub fn faces_in_volume<'a, I>(files: I) -> Vec<(String, BitmapFont)>
where
    I: IntoIterator<Item = (&'a str, &'a [u8])>,
{
    files.into_iter().filter_map(|(name, bytes)| parse(bytes).map(|f| (name.to_string(), f))).collect()
}

#[cfg(test)]
mod drawer_tests {
    /// `fonts/<name>/<size>` — the drawer is the typeface's real name (SQ-1037).
    ///
    /// It matters because a Workbench floppy carries eight of them and a machine
    /// wants exactly one. Take "whatever parses" off that disk and an Amiga game
    /// is drawn in `ruby`; the drawer name is the only thing that says otherwise,
    /// since `dfh_Name` is empty on every face measured (see the module docs).
    #[test]
    fn a_disk_font_path_names_its_drawer() {
        for (path, want) in [
            ("fonts/topaz/11", Some("topaz")),
            ("FONTS/Topaz/8", Some("Topaz")),
            ("Workbench/fonts/garnet/16", Some("garnet")),
            ("fonts/sapphire/19", Some("sapphire")),
        ] {
            assert_eq!(super::drawer_of(path), want, "{path}");
        }
        // Anything that is not `<drawer>/<digits>` names no drawer, which is every
        // loose font file beside a game — `char.data` is matched by SIGNATURE and
        // never by name, and must not be mistaken for a system face.
        for path in ["char.data", "Char.data", "fonts/topaz", "fonts/topaz/eleven", "11", ""] {
            assert_eq!(super::drawer_of(path), None, "{path:?} names no drawer");
        }
    }
}
