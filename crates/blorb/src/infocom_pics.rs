//! Reader for Infocom's *native* picture archives — the `Pic.data` file shipped
//! on the Amiga and Macintosh releases of the graphical version-6 games, and the
//! `.MG1`/`.EG1`/`.EG2`/`.CG1` files shipped on the PC releases of the same
//! games.
//!
//! # One container, three codecs
//!
//! Every flavour shares a 16-byte header and a directory of fixed-size records;
//! Infocom's own interpreter sources spell that layout out field by field
//! (`amiga/gfx.c`, `mac/gfx.p` and `apple/yzip/rel.15/zip.equ`, published at
//! <https://github.com/erkyrath/infocom-zcode-terps>). What differs is byte
//! order — each platform wrote its own — and how the pixels are packed.
//!
//! **Amiga and Macintosh** are big-endian and Huffman-coded. `mac/gfx.c` opens
//! with a prose specification of that codec:
//!
//! ```text
//! Steps in picture compression (undo in reverse order):
//!
//! 1.  Each line of the picture is exclusive-or'ed with the previous line.
//!
//! 2.  A run-length encoding is applied, as follows:  byte values 0
//! through 15 represent colors; byte values 16 through 127 are repetition
//! counts (16 will never actually appear)  Thus: 3 occurrences of byte
//! value 2 will turn into 2 17 (subtract 15 from the 17 to find that the
//! two should be repeated 2 MORE times).
//!
//! 3.  Optionally, the whole thing is Huffman-coded, using an encoding
//! specified in the header file.
//! ```
//!
//! That one codec covers the Macintosh's **monochrome** `Pic.data` as well as
//! its colour `CPic.data` and every Amiga archive: two-colour art runs the same
//! three stages and lands one byte per pixel, using colour numbers 2 and 3 (and
//! 0 for transparent) instead of all sixteen. See [`InfocomPics::decode`] and
//! [`CGA_PALETTE`]; SQ-0838.
//!
//! **PC** archives are little-endian and carry no Huffman tree at all. Each
//! picture's data offset points at a bare GIF-style LZW stream — minimum code
//! size 8, so clear is 256 and end-of-stream 257, codes packed least-significant
//! bit first and growing from 9 to 12 bits — that expands straight to finished
//! pixels. There is **no** run-length stage and **no** per-line XOR: the LZW
//! output *is* the picture. See [`InfocomPics::parse`] for how the two are told
//! apart, and [`Flavour`] for what the published sources do and do not settle.
//!
//! **Apple II** is big-endian like the Amiga, spends 8 bytes on a record where
//! the others spend 12 to 16, and runs the Amiga's codec with its third stage
//! switched off: run-length and per-line XOR, no Huffman. It is the only
//! flavour whose every part — container, codec and colour table — comes out of
//! Infocom's own published sources rather than out of a second implementation:
//! `apple/yzip/rel.15/zip.equ` for the header and the record,
//! `apple/yzip/rel.15/pic.asm` for the decoder, and
//! `apple/yzip/apl2iff/apple.pal` for the sixteen colours. Its archives are not
//! files — each lives in the free tail of a packed volume's segment — so it
//! parses through [`InfocomPics::parse_apple`], which takes the segment and the
//! offset the volume's own index gives for it. SQ-0863.
//!
//! There is no magic number, and the header carries no release number and no
//! serial — nothing that could tie an archive to a story. So a caller must
//! decide by other means that a given file belongs to the story it is about to
//! draw, and lanthorn deliberately does NOT decide it from the filename: every
//! Infocom Amiga release names its archive `Pic.data`, and the PC names are a
//! DOS 8.3 convention that a renamed story file no longer matches. The pairing
//! comes from the medium (both files off one disk image) or from the user
//! naming the archive outright. See `app::graphics::PictureOverride` and
//! SQ-0734.

/// An 8-bit RGB triple.
pub type Rgb = [u8; 3];

/// Which platform wrote an archive, and therefore how to read it.
///
/// # What the published sources settle, and what they do not
///
/// `github.com/erkyrath/infocom-zcode-terps` carries graphics code for the
/// Amiga (`amiga/gfx.c`), the Macintosh (`mac/gfx.c`, `mac/gfx.p`) and the
/// Apple II (`apple/yzip/rel.15/zip.equ`, `pic.asm`) — and **no PC/DOS
/// interpreter at all**. So the *container* below is authoritative: those three
/// agree field for field on the 16-byte header and on the directory record, and
/// the PC files match it exactly once read little-endian.
///
/// The PC *pixel* codec is not in that repository. Two other things stand in
/// for it, and they agree.
///
/// The first is the format itself: the PC archives use GIF89a's own
/// variable-width LZW codec, unmodified — see `lzw_expand` for the constants
/// and their citation to the GIF specification. Stefan Jokisch's
/// `src/dos/bcpic.c` in Frotz, the DOS front end that draws these very files,
/// is a second, independent implementation of that same spec, and it agrees.
///
/// The second is an oracle, because a second implementation is still not the
/// format. Zork Zero's MCGA archive and its Amiga `Pic.data` hold the same
/// artwork on the same 12-bit palettes, and for all 383 pictures whose two
/// directories agree on dimensions the LZW output is **byte-for-byte identical**
/// to what the (source-verified) Huffman path produces. Two unrelated codecs
/// cannot agree on 383 images by accident, and that is also what proves there is
/// no XOR stage on the PC side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
    /// Amiga and Macintosh `Pic.data`: big-endian, Huffman + RLE + per-line XOR.
    AmigaMac,
    /// The PC archives — `.MG1` (MCGA), `.EG1`/`.EG2` (EGA), `.CG1` (CGA):
    /// little-endian, GIF-style LZW, no XOR.
    Pc,
    /// The Apple II archives, carried inside the `.D2`…`.D5` segments of a
    /// packed volume: big-endian, 8-byte records, and the Amiga's RLE +
    /// per-line XOR **without** the Huffman stage. See
    /// [`InfocomPics::parse_apple`] and `InfocomPics::decode_apple`.
    ///
    /// Unlike the other two this flavour is authoritative from Infocom's own
    /// sources end to end: `apple/yzip/rel.15/zip.equ` gives every field of the
    /// header and of the record, and `pic.asm` is the decoder itself.
    Apple,
}

/// Errors that can arise while reading a native Infocom picture archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PicError {
    /// A length or offset ran past the end of the data.
    Truncated,
    /// The header does not describe a picture archive this module reads.
    UnsupportedContainer,
    /// No picture with that id is in the directory.
    NoSuchPicture,
    /// The entry is a size-only placeholder: dimensions but no pixel data.
    /// Blorb conversions render these as `Rect` resources.
    NoPixelData,
    /// The entry uses a compression variant this module does not decode: on the
    /// Amiga/Mac side raw un-Huffman'd RLE, and on either side an embedded IFF
    /// (`EF_IFF`).
    UnsupportedCompression(u16),
    /// The compressed stream did not expand to `width * height` pixels.
    BadPixelData,
    /// A file offered as the next part of a multi-part set does not continue it:
    /// the wrong part number in its header, a different codec, or nothing the
    /// set does not already hold. Never merged blindly (SQ-0798).
    NotAContinuation(&'static str),
}

/// A one-clause reason, for a host that has to tell a user why the archive they
/// named cannot be drawn (SQ-0734). Phrased to complete "…: {reason}".
impl std::fmt::Display for PicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PicError::Truncated => write!(f, "the file is truncated"),
            PicError::UnsupportedContainer => {
                write!(f, "it is not an Infocom picture archive this reader understands")
            }
            PicError::NoSuchPicture => write!(f, "no picture with that number is in the archive"),
            PicError::NoPixelData => write!(f, "the entry is a size-only placeholder"),
            PicError::UnsupportedCompression(flags) => {
                write!(f, "the picture uses a compression variant this reader does not decode (entry flags {flags:#06x})")
            }
            PicError::BadPixelData => write!(f, "the compressed picture data is corrupt"),
            PicError::NotAContinuation(why) => {
                write!(f, "it does not continue this picture set — {why}")
            }
        }
    }
}

/// Entry flag bits, from `amiga/gfx.c`.
const EF_TRANS: u16 = 1; // picture uses colour 0 as transparent
const EF_PHUFF: u16 = 2; // picture is Huffman-coded
const EF_XOR2: u16 = 4; // picture was XORed on alternate lines
const EF_MONO: u16 = 8; // two-colour picture
const EF_IFF: u16 = 32; // picture is IFFed

/// Header flag bits, from `amiga/gfx.c`. Byte 1 of the file is `gh.flags`.
const HF_EHUFF: u8 = 2; // directory records carry a Huffman-tree pointer
const HF_GHUFF: u8 = 4; // the file header carries one global Huffman-tree pointer

/// Size of one Amiga/Mac directory record without a per-entry Huffman pointer:
/// `picID`, `picX`, `picY`, `eFlags` (2 bytes each) then `dataOff` and `palOff`
/// (3 bytes each). `ReadGFXEntry` reads exactly these, in this order.
const ENTRY_SIZE: usize = 14;
/// A per-entry Huffman-tree pointer adds a word to the record — the 16-byte
/// flavour. See [`InfocomPics::parse`].
const HUFF_PTR_SIZE: usize = 2;

/// The record with no palette pointer: `picID`, `picX`, `picY`, `eFlags`,
/// `dataOff` (3 bytes) and one pad byte. `mac/gfx.p`'s `ReadGFXEntry` is where
/// that pad byte is written down, and it names the condition too — a *mono*
/// entry has no palette offset and so "skip[s] over /single/ pad byte" instead:
///
/// ```text
/// { is there a palette entry? }
/// IF ge.mono THEN                         { NO [implied] }
///     BEGIN
///     doZero (4);
///     pEnt := Ptr (ORD4(pEnt) + 1);       { skip over /single/ pad byte }
///     END
/// ELSE                                    { Yes, read/align palOff }
/// ```
///
/// **Both machines spend the same twelve bytes for the same reason**, and it is
/// one phenomenon and not two: a screen whose colours are not the picture's to
/// choose has no palette to store. On the PC that is EGA and CGA, whose hardware
/// fixed sixteen and two colours respectively; on the Macintosh it is the
/// two-colour `Pic.data` that ships beside the colour `CPic.data`. All 483 of
/// the Mac monochrome archive's records leave this byte zero (SQ-0838).
const ENTRY_SIZE_NO_PAL: usize = 12;
/// The PC record with a palette pointer, laid out exactly like the Amiga's 14.
/// MCGA archives use it — MCGA could pick its 16 colours freely, so it had to
/// carry them.
const PC_ENTRY_SIZE_PAL: usize = ENTRY_SIZE;

/// The Apple II record — `PLDSIZE` in `apple/yzip/rel.15/zip.equ`, and the
/// fourth entry length the format uses.
///
/// The other three flavours spend 2 bytes each on width, height and flags; the
/// Apple spends **one**, and drops the palette pointer entirely. `zip.equ` lays
/// the whole record out and computes the size from it, so the 8 is derived
/// rather than asserted:
///
/// ```text
/// PLDID   EQU 0           ; Picture ID
/// PLDWID  EQU PLDID+2     ; Picture Width
/// PLDHGHT EQU PLDWID+1    ; Picture Height
/// PLDFLG  EQU PLDHGHT+1   ; Flags
/// PLDPTR  EQU PLDFLG+1    ; Pointer to picture data
/// PLDSIZE EQU PLDPTR+3    ; size of local directory entry
/// ```
///
/// One byte each caps a picture at 255×255, which the Apple's own picture space
/// (140×192 — `apple.equ`'s `MAXWIDTH EQU 140 ; 560 / 4` and `MAXHEIGHT EQU
/// 192`) never comes close to.
const APPLE_ENTRY_SIZE: usize = 8;

/// Bytes of length prefix in front of an Apple picture's RLE stream. `pic.asm`
/// steps over them before decoding — "`lda #3 ; 3 bytes of width data start
/// it`" — and they hold the compressed length, which is what makes them an
/// independent check on the decoder (see [`InfocomPics::decode_apple`]).
const APPLE_PREFIX: usize = 3;

/// `PLDFLG` bit 0: the picture has a transparent colour. `pic.asm`:
///
/// ```text
/// lda PICINFO+PLDFLG      ; get flag byte
/// and #1                  ; is there a transparent color?
/// bne ZDSP11              ; ayyup
/// lda #$FF                ; make TRANSCLR be $FF
/// ```
///
/// Which colour is the flag byte's **top nibble** — the same idea as the other
/// flavours' `flags >> 12`, one byte narrower because `PLDFLG` is one byte
/// wide. `pic.asm` shifts it down four bits, calling the byte "hi byte of flag
/// word", which is exactly what it is: the other machines' 16-bit `eFlags` with
/// the always-zero low half dropped.
///
/// ```text
/// ZDSP11:
///     lda PICINFO+PLDFLG  ; get hi byte of flag word
///     lsr A               ; put in lower byte
///     lsr A               ; put in lower byte
///     lsr A               ; put in lower byte
///     lsr A               ; put in lower byte
/// ZDSP12:
///     sta TRANSCLR        ; save transparent color
/// ```
const APPLE_EF_TRANS: u16 = 1;

/// Header flag `PHFPAL` — `zip.equ`, "No pallette information".
///
/// The Apple's header flags are its own set and only partly line up with the
/// Amiga's `HF_*`. `zip.equ` names four:
///
/// ```text
/// PHFGD    EQU $1     ; data has global directory
/// PHFHUFF  EQU $2     ; Huffman encoded pictures
/// PHFHUFF1 EQU $4     ; All pictures use same Huff tree
/// PHFPAL   EQU $8     ; No pallette information
/// ```
///
/// `$2` and `$4` are the same two bits [`HF_EHUFF`] and [`HF_GHUFF`] name, with
/// the same meaning, which is why one tree test serves every flavour. `$8` is
/// **not** the PC's picture-space-width bit — it says the file stores no
/// palettes, which is the same thing the 8-byte record says by having no room
/// for a palette pointer. Every Arthur archive reads `0x08`: no global
/// directory in the file itself, no Huffman, no palettes.
const PHFPAL: u8 = 8;

/// GIF-style LZW, as the PC archives use it. Minimum code size 8, so the two
/// reserved codes land at 256 and 257 and the first assignable code at 258;
/// codes are packed least-significant bit first and widen from 9 bits to 12.
const LZW_MIN_CODE_WIDTH: u32 = 9;
const LZW_CLEAR: u16 = 256;
const LZW_END: u16 = 257;
const LZW_FIRST_CODE: u16 = 258;
const LZW_MAX_CODE_WIDTH: u32 = 12;
const LZW_MAX_CODES: usize = 1 << LZW_MAX_CODE_WIDTH;
/// Bytes of length prefix in front of each picture's compressed data
/// (`LBYTES` in `amiga/gfx.c`), twice over: `minSize` then `midSize`.
const LBYTES: usize = 3;
/// The flat Huffman tree is a 256-byte array of node/leaf bytes.
const HUFF_LEN: usize = 256;

/// The 16-colour default palette. A picture whose directory entry names no
/// palette inherits whatever palette the interpreter last loaded; when there is
/// nothing to inherit this is the fallback, and it is also what the Blorb
/// conversions of Zork Zero baked into such pictures.
///
/// It is *not* the IBM EGA or CGA hardware table — index 6 is dark yellow where
/// both of those show brown — and a PC EGA or CGA archive, which stores no
/// palettes of its own, must be drawn through [`InfocomPics::hardware_palette`]
/// instead of through this. SQ-0794.
pub const DEFAULT_PALETTE: [Rgb; 16] = [
    [0, 0, 0],
    [0, 0, 170],
    [0, 170, 0],
    [0, 170, 170],
    [170, 0, 0],
    [170, 0, 170],
    [170, 170, 0],
    [170, 170, 170],
    [85, 85, 85],
    [85, 85, 255],
    [85, 255, 85],
    [85, 255, 255],
    [255, 85, 85],
    [255, 85, 255],
    [255, 255, 85],
    [255, 255, 255],
];

/// The IBM EGA's sixteen default colours, which an `.EG1`/`.EG2` archive's
/// pixel indices name directly (SQ-0794).
///
/// The EGA drove six digital lines — a primary and a secondary bit per channel
/// — so a channel is off, one third, two thirds or full: **0, 85, 170, 255**.
/// Indices 0..=7 use the primaries alone (170 where lit) and 8..=15 add the
/// intensity bit (255 where the primary is lit, 85 where it is not).
///
/// **Index 6 is the exception, and it is the whole reason this table exists
/// separately from [`DEFAULT_PALETTE`].** Arithmetic says dark yellow,
/// `(170, 170, 0)`; the hardware halves green and shows **brown**,
/// `(170, 85, 0)`. Zork Zero's EGA artwork dithers brown against bright red to
/// make its bronze scrollwork, so getting this one entry wrong turns the whole
/// plate pink and olive.
///
/// Verified against four sources that agree entry for entry:
///
/// * Spatterlight bocfel's `ega_colormap` (`z6/draw_image.cpp:58`), which is
///   what it hands every EGA picture (`populate_color_table`, same file);
/// * the IBM EGA's own 2-bits-per-channel encoding, as ModdingWiki's *EGA
///   Palette* states it — "each 2-bit EGA red, green or blue value should be
///   multiplied by 85", with index 6 remapped to `#AA5500` by "additional
///   circuitry";
/// * int10h.org's *IBM 5153's True CGA Palette*, which quotes the canonical
///   sixteen as `#000000 #0000AA #00AA00 #00AAAA #AA0000 #AA00AA #AA5500
///   #AAAAAA #555555 #5555FF #55FF55 #55FFFF #FF5555 #FF55FF #FFFF55 #FFFFFF`;
/// * Frotz's DOS front end, indirectly but decisively: `bcpic.c` sets
///   `colour_shift = 0` for `_EGA_`, so a stored index goes to the EGA hardware
///   colour of the same number with no reserved-slot shift — which is what makes
///   this a flat 0..=15 table rather than one starting at `PALETTE_BASE`.
///
/// Bocfel's own comment records a disagreement — "this is `{170,170,0}` in
/// pix2gif (?)" — and pix2gif is the one that is wrong: it is the arithmetic
/// value, i.e. exactly the entry [`DEFAULT_PALETTE`] carries.
pub const EGA_PALETTE: [Rgb; 16] = [
    [0, 0, 0],
    [0, 0, 170],
    [0, 170, 0],
    [0, 170, 170],
    [170, 0, 0],
    [170, 0, 170],
    [170, 85, 0], // brown, not the arithmetic dark yellow
    [170, 170, 170],
    [85, 85, 85],
    [85, 85, 255],
    [85, 255, 85],
    [85, 255, 255],
    [255, 85, 85],
    [255, 85, 255],
    [255, 255, 85],
    [255, 255, 255],
];

/// The colours an IBM PC two-colour archive's pixel indices name — **black and
/// the card's light grey, and nothing else** (SQ-0794, SQ-0956).
///
/// # Two machines, TWO tables — the lit state is not the same colour
///
/// This was one table for both two-colour machines until a capture of each was
/// put beside the other, and they disagree about white (SQ-0956):
///
/// | capture                                  | page      | lit state |
/// |------------------------------------------|-----------|-----------|
/// | `machine-screenshots/dos-zorkzero-cga.png` | `#000000` | `#A0A0A0` |
/// | `machine-screenshots/mac-zorkzero-game.png` | `#FFFFFF` | `#000000` (ink) |
///
/// The Macintosh's plate is black ink on a real white page — 77.2% of the game
/// window is `#FFFFFF` against 22.8% `#000000`, and the pillars are dithered black
/// against it — so it keeps the white it always had, in [`MAC_MONO_PALETTE`]. The IBM's is the other polarity
/// and its lit state is **light grey**: `#A0A0A0` is a DOS emulator's rendering of
/// EGA entry 7, `#AAAAAA`, the same value `dos-hitchhiker.png` measures for its ink
/// and the value `zvm::interpreter`'s IBM PC row already records. A colour NUMBER,
/// not a shade — see `machine-screenshots/info.txt` on why these captures pin
/// numbers.
///
/// The `EF_MONO` flag cannot tell the two apart (bocfel handles
/// `kGraphicsTypeMacBW` and `kGraphicsTypeCGA` in a single fallthrough, and the
/// Mac archive's pixels bear the identity out: across all 386 of Zork Zero's
/// monochrome pictures the indices that appear are exactly `{0, 2, 3}`, with 0
/// appearing only in the 128 that declare `EF_TRANS`). The CONTAINER can, and
/// that is what [`InfocomPics::two_colour_palette`] reads.
///
/// # Why a CGA rendition has two colours and not four
///
/// A `.CG1` declares a 640-wide picture space ([`InfocomPics::picture_space_width`]),
/// and the IBM CGA had exactly one 640-wide mode: mode 6, 640x200, **two**
/// colours. Its four-colour mode is 320 wide. So there is no cyan/magenta or
/// green/red/brown palette to choose here; there is a foreground and a
/// background, and Infocom used the default white on black.
///
/// Both reference interpreters say the same thing in their own terms:
///
/// * Frotz's DOS front end wrote straight to CGA hardware, packing the
///   picture down to one bit per pixel before OR-ing it into the framebuffer
///   — a set bit is a white pixel, a clear bit a black one, and nothing else
///   is representable.
/// * Spatterlight bocfel's `populate_color_table` (`z6/draw_image.cpp`) groups
///   `kGraphicsTypeCGA` with `kGraphicsTypeMacBW` and fills only two slots —
///   `color_table[2]` white, `color_table[3]` black — leaving the rest black.
///   Its opaque path, `draw_opaque_cga`, expands one bit per pixel to
///   `monochrome_white`/`monochrome_black`.
///
/// # Why slots 1 and 2 are both the lit state
///
/// [`InfocomPics::decode`] normalises a CGA archive's **two** pixel packings into
/// one index-per-pixel buffer, and they do not agree on which index means white:
///
/// | packing | condition | indices it yields |
/// |---|---|---|
/// | one bit per pixel | `EF_MONO`, no `EF_TRANS` | 1 = set = white, 0 = clear = black |
/// | one byte per pixel | `EF_MONO` and `EF_TRANS` | 0 = transparent, 2 = white, 3 = black |
///
/// The bit-packed row is the PC's alone. Monochrome MACINTOSH art is always the
/// byte-per-pixel form — bocfel's "unlike monochrome Mac images which use the
/// same wasteful format as transparent ones" — so it reaches slots 2 and 3 and
/// never slot 1, whether or not it declares `EF_TRANS`.
///
/// Slots 2 and 3 are the archive's own colour numbers, exactly as both witnesses
/// above read them. Slot 1 is this decoder's rendering of a set bit, and giving
/// it the lit state is unambiguous because the two packings never share a picture:
/// across `zork0.cg1`, `arthur.cg1`, `journey.cg1` and `shogun.cg1` — 710 pictures
/// — the packed ones use `{0, 1}` and only `{0, 1}`, and the byte-per-pixel ones
/// use `{0, 2, 3}` and only `{0, 2, 3}`. Index 1 therefore never appears in a
/// picture where it would have to mean black.
///
/// Slot 0 is black for the packed pictures; in the byte-per-pixel ones it is the
/// transparent colour, which [`Picture::rgba_with`] drops before this table is
/// consulted.
///
/// bocfel lets a theme override its `monochrome_black`/`monochrome_white`. This
/// table does not: it states what the hardware did, and a host that wants to
/// recolour it can pass its own table to [`Picture::rgba_with`].
pub const CGA_PALETTE: [Rgb; 16] = [
    [0, 0, 0],       // 0: packed clear bit; transparent in the byte-per-pixel form
    [170, 170, 170], // 1: packed set bit — the card's light grey, EGA entry 7
    [170, 170, 170], // 2: the archive's white, as the card showed it
    [0, 0, 0],       // 3: the archive's black
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
];

/// The colours the **Macintosh's** monochrome `Pic.data` names: black ink on a
/// real white page (SQ-0838, split from [`CGA_PALETTE`] by SQ-0956).
///
/// Same `EF_MONO` flag, same byte-per-pixel packing, same `{0, 2, 3}` indices — and
/// a different lit state, which is the whole reason this is a second constant.
/// `machine-screenshots/mac-zorkzero-game.png` (Zork Zero at the Banquet Hall on a
/// compact Macintosh — a 512x342 screen inside a 20px emulator border) censuses
/// **77.2% `#FFFFFF`** against **22.8% `#000000`** inside the 480x300 game window,
/// those two and nothing else, because the capture is 1-bit. The pillars are
/// dithered black against that white. That
/// white is the same one `mac/xzip.lst` states for the machine's own page
/// (`SetColor := (zWHITE*256) + zBLACK`), so the archive and the interpreter agree,
/// exactly as SQ-0838 found them agreeing about the window size.
///
/// Slot 1 is unreachable here — monochrome Mac art is never bit-packed — but it is
/// filled in for the same reason the CGA table fills it: the two tables must be
/// interchangeable at every index a caller can index.
pub const MAC_MONO_PALETTE: [Rgb; 16] = [
    [0, 0, 0],       // 0: transparent in the byte-per-pixel form
    [255, 255, 255], // 1: a set bit, were one ever packed
    [255, 255, 255], // 2: the archive's white — the Mac's page
    [0, 0, 0],       // 3: the archive's black — its ink
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
];

/// The Apple II double-hi-res sixteen, which an Apple archive's pixel indices
/// name directly — the analogue of [`EGA_PALETTE`], and needed for the same
/// reason: the record has no palette pointer because the colours were in the
/// hardware, not in the file.
///
/// **This is Infocom's own table.** `apple/yzip/apl2iff/apple.pal` in the
/// published tree is the palette their `apl2iff` tool expanded these very
/// archives through, and its 48 bytes of RGB are transcribed here entry for
/// entry:
///
/// ```text
/// 0f0f0f cc0077 999911 ff1111 33cc99 bbbbcc 44ff44 ffdd00
/// 0044cc ee66ff 99cc99 ff99aa 11ccee aaddff aaffaa ffffff
/// ```
///
/// The order is the Apple II DHGR order, and that is the corroboration: black,
/// deep red, brown, orange, dark green, grey, green, yellow, dark blue, violet,
/// grey, pink, medium blue, light blue, aqua, white — the sixteen in the
/// sequence the hardware's four-bit nibble produces them, which is what
/// `applebit.c` reads out of a Dazzle Draw screen ("Each pixel is four bits")
/// and what `pic.asm`'s `PIC2SCR` writes back ("`lda #4 ; 4 bits per pixel`").
///
/// Note index 0 is `0f0f0f` and not pure black, and the two greys at 5 and 10
/// are deliberately unequal (`bbbbcc` and `99cc99`) where the hardware makes
/// them the same colour. Both are Infocom's values and are kept rather than
/// "corrected": the file is the authority for what these indices meant to the
/// people who drew the art.
pub const APPLE_DHGR_PALETTE: [Rgb; 16] = [
    [0x0f, 0x0f, 0x0f],
    [0xcc, 0x00, 0x77],
    [0x99, 0x99, 0x11],
    [0xff, 0x11, 0x11],
    [0x33, 0xcc, 0x99],
    [0xbb, 0xbb, 0xcc],
    [0x44, 0xff, 0x44],
    [0xff, 0xdd, 0x00],
    [0x00, 0x44, 0xcc],
    [0xee, 0x66, 0xff],
    [0x99, 0xcc, 0x99],
    [0xff, 0x99, 0xaa],
    [0x11, 0xcc, 0xee],
    [0xaa, 0xdd, 0xff],
    [0xaa, 0xff, 0xaa],
    [0xff, 0xff, 0xff],
];

/// Colour index at which a picture's own palette starts. Indices 0 and 1 are
/// reserved (0 is the transparent colour when `EF_TRANS` is set); a 14-entry
/// palette therefore fills 2..=15.
const PALETTE_BASE: usize = 2;

/// One directory record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PicEntry {
    /// Picture number, as the story's `@draw_picture` refers to it.
    pub id: u16,
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// Entry flags (`EF_*`).
    pub flags: u16,
    data: usize,
    palette: usize,
    /// Byte offset of the 256-byte Huffman tree this picture decodes through —
    /// the file's one global tree, or the record's own, per the header flags.
    huff: usize,
}

impl PicEntry {
    /// Whether this entry carries compressed pixels. When false the entry is a
    /// size-only placeholder that the game fills with a solid rectangle.
    pub fn has_pixels(&self) -> bool {
        self.data != 0 && self.width != 0 && self.height != 0
    }

    /// Whether this entry names a palette of its own.
    ///
    /// A zero palette offset is the native format's way of saying *adaptive*:
    /// the picture has no colours of its own and must be drawn through whatever
    /// palette the interpreter last loaded. It is exactly the condition Blorb
    /// spells out with a top-level `APal` chunk — for Zork Zero the 172 entries
    /// here that have pixels and no palette are, id for id, the 172 numbers
    /// `Zork0.blb` lists in `APal`.
    ///
    /// A PC **EGA or CGA** archive stores no palettes at all — its records are
    /// 12 bytes with no room for one, because those adapters fix their colours
    /// in hardware. Every picture in such a file therefore reads as adaptive,
    /// and a caller must supply the hardware table —
    /// [`InfocomPics::hardware_palette`] is where it comes from. Only MCGA,
    /// which could choose its sixteen colours freely, carries palettes on the PC
    /// side.
    pub fn has_own_palette(&self) -> bool {
        self.palette != 0
    }
}

/// A decoded picture: palette indices plus the palette to read them through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// One palette index per pixel, row-major, `width * height` long.
    pub indices: Vec<u8>,
    /// The picture's own palette, if it carries one. `None` means it inherits
    /// the interpreter's current palette.
    pub palette: Option<[Rgb; 16]>,
    /// The colour index drawn as transparent, if any.
    pub transparent: Option<u8>,
}

impl Picture {
    /// Straight RGBA8 expansion, row-major. Falls back to [`DEFAULT_PALETTE`]
    /// when the picture carries no palette of its own; the transparent index,
    /// if any, gets alpha 0.
    ///
    /// A picture with no palette is *adaptive* (see [`PicEntry::has_own_palette`])
    /// and the fallback is only what to show when nothing has been loaded yet —
    /// a caller that has a current palette should pass it to [`rgba_with`].
    ///
    /// [`rgba_with`]: Picture::rgba_with
    pub fn rgba(&self) -> Vec<u8> {
        self.rgba_with(&self.palette.unwrap_or(DEFAULT_PALETTE))
    }

    /// RGBA8 expansion through a caller-supplied colour table — the adaptive
    /// path, where the palette comes from the interpreter rather than the
    /// picture.
    pub fn rgba_with(&self, pal: &[Rgb; 16]) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.indices.len() * 4);
        for &i in &self.indices {
            let c = pal[usize::from(i) & 15];
            let a = if Some(i) == self.transparent { 0 } else { 255 };
            out.extend_from_slice(&[c[0], c[1], c[2], a]);
        }
        out
    }
}

/// A parsed native picture archive: owns the file bytes and the directory.
#[derive(Debug)]
pub struct InfocomPics {
    data: Vec<u8>,
    entries: Vec<PicEntry>,
    part: u8,
    /// How many files have been merged into this archive — 1 until
    /// [`InfocomPics::append_part`] absorbs a continuation (SQ-0798).
    parts: u8,
    flavour: Flavour,
}

impl InfocomPics {
    /// Parse a native picture archive of either flavour.
    ///
    /// # Telling the two apart
    ///
    /// Neither flavour carries a signature, and the two headers occupy the same
    /// sixteen bytes, so the discriminator has to come from the format's own
    /// semantics. It does: **a Huffman-coded archive must name a Huffman tree**,
    /// and `ReadGFXEntry` in `amiga/gfx.c` says where:
    ///
    /// ```text
    /// if (BAND (gh.flags, HF_EHUFF+HF_GHUFF) == HF_EHUFF)
    ///     { ge.huffOff = 0; doCopy (pEnt, (UBYTE *)(&ge.huffOff) + 2, 2); ge.huffOff *= 2; }
    /// else    ge.huffOff = gh.huffOff;
    /// ```
    ///
    /// Either `HF_EHUFF` stands alone and every record names its own tree, or
    /// the header's word at offset 2 names the file's one global tree. An
    /// archive that does neither cannot be running the Huffman codec — and that
    /// is what every PC archive looks like: `HF_EHUFF` clear and a tree word of
    /// zero, which reads zero in either byte order, so the test does not have to
    /// know the byte order before it can pick one.
    ///
    /// Two header fields that look like discriminators are **not** used, because
    /// measurement says they do not separate the flavours:
    ///
    /// * the record size at offset 8 — MCGA archives use 14, the same as the
    ///   Amiga, and only EGA/CGA use 12;
    /// * the flag byte at offset 1. It is the picture-space width, not a
    ///   platform: Frotz's `ux_pic.c` reads it as
    ///   `x_scale = (flags & 0x08) ? 640 : 320`, which is why CGA and EGA
    ///   archives read `0x38` and MCGA ones `0x30`.
    ///
    /// Frotz never has to make this decision at all — `bcpic.c` builds the
    /// filename from the display mode (`extension[1] = "cmem"[display - 2]`) and
    /// so always knows what it opened — and it labels the tree word at offset 2
    /// `unused1`, which it is for a file that has no tree. Infocom's own sources
    /// are what give that word a meaning, and having one is exactly what being
    /// the other flavour consists of.
    ///
    /// # Header
    ///
    /// `InitGFX` in `amiga/gfx.c` reads the 16-byte header field by field, and
    /// that is where every constant below comes from. `mac/gfx.p`'s
    /// `gfx_Header` record and the Apple II's `zip.equ` (`PHFID`, `PHFLG`,
    /// `PHHUFF`, `PHNLD`, `PHNGD`, `PHDSIZE`, `PHSIZE = 16`) name the same
    /// fields at the same offsets, which is what makes the layout common to
    /// every flavour rather than an Amiga peculiarity:
    ///
    /// | offset | field | note |
    /// |---|---|---|
    /// | 0 | `gh.fileID` | byte; "not used on Amiga" |
    /// | 1 | `gh.flags` | byte; the `HF_*` set |
    /// | 2..4 | `gh.huffOff` | word, **doubled** to a byte offset |
    /// | 4..6 | `gh.nPics` | records in this file |
    /// | 6..8 | `gh.ngPics` | records in the global file, if any |
    /// | 8 | `gh.dirEntryLen` | byte; directory record size |
    ///
    /// # Record sizes
    ///
    /// `mac/gfx.p` calls `entryLen` "length of each entry (12-14-16)" and
    /// `ReadGFXEntry` shows how each length is spent. All three start with the
    /// same 8 bytes of `picID`/`picX`/`picY`/`eFlags` and a 3-byte `dataOff`.
    /// Then:
    ///
    /// * **12** — no palette pointer, just a pad byte ("skip over /single/ pad
    ///   byte"). The EGA and CGA archives, whose colours the hardware fixes —
    ///   **and the Macintosh's monochrome `Pic.data`**, whose two colours are
    ///   likewise not the picture's to choose. `ReadGFXEntry` reaches this
    ///   branch on `ge.mono`, i.e. on the entry flag `GF_MONO`, and the two
    ///   machines are one case and not two (SQ-0838).
    /// * **14** — a 3-byte `palOff`. The Amiga/Mac colour archives, and MCGA.
    /// * **16** — `palOff` plus a per-entry Huffman-tree word, which
    ///   `ReadGFXEntry` reads only when the header declares `HF_EHUFF` without
    ///   `HF_GHUFF`:
    ///
    /// ```text
    /// if (BAND (gh.flags, HF_EHUFF+HF_GHUFF) == HF_EHUFF)
    ///     { ge.huffOff = 0; doCopy (pEnt, (UBYTE *)(&ge.huffOff) + 2, 2); ge.huffOff *= 2; }
    /// else    ge.huffOff = gh.huffOff;
    /// ```
    ///
    /// Zork Zero, Journey and Arthur declare `6` (`HF_EHUFF | HF_GHUFF`) on the
    /// Amiga and share one global tree in 14-byte records; Shogun declares `2`
    /// (`HF_EHUFF` alone) and carries 48 trees in 16-byte records; the
    /// Macintosh's monochrome `Pic.data` declares `0x0e` — Zork Zero's `6` plus
    /// the `GF_MONO` bit — and shares one global tree in **12**-byte records.
    /// So 16 is demanded exactly when `HF_EHUFF` stands alone, and otherwise 12
    /// and 14 are both read, the size alone saying whether palettes are stored.
    /// That is the same rule the PC side already used, and it is now one rule.
    ///
    /// # Validation
    ///
    /// The format carries no signature, so this validates structurally: the
    /// declared record size must be one the flavour allows, the directory must
    /// fit, ids must ascend (`ReadGFXEntry` binary-searches the directory, so
    /// they have to), and every offset a record names must land inside the file
    /// and past the directory.
    pub fn parse(data: Vec<u8>) -> Result<InfocomPics, PicError> {
        if data.len() < 16 {
            return Err(PicError::Truncated);
        }
        let per_entry_huff = data[1] & (HF_EHUFF | HF_GHUFF) == HF_EHUFF;
        // No tree named anywhere means the Huffman codec cannot be in play, and
        // a zero word reads zero whichever end it is written from — so this one
        // test picks the flavour without first having to know the byte order.
        //
        // Which of the two TREELESS flavours it is, is then settled by the
        // record size, and unambiguously: the Apple spends 8 (`PLDSIZE`) and the
        // PC 12 or 14, and no length is legal for both. The tree word keeps
        // doing all the work it did before — it still separates Huffman from
        // not-Huffman, and nothing Huffman-coded reaches this line — so the
        // discriminator is not weakened, only refined on the side of it that
        // already held more than one rendition.
        if !per_entry_huff && be16(&data, 2) == 0 {
            return if usize::from(data[8]) == APPLE_ENTRY_SIZE {
                Self::parse_apple(data, 0)
            } else {
                Self::parse_pc(data)
            };
        }
        // 16 exactly when the records carry their own trees; otherwise 14, or 12
        // when the pictures are monochrome and have no palette to point at.
        let entry_size = usize::from(data[8]);
        let allowed = if per_entry_huff {
            ENTRY_SIZE + HUFF_PTR_SIZE
        } else if entry_size == ENTRY_SIZE_NO_PAL {
            ENTRY_SIZE_NO_PAL
        } else {
            ENTRY_SIZE
        };
        if entry_size != allowed {
            return Err(PicError::UnsupportedContainer);
        }
        let count = usize::from(be16(&data, 4));
        // `gh.huffOff`, stored as a word that addresses words.
        let global_huff = usize::from(be16(&data, 2)) * 2;
        let dir_end = 16 + count * entry_size;
        if count == 0 || dir_end > data.len() {
            return Err(PicError::UnsupportedContainer);
        }
        // A tree must sit past the directory and inside the file. With one
        // global tree that is a single check; with per-entry trees the header
        // word is unused (zero in Shogun) and each record answers for itself.
        let in_range = |off: usize| (dir_end..data.len()).contains(&off);
        if !per_entry_huff && !in_range(global_huff) {
            return Err(PicError::UnsupportedContainer);
        }

        let mut entries: Vec<PicEntry> = Vec::with_capacity(count);
        for i in 0..count {
            let o = 16 + i * entry_size;
            let e = PicEntry {
                id: be16(&data, o),
                width: be16(&data, o + 2),
                height: be16(&data, o + 4),
                flags: be16(&data, o + 6),
                data: be24(&data, o + 8),
                // A 12-byte record spends its last byte on padding, not on a
                // palette pointer — `ReadGFXEntry`'s mono branch.
                palette: if entry_size == ENTRY_SIZE_NO_PAL {
                    0
                } else {
                    be24(&data, o + 11)
                },
                huff: if per_entry_huff {
                    usize::from(be16(&data, o + 14)) * 2
                } else {
                    global_huff
                },
            };
            if e.data >= data.len() || e.palette >= data.len() {
                return Err(PicError::Truncated);
            }
            if e.has_pixels() && e.flags & EF_PHUFF != 0 && !in_range(e.huff) {
                return Err(PicError::UnsupportedContainer);
            }
            if entries.last().is_some_and(|p| p.id >= e.id) {
                return Err(PicError::UnsupportedContainer);
            }
            entries.push(e);
        }
        let part = data[0];
        Ok(InfocomPics {
            data,
            entries,
            part,
            parts: 1,
            flavour: Flavour::AmigaMac,
        })
    }

    /// Parse a PC archive: `.MG1`, `.EG1`, `.EG2` or `.CG1`.
    ///
    /// Same header, same directory, read the other way round — with one twist
    /// that is not a matter of taste. The 16-bit fields are little-endian, as
    /// x86 wrote them, but the **3-byte offsets stay most-significant byte
    /// first**. The Apple II is the source that settles that, because it is the
    /// other little-endian platform Infocom shipped this format on, and
    /// `pic.asm` reads its pointer high byte first:
    ///
    /// ```text
    /// lda   PICINFO+PLDPTR     ; MSB of offset
    /// sta   PFSEEK+SM_FPOS+2   ; MSB of seek
    /// lda   PICINFO+PLDPTR+1   ; Middle
    /// ```
    ///
    /// It is also self-evident in the files: every archive's first data or
    /// palette offset, read that way, lands exactly on the byte after the
    /// directory.
    fn parse_pc(data: Vec<u8>) -> Result<InfocomPics, PicError> {
        let entry_size = usize::from(data[8]);
        if entry_size != ENTRY_SIZE_NO_PAL && entry_size != PC_ENTRY_SIZE_PAL {
            return Err(PicError::UnsupportedContainer);
        }
        let count = usize::from(le16(&data, 4));
        let dir_end = 16 + count * entry_size;
        if count == 0 || dir_end > data.len() {
            return Err(PicError::UnsupportedContainer);
        }
        let in_range = |off: usize| off == 0 || (dir_end..data.len()).contains(&off);

        let mut entries: Vec<PicEntry> = Vec::with_capacity(count);
        for i in 0..count {
            let o = 16 + i * entry_size;
            let e = PicEntry {
                id: le16(&data, o),
                width: le16(&data, o + 2),
                height: le16(&data, o + 4),
                flags: le16(&data, o + 6),
                data: be24(&data, o + 8),
                // A 12-byte record spends its last byte on padding, not on a
                // palette pointer.
                palette: if entry_size == PC_ENTRY_SIZE_PAL {
                    be24(&data, o + 11)
                } else {
                    0
                },
                huff: 0,
            };
            if !in_range(e.data) || !in_range(e.palette) {
                return Err(PicError::UnsupportedContainer);
            }
            // Nothing here can be Huffman-coded — the file named no tree, which
            // is how it got routed to this parser in the first place.
            if e.flags & EF_PHUFF != 0 {
                return Err(PicError::UnsupportedContainer);
            }
            if entries.last().is_some_and(|p| p.id >= e.id) {
                return Err(PicError::UnsupportedContainer);
            }
            entries.push(e);
        }
        let part = data[0];
        Ok(InfocomPics {
            data,
            entries,
            part,
            parts: 1,
            flavour: Flavour::Pc,
        })
    }

    /// Parse an Apple II archive out of the segment that carries it.
    ///
    /// # Why this one takes a base, when the others do not
    ///
    /// An Apple archive is not a file. It lives in the free tail of a packed
    /// volume's segment — `ARTHUR.D2`…`ARTHUR.D5` — **behind** that segment's
    /// story pages, and its `PLDPTR` offsets are positions in the SEGMENT, not
    /// in the archive. `pic.asm` seeks the segment file directly with them:
    ///
    /// ```text
    /// lda PICINFO+PLDPTR      ; MSB of offset
    /// sta PFSEEK+SM_FPOS+2    ; MSB of seek
    /// lda PICINFO+PLDPTR+1    ; Middle
    /// and #$FE                ; seek only to 512byte boundary
    /// sta PFSEEK+SM_FPOS+1
    /// SET_MARK PFSEEK         ; go to pic data
    /// ```
    ///
    /// So `data` is the whole segment and `base` is where its header starts.
    /// Slicing the archive out on its own would strand every offset by exactly
    /// `base`, which is why there is no such thing as a loose Apple archive to
    /// hand to [`parse`](InfocomPics::parse).
    ///
    /// # Finding `base`
    ///
    /// The caller does not have to search for it: the packed volume's own index
    /// says where it is, one entry per segment. `zip.equ` names the field —
    ///
    /// ```text
    /// SGTCHKS  EQU 0  ; check sum for file
    /// SGTPICOF EQU 2  ; picture data offset
    /// SGTNSEG  EQU 4  ; # of segments in this list
    /// SGTGPOF  EQU 6  ; Global Directory Offset
    /// ```
    ///
    /// — and `READ_IN_PDATA` turns it into a file position by shifting it left
    /// nine, i.e. it is a **block number**, with zero meaning the segment has no
    /// artwork. See `infocom_packed::PackedVolume::picture_offsets`.
    ///
    /// # The header, and how it differs
    ///
    /// The same 16 bytes as every flavour, read big-endian. `pic.asm` says so of
    /// the one field it would otherwise be guesswork for, because the 6502 would
    /// naturally read a word the other way and this one does not:
    ///
    /// ```text
    /// lda PIC_DIR+PHNLD       ; get # of entries
    /// lda PIC_DIR+PHNLD+1     ; it's in reverse order
    /// ```
    ///
    /// The record is `APPLE_ENTRY_SIZE` bytes: `PLDID` (2, big-endian),
    /// `PLDWID` (1), `PLDHGHT` (1), `PLDFLG` (1), `PLDPTR` (3, big-endian).
    /// There is no palette pointer and no per-entry tree, so both offsets are
    /// fixed at zero — the file stores no palettes at all, which is what the
    /// header's `PHFPAL` bit states independently.
    ///
    /// Validation is the same shape as the other two: the directory must fit,
    /// ids must ascend, and every data offset must land past the directory and
    /// inside the segment. A zero offset is a size-only placeholder, as
    /// everywhere else.
    pub fn parse_apple(data: Vec<u8>, base: usize) -> Result<InfocomPics, PicError> {
        if base + 16 > data.len() {
            return Err(PicError::Truncated);
        }
        if usize::from(data[base + 8]) != APPLE_ENTRY_SIZE {
            return Err(PicError::UnsupportedContainer);
        }
        // `PHFPAL` and the 8-byte record are two statements of one fact — the
        // file stores no palettes — so they are required to agree. A record with
        // no palette pointer in a file that claims to have palettes is not a
        // rendition that exists; it is arbitrary bytes that happened to read 8.
        if data[base + 1] & PHFPAL == 0 {
            return Err(PicError::UnsupportedContainer);
        }
        let count = usize::from(be16(&data, base + 4));
        let dir_end = base + 16 + count * APPLE_ENTRY_SIZE;
        if count == 0 || dir_end > data.len() {
            return Err(PicError::UnsupportedContainer);
        }
        let in_range = |off: usize| off == 0 || (dir_end..data.len()).contains(&off);

        let mut entries: Vec<PicEntry> = Vec::with_capacity(count);
        for i in 0..count {
            let o = base + 16 + i * APPLE_ENTRY_SIZE;
            let e = PicEntry {
                id: be16(&data, o),
                // One byte each. `PLDWID`, `PLDHGHT`.
                width: u16::from(data[o + 2]),
                height: u16::from(data[o + 3]),
                // One byte, and it is the TOP half of the other flavours' word —
                // see `APPLE_EF_TRANS`. Kept unshifted so the flag bit stays
                // bit 0; `decode` knows which flavour it is reading.
                flags: u16::from(data[o + 4]),
                data: be24(&data, o + 5),
                // No `palOff` and no per-entry tree: the record has no room for
                // either, and `PHFPAL` says the file has no palettes to point at.
                palette: 0,
                huff: 0,
            };
            if !in_range(e.data) {
                return Err(PicError::UnsupportedContainer);
            }
            if entries.last().is_some_and(|p| p.id >= e.id) {
                return Err(PicError::UnsupportedContainer);
            }
            entries.push(e);
        }
        let part = data[base];
        Ok(InfocomPics { data, entries, part, parts: 1, flavour: Flavour::Apple })
    }

    /// The archive's part number. Multi-part sets number their files 1, 2, …
    ///
    /// Arthur and Journey split their EGA artwork in two: `.EG1` reads 1 and
    /// `.EG2` reads 2, and a complete EGA set for either game means both.
    ///
    /// The Apple numbers by **floppy**: `PHFID` is the disk the segment came
    /// from, so Arthur's four archives read 2, 3, 4 and 5 and a complete set
    /// starts at 2 rather than at 1.
    pub fn part(&self) -> u8 {
        self.part
    }

    /// How many files this archive is made of — 1 for an ordinary one, 2 once a
    /// continuation has been absorbed by [`append_part`](InfocomPics::append_part).
    ///
    /// [`part`](InfocomPics::part) stays the number of the FIRST file, so the two
    /// together say which parts are in hand: `part()..part() + parts()`.
    pub fn parts(&self) -> u8 {
        self.parts
    }

    /// The part number this archive would accept next.
    pub fn next_part(&self) -> u8 {
        self.part.saturating_add(self.parts)
    }

    /// Absorb the next file of a multi-part set, so the whole set reads as one
    /// archive (SQ-0798).
    ///
    /// # Why multi-part sets exist
    ///
    /// EGA art is 640 pixels wide and did not fit on one 360K floppy, so Arthur
    /// and Journey split theirs across `.EG1` and `.EG2`. The split is not a
    /// partition: each disk had to be self-sufficient for its stretch of the
    /// game, so a picture wanted on both sides is stored on **both** — 55 of
    /// Arthur's 171 ids and 12 of Journey's 135 appear twice. Reading only part 1
    /// costs Arthur 40 of its 137 pictures and Journey 55 of its 135.
    ///
    /// # The merge rule for a repeated id, and how it was settled
    ///
    /// **The earlier part wins**, and it costs nothing: measured across both
    /// games, every id in two parts has the same dimensions and the same entry
    /// flags, and every one of them that carries pixels decodes to byte-identical
    /// output from either copy (Arthur 37 identical + 18 that are placeholders in
    /// both, Journey 12 identical, zero differing, zero where one part has pixels
    /// and the other does not). So the choice is arbitrary on this corpus, and
    /// first-wins is the one that is deterministic and needs no second decode.
    ///
    /// # What it refuses, loudly
    ///
    /// The header's part byte is the format's own statement of which file this is
    /// (`bcpic.c` builds the very filename from it), so it is checked rather than
    /// trusted: the continuation must call itself [`next_part`](InfocomPics::next_part),
    /// must have been written by the same codec, and must actually **extend** the
    /// set. An archive that repeats what is already held adds nothing and is far
    /// more likely to be an unrelated file that happens to sit under the next
    /// name. `self` is left untouched on any error, so a refusal costs the caller
    /// only part 1 — which is what it already had.
    pub fn append_part(&mut self, next: InfocomPics) -> Result<(), PicError> {
        if next.flavour != self.flavour {
            return Err(PicError::NotAContinuation("a different codec wrote it"));
        }
        if next.part != self.next_part() {
            return Err(PicError::NotAContinuation("its header states a different part number"));
        }
        let mut added: Vec<PicEntry> =
            next.entries.iter().copied().filter(|e| self.entry(e.id).is_none()).collect();
        if added.is_empty() {
            return Err(PicError::NotAContinuation(
                "it holds no picture this set does not already have",
            ));
        }
        // Every offset in a record addresses this archive's own bytes, so the
        // continuation's bytes are appended and its offsets shifted by where they
        // landed. Zero keeps its meaning — "no pixels", "no palette of its own",
        // "no Huffman tree" — and is therefore never shifted.
        let base = self.data.len();
        for e in &mut added {
            for off in [&mut e.data, &mut e.palette, &mut e.huff] {
                if *off != 0 {
                    *off += base;
                }
            }
        }
        self.data.extend_from_slice(&next.data);
        self.entries.append(&mut added);
        // `ReadGFXEntry` binary-searches the directory, and `parse` enforces
        // ascending ids for the same reason; a merged directory keeps that shape.
        self.entries.sort_unstable_by_key(|e| e.id);
        self.parts += 1;
        Ok(())
    }

    /// Which platform wrote this archive, and therefore which codec read it.
    pub fn flavour(&self) -> Flavour {
        self.flavour
    }

    /// The width of the picture space this archive's coordinates are expressed
    /// in: 640 for EGA and CGA, 320 for MCGA and for the Amiga/Mac.
    ///
    /// This is a real property of the archive, not a rendering preference — the
    /// same Zork Zero plate is stored 320 wide in `zork0.mg1` and 640 wide in
    /// `zork0.eg1`, because EGA and CGA addressed a 640-column screen with
    /// pixels half as wide. A host that plots one of these one pixel per pixel
    /// without knowing which it holds gets the aspect ratio wrong by two.
    ///
    /// Source: header byte 1 (`gh.flags`), bit 3. Two independent
    /// implementations read it the same way — Frotz's `src/curses/ux_pic.c`
    /// (`x_scale = (flags & 0x08) ? 640 : 320`) and, through its
    /// `hw_screenwidth`/`pixelwidth` table, Spatterlight's bocfel. Agreement is
    /// measured on this corpus too: every authentic `.MG1` reads `0x30` and
    /// every `.EG1`/`.EG2`/`.CG1` reads `0x38` or `0x39`, while `zork0.pic`
    /// reads `0x06`.
    ///
    /// **The Macintosh's monochrome archive is the exception, and it is 480 wide**
    /// (SQ-0838). Bit 3 of the flags byte is set in its `0x0e` too, so the PC
    /// rule would call it 640 and be wrong by a third; the answer comes from the
    /// archive's own [`Self::is_monochrome`] instead, and it is sourced twice
    /// over. `mac/gfx.p` writes the whole picture space into the flag's own
    /// definition — `GF_MONO = $0008; { this pic is mono, and scaled for a
    /// 480x300 screen (std Mac) }` — and bocfel's width table says the same
    /// (`case kGraphicsTypeMacBW: hw_screenwidth = 480; pixelwidth = 1.0;`).
    /// The directory agrees: the archive's widest picture is exactly 480, where
    /// the colour `CPic.data` beside it tops out at 320.
    ///
    /// An Amiga/Mac COLOUR archive still answers 320, as every one in hand is.
    ///
    /// **The Apple is 140**, and it is the one rendition whose flag bit 3 must
    /// NOT be read as a width: that bit is `PHFPAL` there, not the PC's
    /// picture-space selector, so the PC rule would call every Arthur archive
    /// 640 and be wrong by more than four. The number comes from the machine
    /// instead — `apple.equ`'s `MAXWIDTH EQU 140 ; 560 / 4`, i.e. the
    /// double-hi-res screen's 560 dots at four bits a pixel — and the directory
    /// agrees, topping out at exactly 140 across all four of Arthur's archives.
    pub fn picture_space_width(&self) -> u16 {
        match self.flavour {
            Flavour::AmigaMac if self.is_monochrome() => 480,
            Flavour::AmigaMac => 320,
            Flavour::Pc if self.data[1] & 0x08 != 0 => 640,
            Flavour::Pc => 320,
            Flavour::Apple => 140,
        }
    }

    /// The height of the picture space [`Self::picture_space_width`] names — the
    /// other axis of the screen this archive's coordinates were drawn for.
    ///
    /// Every rendition Infocom shipped is 200 lines tall except the standard
    /// Macintosh's monochrome one, which `mac/gfx.p` calls a "480x300 screen
    /// (std Mac)". That is not a scaled copy of the 320×200 colour art: the two
    /// archives on the Macintosh Zork Zero disk hold the same 483 pictures at
    /// separately drawn sizes, agreeing on 1.5× only for the full-screen plates
    /// (480×300 against 320×200) and going their own way on the sprites.
    ///
    /// A host needs both axes because a 480×300 space is the one that does NOT
    /// double onto a 640×400 unit screen — see `app`'s `PictSource::art_scale`,
    /// where every other rendition's vertical factor works out to 2 and this
    /// one's to 1.
    ///
    /// The Apple's is **192** — `apple.equ`'s `MAXHEIGHT EQU 192`, the
    /// double-hi-res screen's line count — and the directory agrees, its
    /// full-screen plates being exactly 140×192.
    pub fn picture_space_height(&self) -> u16 {
        match self.flavour {
            Flavour::AmigaMac if self.is_monochrome() => 300,
            Flavour::Apple => 192,
            _ => 200,
        }
    }

    /// The colour table this archive's HARDWARE fixed, for the renditions that
    /// store no palettes because there was nothing to store. `None` when the
    /// pictures bring their own colours (SQ-0794).
    ///
    /// This is the answer to the question [`PicEntry::has_own_palette`] raises
    /// and cannot settle: a 12-byte PC record has no room for a palette pointer,
    /// so every picture in an EGA or CGA archive reads as adaptive, when in fact
    /// none of them is — their colours were in the video card. A caller that
    /// takes their silence for deference draws them through
    /// [`DEFAULT_PALETTE`], which is a different table, and Zork Zero's bronze
    /// scrollwork comes out pink.
    ///
    /// # How the rendition is told, without asking the filename
    ///
    /// Both reference interpreters know which card they are on before they open
    /// anything — bocfel maps `.EG1` to `kGraphicsTypeEGA` and `.CG1` to
    /// `kGraphicsTypeCGA` in `find_graphics_files.cpp`, and Frotz's `bcpic.c`
    /// builds the extension from the display mode. lanthorn has no display mode
    /// and does not read renditions out of filenames (see the module header), so
    /// this reads the directory instead. Two questions, in order:
    ///
    /// 1. **Does any picture carry a palette?** If so the archive brings its own
    ///    colours and nothing here applies — MCGA, or an Amiga/Mac *colour*
    ///    archive. This is the record shape asking itself — a 12-byte record can
    ///    never answer yes — and it is deliberately `any` rather than `all`,
    ///    because an MCGA archive mixes pictures that carry palettes with
    ///    pictures that genuinely are adaptive.
    /// 2. **Does every picture with pixels set `EF_MONO`?** That is a two-colour
    ///    rendition. `EF_MONO` is "two-color picture", which no 16-colour
    ///    rendition has a use for, and the corpus splits on it perfectly: all 710
    ///    pixel-bearing pictures in `zork0.cg1`, `arthur.cg1`, `journey.cg1` and
    ///    `shogun.cg1` set it, and none of the 617 in `zork0.eg1`, `arthur.eg1`,
    ///    `journey.eg1`, `shogun.eg1` or `FMVPOKER.EG1` does. All 386 in the
    ///    Macintosh Zork Zero's `Pic.data` set it too, which is the whole reason
    ///    this question is asked of both flavours and not only the PC (SQ-0838).
    ///    The pixels agree independently: every two-colour picture stays inside
    ///    indices 0..=3 and every EGA one uses all sixteen.
    ///
    /// Otherwise **only a PC archive gets an answer**, and it is EGA — the safe
    /// way round, because EGA's table is a superset of the colours two-colour art
    /// can name, so a misread CGA plate comes out in the wrong hues whereas a
    /// misread EGA plate would come out as thirteen shades of black. An
    /// Amiga/Mac archive with no palettes and no `EF_MONO` is genuinely
    /// adaptive — its colours come from the interpreter, and there is no video
    /// card to ask.
    /// Is this a TWO-COLOUR rendition?
    ///
    /// The same `EF_MONO` test [`Self::hardware_palette`] uses to choose the
    /// two-colour table, asked directly, because the answer decides more than
    /// which palette to expand through: a two-colour display cannot give a story
    /// the arbitrary colours §8.3 lets it name (SQ-0806). Two STATES is not none
    /// — Zork Zero's own `color` command on a CGA machine offers a swap of them,
    /// observed on the emulator — but a story asking for blue is asking for
    /// something that is not there.
    ///
    /// Content, not extension — a `.cg1` somebody renamed is still a `.cg1`, a
    /// `.eg1` somebody renamed is not one, and the Macintosh's monochrome
    /// `Pic.data` answers yes under exactly the same test as a `.cg1`.
    pub fn is_monochrome(&self) -> bool {
        self.two_colour_palette().is_some()
    }

    /// The two-colour table for the machine THIS archive came off, or `None`
    /// when the archive is not a two-colour rendition (SQ-0956).
    ///
    /// The `EF_MONO` test is one test and the answer is two tables, because the
    /// two machines' lit state is not the same colour — see [`CGA_PALETTE`] and
    /// [`MAC_MONO_PALETTE`], each of which carries its own capture. The flavour is
    /// the discriminator and it is the only one there is: nothing inside a `.CG1`
    /// names its machine, but the container format does.
    pub fn two_colour_palette(&self) -> Option<[Rgb; 16]> {
        if self.entries.iter().any(|e| e.has_own_palette()) || self.flavour == Flavour::Apple {
            return None;
        }
        let mut with_pixels = 0usize;
        let mut mono = 0usize;
        for e in self.entries.iter().filter(|e| e.has_pixels()) {
            with_pixels += 1;
            mono += usize::from(e.flags & EF_MONO != 0);
        }
        if with_pixels == 0 || mono != with_pixels {
            return None;
        }
        Some(if self.flavour == Flavour::Pc { CGA_PALETTE } else { MAC_MONO_PALETTE })
    }

    pub fn hardware_palette(&self) -> Option<[Rgb; 16]> {
        if self.entries.iter().any(|e| e.has_own_palette()) {
            return None;
        }
        // The Apple's hardware fixed sixteen, and its flag byte is a different
        // set of bits — `EF_MONO` is not among them (`zip.equ` defines bit 0 and
        // the top nibble and nothing else), so the two-colour test below must
        // not be asked of it.
        if self.flavour == Flavour::Apple {
            return self.entries.iter().any(|e| e.has_pixels()).then_some(APPLE_DHGR_PALETTE);
        }
        // Two colours, and WHICH two is the machine's to say (SQ-0838, SQ-0956).
        if let Some(two) = self.two_colour_palette() {
            return Some(two);
        }
        if !self.entries.iter().any(|e| e.has_pixels()) {
            return None;
        }
        // The sixteen-colour fallback is the PC's alone. An Amiga/Mac archive
        // that stores no palettes is *adaptive* — its colours come from the
        // interpreter's current table, not from a video card — so answering
        // `EGA_PALETTE` for one would be inventing hardware it never had.
        (self.flavour == Flavour::Pc).then_some(EGA_PALETTE)
    }

    /// Every directory record, in file order.
    pub fn entries(&self) -> &[PicEntry] {
        &self.entries
    }

    /// The directory record for a picture number.
    pub fn entry(&self, id: u16) -> Option<&PicEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Every picture that must be drawn through the interpreter's current
    /// palette rather than one of its own — this format's `APal` (see
    /// [`PicEntry::has_own_palette`]). Placeholders are excluded: an entry with
    /// no pixels has nothing to colour.
    pub fn adaptive_pictures(&self) -> Vec<u16> {
        self.entries
            .iter()
            .filter(|e| e.has_pixels() && !e.has_own_palette())
            .map(|e| e.id)
            .collect()
    }

    /// A picture's own 16-entry colour table, without decoding its pixels —
    /// what a non-adaptive draw establishes as the current palette. `None` for
    /// an unknown id or an adaptive picture, which has no table of its own.
    pub fn palette_of(&self, id: u16) -> Option<[Rgb; 16]> {
        self.palette(self.entry(id)?)
    }

    /// Decode one picture to palette indices plus its palette.
    pub fn decode(&self, id: u16) -> Result<Picture, PicError> {
        let e = *self.entry(id).ok_or(PicError::NoSuchPicture)?;
        if !e.has_pixels() {
            return Err(PicError::NoPixelData);
        }
        let indices = match self.flavour {
            Flavour::AmigaMac => self.decode_huffed(&e)?,
            Flavour::Pc => self.decode_lzw(&e)?,
            Flavour::Apple => self.decode_apple(&e)?,
        };
        Ok(Picture {
            width: e.width,
            height: e.height,
            indices,
            palette: self.palette(&e),
            // `EF_TRANS` says a colour drops out; which colour is the flag
            // word's top nibble. Both witnesses agree. The Apple II's `pic.asm`
            // uses `$FF` for "none" and otherwise shifts the flag right four
            // bits into `TRANSCLR`; Frotz's `bcpic.c` does the same thing in C —
            // "Bit 0 of "flags" indicates that the picture uses a transparent
            // colour, the top four bits tell us which colour it is", i.e.
            // `transparent = pic_flags >> 12`. Amiga/Mac archives leave that
            // nibble zero throughout, which is the same thing `amiga/gfx.c`
            // describes in prose ("picture uses color 0"); the PC EGA archives
            // really do use it, naming colours 1, 2, 3, 7, 8 and 9.
            //
            // The Apple stores the same nibble in the same place of a field one
            // byte wide instead of two, so the shift is 4 and not 12 — see
            // `APPLE_EF_TRANS` for `pic.asm`'s own four `lsr A`s. Arthur uses
            // it, naming colours 1, 2, 3 and 4.
            transparent: match self.flavour {
                Flavour::Apple => {
                    (e.flags & APPLE_EF_TRANS != 0).then_some((e.flags >> 4) as u8)
                }
                _ => (e.flags & EF_TRANS != 0).then_some((e.flags >> 12) as u8),
            },
        })
    }

    /// Amiga/Mac: Huffman, then run-length, then undo the per-line XOR.
    ///
    /// # `EF_MONO` changes nothing here, and that is the finding
    ///
    /// On the PC side `EF_MONO` picks a second pixel packing — one bit per
    /// pixel, rows padded to whole bytes (see [`decode_lzw`]). On this side it
    /// picks **nothing**: a monochrome Macintosh picture runs the same Huffman,
    /// the same run-length stage and the same per-line XOR, and lands as one
    /// byte per pixel exactly as a sixteen-colour one does. The flag is a
    /// statement about the artwork, not about its packing.
    ///
    /// Three witnesses, and they agree:
    ///
    /// * `mac/gfx.p` sizes the mono buffer at one byte per pixel outright —
    ///   "because a mono pic decompresses into so many bytes (144000 prior to
    ///   MonoPic)", and 144000 is 480×300, the mono picture space (see
    ///   [`Self::picture_space_width`]). Its `MonoPic` then packs those bytes
    ///   down to bits for QuickDraw, which is a *display* step: "note: src rows
    ///   are unpadded".
    /// * bocfel routes `kGraphicsTypeMacBW` to `decompress_amiga` and then to
    ///   `draw_amiga_mac_cga_ega_vga`, the byte-per-pixel path, and says why in
    ///   as many words: "Opaque CGA images store 8 pixels per byte (unlike
    ///   monochrome Mac images which use the same wasteful format as transparent
    ///   ones)".
    /// * the archive itself. All 386 pixel-bearing pictures of the Macintosh
    ///   Zork Zero's `Pic.data` (r296/s881019) expand to exactly `width *
    ///   height` bytes through this function, with no length left over and none
    ///   short.
    ///
    /// SQ-0838.
    ///
    /// [`decode_lzw`]: InfocomPics::decode_lzw
    fn decode_huffed(&self, e: &PicEntry) -> Result<Vec<u8>, PicError> {
        if e.flags & EF_IFF != 0 || e.flags & EF_PHUFF == 0 {
            return Err(PicError::UnsupportedCompression(e.flags));
        }

        let w = usize::from(e.width);
        let h = usize::from(e.height);
        let hdr = e.data + 2 * LBYTES;
        if hdr > self.data.len() {
            return Err(PicError::Truncated);
        }
        let min_size = be24(&self.data, e.data);
        let mid_size = be24(&self.data, e.data + LBYTES);
        let payload = self
            .data
            .get(hdr..hdr + min_size)
            .ok_or(PicError::Truncated)?;

        let mut indices = unhuff_unrle(self.huff_tree(e.huff), payload, mid_size, w * h)?;
        // Undo step 1: each line was XORed with the line above it. Infocom's own
        // decoder zero-fills a virtual row -1, so row 0 comes through unchanged.
        // `EF_XOR2` claims alternate lines, but every known decoder — Infocom's
        // included — XORs every line.
        let _ = EF_XOR2;
        for i in w..w * h {
            indices[i] ^= indices[i - w];
        }
        Ok(indices)
    }

    /// Apple II: run-length, then undo the per-line XOR. No Huffman stage.
    ///
    /// # The codec is the Amiga's, minus its third step
    ///
    /// `mac/gfx.c`'s prose spec (quoted in the module header) lists three stages
    /// and calls the last one **optional**: "3. *Optionally*, the whole thing is
    /// Huffman-coded". The Apple is where that option is declined. `PHFHUFF` is
    /// clear in every Arthur archive and the header names no tree, so what is
    /// left is stages 2 and 1 — the run-length encoding, then the per-line XOR —
    /// and `pic.asm`'s `ZDECLP` is exactly those two, fused into one loop:
    ///
    /// ```text
    /// ZDECLP:
    ///     ldy P_IDX               ; get data index
    ///     lda (L),Y               ; get data byte
    ///     sta ARG8                ; save here
    ///     iny                     ; point to next one
    /// ZDCLP0:
    ///     lda (L),Y               ; is this count or data?
    ///     cmp #16                 ; if <= 15, previous one was data
    ///     bcs ZDCL2               ; nope, must be compressed
    ///     lda #15                 ; show 1 bytes
    ///     bne ZDCL3               ; and count this one
    /// ZDCL2:
    ///     iny                     ; point to next byte
    /// ZDCL3:
    ///     sty P_IDX               ; save index
    ///     sec                     ; get ready for sub
    ///     sbc #14                 ; make good counter
    ///     tax                     ; put count into x
    /// ZDCLPC:
    ///     ldy P_LOFF              ; get line offset
    ///     lda ARG8                ; get data byte
    ///     eor (J),Y               ; XOR with previous line
    ///     sta (K),Y               ; and save away
    /// ```
    ///
    /// A byte is a colour. The byte after it is a repeat count if it is 16 or
    /// more, and `count = n - 14`; otherwise it is the next colour and the run is
    /// one long (`15 - 14`). That is the Macintosh's rule stated as arithmetic
    /// rather than in prose, and the two agree on the worked example the prose
    /// gives — "3 occurrences of byte value 2 will turn into `2 17`" — since
    /// `17 - 14 == 3`.
    ///
    /// `J` is the previous decoded line and `K` the current; `ZDLI` zero-fills
    /// `J` before the first row, so row 0 XORs against zeros and comes through
    /// unchanged, and `COPYPIC1` swaps the two buffers at every row end. Note
    /// the run is of the **pre-XOR** value: each of a run's pixels is XORed
    /// against a different pixel of the row above, so the run cannot be
    /// expanded after the XOR.
    ///
    /// # The length prefix, which is also the oracle
    ///
    /// The pixel data does not start at `PLDPTR`; three bytes of header do, and
    /// `pic.asm` steps over them (`lda #3 ; 3 bytes of width data start it`).
    /// They are the compressed length, big-endian, and this decoder does not
    /// need them — which is exactly what makes them worth checking, because they
    /// are an independent statement of where the RLE stream ends. Across all 135
    /// pixel-bearing pictures of Arthur release 63 (serial 890622,
    /// `Arthur Quest 4 Excalibur.2mg`) the bytes this loop consumes equal the
    /// declared length plus three, every time, and the pictures pack contiguously
    /// with no gaps. A decoder that stopped in the wrong place could not do that
    /// 135 times.
    ///
    /// Output is one byte per pixel, `width * height` long — the same shape
    /// every other flavour lands in. The Apple's own four-bit screen packing
    /// (`PIC2SCR`, "`lda #4 ; 4 bits per pixel`") is a step this decoder does not
    /// take and must not: that is the interpreter writing to double-hi-res video
    /// memory, downstream of the format.
    fn decode_apple(&self, e: &PicEntry) -> Result<Vec<u8>, PicError> {
        let w = usize::from(e.width);
        let h = usize::from(e.height);
        let want = w * h;
        let mut p = e.data.checked_add(APPLE_PREFIX).ok_or(PicError::Truncated)?;
        if p > self.data.len() {
            return Err(PicError::Truncated);
        }

        let mut out = vec![0u8; want];
        // `P_LOFF` walks the row; `filled` is how much of `out` is decoded, and
        // the row above starts at `filled - w`. Writing straight into `out` and
        // XORing against the row already there is the same thing `pic.asm` does
        // with its two swapped line buffers, with `ZDLI`'s zero fill standing in
        // for the row before row 0 (`out` starts zeroed, and row 0's XOR
        // partner is the implicit zero row).
        let mut filled = 0usize;
        while filled < want {
            let d = *self.data.get(p).ok_or(PicError::BadPixelData)?;
            p += 1;
            let n = *self.data.get(p).ok_or(PicError::BadPixelData)?;
            let count = if n < 16 {
                // Not a count — it is the next colour, so this run is one long.
                1
            } else {
                p += 1;
                usize::from(n) - 14
            };
            for _ in 0..count {
                if filled == want {
                    break;
                }
                // Row 0 has no row above it and `out` is zero-filled, so this is
                // `d ^ 0 == d` there, exactly as `ZDLI` arranges.
                let above = if filled >= w { out[filled - w] } else { 0 };
                out[filled] = d ^ above;
                filled += 1;
            }
        }
        Ok(out)
    }

    /// PC: one bare LZW stream per picture, expanding straight to pixels.
    ///
    /// The stream is self-delimiting — it ends on the end-of-stream code — so
    /// the byte count it yields says which of the format's two pixel packings a
    /// picture uses, and the two are never the same size for a picture wider
    /// than one pixel:
    ///
    /// * `width * height` bytes: one palette index per byte. Every MCGA and EGA
    ///   picture, and Zork Zero's whole CGA archive.
    /// * `ceil(width / 8) * height` bytes: one **bit** per pixel, rows padded
    ///   out to whole bytes — `EF_MONO`, "two-color picture". Arthur, Journey
    ///   and Shogun each keep their large CGA artwork this way, 228 pictures
    ///   between them. Frotz's `bcpic.c` calls it out in the same words:
    ///
    ///   ```text
    ///   (There is a special case: CGA pictures
    ///   with no transparent colour are stored as bit patterns, i.e.
    ///   every byte holds the pattern for eight pixels. A pixel must
    ///   be white if the corresponding bit is set, otherwise it must
    ///   be black.)
    ///   ```
    ///
    ///   `bcpic.c` reaches that case by knowing it opened a `.CG1` — it picks
    ///   the extension from the display mode, so it never has to ask a file what
    ///   it is. A reader handed only bytes has no display mode, which is why the
    ///   packing is decided here by `EF_MONO` plus the length the stream itself
    ///   declares. On the 228 packed pictures in the corpus the two rules pick
    ///   the same ones: every packed picture sets `EF_MONO` and clears
    ///   `EF_TRANS`, and no EGA or MCGA picture sets `EF_MONO` at all.
    ///
    /// The high bit of each byte is the leftmost pixel. `bcpic.c` writes the
    /// decoded byte straight into the CGA framebuffer, where bit 7 is the
    /// left-hand pixel of its eight, and unpacking that way also leaves
    /// consistently fewer horizontal colour changes across all three CGA
    /// archives than the other order does.
    fn decode_lzw(&self, e: &PicEntry) -> Result<Vec<u8>, PicError> {
        if e.flags & EF_IFF != 0 {
            return Err(PicError::UnsupportedCompression(e.flags));
        }
        let w = usize::from(e.width);
        let h = usize::from(e.height);
        let flat = w * h;
        let raw = lzw_expand(&self.data[e.data..], flat)?;
        if raw.len() == flat {
            return Ok(raw);
        }
        let stride = w.div_ceil(8);
        if e.flags & EF_MONO != 0 && raw.len() == stride * h {
            let mut out = vec![0u8; flat];
            for y in 0..h {
                for x in 0..w {
                    out[y * w + x] = raw[y * stride + (x >> 3)] >> (7 - (x & 7)) & 1;
                }
            }
            return Ok(out);
        }
        Err(PicError::BadPixelData)
    }

    /// The 256-byte Huffman tree at `off` — the file's global one, or the one a
    /// 16-byte record names for itself.
    fn huff_tree(&self, off: usize) -> &[u8] {
        let end = (off + HUFF_LEN).min(self.data.len());
        &self.data[off..end]
    }

    /// Build the 16-entry colour table for an entry. The stored RGB triples
    /// occupy indices [`PALETTE_BASE`]`..`; anything they do not cover keeps the
    /// default. The stipple-id table that follows the triples (`gp.stips`, a
    /// pattern per colour for 1-bit screens) is not used for colour rendering.
    ///
    /// Both flavours lay the palette out the same way, which is worth saying
    /// because it was found twice independently. `bcpic.c`, on the PC side:
    /// "The first colour to be defined is colour 2. Every map defines up to 14
    /// colours (colour 2 to 15)."
    fn palette(&self, e: &PicEntry) -> Option<[Rgb; 16]> {
        if e.palette == 0 {
            return None;
        }
        let count = usize::from(*self.data.get(e.palette)?);
        let rgb = self.data.get(e.palette + 1..e.palette + 1 + count * 3)?;
        let mut pal = DEFAULT_PALETTE;
        for (i, c) in rgb.as_chunks::<3>().0.iter().enumerate() {
            match pal.get_mut(PALETTE_BASE + i) {
                Some(slot) => *slot = [c[0], c[1], c[2]],
                None => break,
            }
        }
        Some(pal)
    }
}

/// Undo steps 3 and 2: walk the flat Huffman tree for `symbols` symbols,
/// expanding run-length symbols as they come out.
///
/// The tree is a byte array of node pairs. At node `n` (always even) the child
/// for a clear bit is `tree[n]` and for a set bit `tree[n + 1]`. A value below
/// 128 is an internal node id — double it for the array index. A value of 128
/// or more is a leaf carrying symbol `value - 128`. Symbols 0..=15 are colour
/// indices; 16..=127 repeat the last colour `symbol - 15` further times. Bits
/// are read most-significant first.
fn unhuff_unrle(
    tree: &[u8],
    payload: &[u8],
    symbols: usize,
    expect: usize,
) -> Result<Vec<u8>, PicError> {
    let mut out = Vec::with_capacity(expect);
    let mut node = 0usize;
    let mut last = 0u8;
    let mut left = symbols;
    let mut bits = payload.iter().flat_map(|b| (0..8).map(move |i| b >> (7 - i) & 1));
    while left > 0 {
        let bit = bits.next().ok_or(PicError::BadPixelData)?;
        let child = *tree
            .get(node + usize::from(bit))
            .ok_or(PicError::BadPixelData)?;
        if child < 128 {
            node = usize::from(child) * 2;
            continue;
        }
        let sym = child - 128;
        if sym < 16 {
            out.push(sym);
            last = sym;
        } else {
            if out.is_empty() {
                return Err(PicError::BadPixelData);
            }
            out.extend(std::iter::repeat_n(last, usize::from(sym) - 15));
        }
        if out.len() > expect {
            return Err(PicError::BadPixelData);
        }
        left -= 1;
        node = 0;
    }
    if out.len() != expect {
        return Err(PicError::BadPixelData);
    }
    Ok(out)
}

/// Expand one PC picture's LZW stream, stopping at the end-of-stream code.
///
/// This is GIF's variable-width LZW with the minimum code size fixed at 8, per
/// the GIF89a specification: 256 clears the table, 257 ends the stream,
/// assignable codes start at 258, and the code width runs 9 to 12 bits,
/// widening as soon as the table fills the current width — 9 bits address
/// both 256 literal values and 256 table entries, and 12 bits address both
/// 256 literal values and all 3840 table entries above them. Codes are packed
/// least-significant bit first, so a code straddling a byte boundary takes
/// its low bits from the earlier byte.
///
/// 3840 table entries above 256 literals is 4096 codes, which is this module's
/// [`LZW_MAX_CODES`]. A code may legally refer to the table entry currently
/// being assigned (it names the entry this very code is about to create) —
/// that case is the placeholder below.
///
/// `limit` caps the output: a picture is `width * height` pixels at the very
/// most, and a stream claiming more than that is malformed rather than large.
fn lzw_expand(stream: &[u8], limit: usize) -> Result<Vec<u8>, PicError> {
    let mut prefix = [0u16; LZW_MAX_CODES];
    let mut suffix = [0u8; LZW_MAX_CODES];
    let mut out: Vec<u8> = Vec::with_capacity(limit);
    // Codes decode back to front, so a string is stacked and then unwound.
    let mut stack: Vec<u8> = Vec::with_capacity(LZW_MAX_CODES);
    let mut next = LZW_FIRST_CODE;
    let mut width = LZW_MIN_CODE_WIDTH;
    let mut prev: Option<u16> = None;
    let mut bit = 0usize;
    let bits = stream.len() * 8;

    loop {
        if bit + width as usize > bits {
            return Err(PicError::BadPixelData);
        }
        let mut code = 0u16;
        for i in 0..width as usize {
            let at = bit + i;
            code |= u16::from(stream[at >> 3] >> (at & 7) & 1) << i;
        }
        bit += width as usize;

        if code == LZW_CLEAR {
            next = LZW_FIRST_CODE;
            width = LZW_MIN_CODE_WIDTH;
            prev = None;
            continue;
        }
        if code == LZW_END {
            return Ok(out);
        }

        stack.clear();
        // The one code that is legal before it has been assigned: it can only be
        // the string just emitted followed by that string's own first byte.
        let mut walk = if code < next {
            code
        } else if code == next {
            stack.push(0); // placeholder for that first byte, patched below
            prev.ok_or(PicError::BadPixelData)?
        } else {
            return Err(PicError::BadPixelData);
        };
        while walk >= LZW_CLEAR {
            stack.push(suffix[usize::from(walk)]);
            walk = prefix[usize::from(walk)];
        }
        let first = walk as u8;
        stack.push(first);
        if code == next {
            stack[0] = first;
        }

        if out.len() + stack.len() > limit {
            return Err(PicError::BadPixelData);
        }
        out.extend(stack.iter().rev());

        if let Some(p) = prev {
            if usize::from(next) < LZW_MAX_CODES {
                prefix[usize::from(next)] = p;
                suffix[usize::from(next)] = first;
                next += 1;
                if usize::from(next) == 1 << width && width < LZW_MAX_CODE_WIDTH {
                    width += 1;
                }
            }
        }
        prev = Some(code);
    }
}

fn be16(b: &[u8], o: usize) -> u16 {
    u16::from(b[o]) << 8 | u16::from(b[o + 1])
}

fn le16(b: &[u8], o: usize) -> u16 {
    u16::from(b[o]) | u16::from(b[o + 1]) << 8
}

fn be24(b: &[u8], o: usize) -> usize {
    usize::from(b[o]) << 16 | usize::from(b[o + 1]) << 8 | usize::from(b[o + 2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A hand-built one-picture archive: 4x2, rows `2222` then `3333`.
    ///
    /// Pre-XOR the rows are `2222` and `1111`, which run-length encodes to the
    /// four symbols `[2, 18, 1, 18]` (18 = repeat 3 more). Huffman codes:
    /// `0` -> 2, `10` -> 1, `11` -> 18, so the bit stream is `0 11 10 11`,
    /// padded to `0b0111_0110`.
    fn synthetic() -> Vec<u8> {
        let mut f = vec![0u8; 16];
        f[0] = 1; // part
        f[2] = 0x00; // huffOff, in words: 0x0F * 2 = 30 = end of directory
        f[3] = 0x0F;
        f[5] = 1; // one picture
        f[8] = ENTRY_SIZE as u8;
        let data_off = 16 + ENTRY_SIZE + HUFF_LEN;
        f.extend_from_slice(&[
            0, 7, // id 7
            0, 4, // width
            0, 2, // height
            0, (EF_TRANS | EF_PHUFF) as u8,
            (data_off >> 16) as u8,
            (data_off >> 8) as u8,
            data_off as u8,
            0,
            0,
            0, // no palette
        ]);
        let mut tree = vec![0u8; HUFF_LEN];
        tree[0] = 128 + 2; // bit 0 -> leaf, symbol 2
        tree[1] = 1; // bit 1 -> internal node 1, i.e. index 2
        tree[2] = 128 + 1; // leaf, symbol 1
        tree[3] = 128 + 18; // leaf, symbol 18 (repeat 3 more)
        f.extend_from_slice(&tree);
        f.extend_from_slice(&[0, 0, 1]); // minSize
        f.extend_from_slice(&[0, 0, 4]); // midSize: four symbols
        f.push(0b0111_0110);
        f
    }

    /// A hand-built TWO-picture archive in the 16-byte flavour: header flags
    /// `HF_EHUFF` alone, so every record names its own Huffman tree and the
    /// header's global tree word is unused (zero, as it is in Shogun).
    ///
    /// The two trees deliberately decode the *same* bit patterns to different
    /// symbols, so reading either picture through the other's tree produces
    /// different pixels. Picture 7 is `synthetic`'s: tree A codes `0` -> 2,
    /// `10` -> 1, `11` -> 18, bit stream `0b0111_0110`. Picture 9 uses tree B —
    /// `1` -> 4, `00` -> 6, `01` -> 18 — over `[4, 18, 6, 18]`, bit stream
    /// `1 01 00 01` padded to `0b1010_0010`, which un-XORs to rows `4444`,
    /// `2222`.
    fn synthetic_per_entry_huff() -> Vec<u8> {
        const E: usize = ENTRY_SIZE + HUFF_PTR_SIZE;
        let mut f = vec![0u8; 16];
        f[0] = 1;
        f[1] = HF_EHUFF; // no HF_GHUFF: the trees live in the records
        f[5] = 2; // two pictures
        f[8] = E as u8;
        let dir_end = 16 + 2 * E;
        let (tree_a, tree_b) = (dir_end, dir_end + HUFF_LEN);
        let data_a = tree_b + HUFF_LEN;
        let data_b = data_a + 2 * LBYTES + 1;
        let mut record = |id: u16, data: usize, huff: usize| {
            f.extend_from_slice(&id.to_be_bytes());
            f.extend_from_slice(&[0, 4, 0, 2, 0, (EF_TRANS | EF_PHUFF) as u8]);
            f.extend_from_slice(&[(data >> 16) as u8, (data >> 8) as u8, data as u8]);
            f.extend_from_slice(&[0, 0, 0]); // no palette
            f.extend_from_slice(&u16::try_from(huff / 2).unwrap().to_be_bytes());
        };
        record(7, data_a, tree_a);
        record(9, data_b, tree_b);

        let mut a = vec![0u8; HUFF_LEN];
        a[0] = 128 + 2; // `0`  -> symbol 2
        a[1] = 1; // `1`  -> node 1
        a[2] = 128 + 1; // `10` -> symbol 1
        a[3] = 128 + 18; // `11` -> repeat 3 more
        let mut b = vec![0u8; HUFF_LEN];
        b[0] = 1; // `0`  -> node 1
        b[1] = 128 + 4; // `1`  -> symbol 4
        b[2] = 128 + 6; // `00` -> symbol 6
        b[3] = 128 + 18; // `01` -> repeat 3 more
        f.extend_from_slice(&a);
        f.extend_from_slice(&b);
        f.extend_from_slice(&[0, 0, 1, 0, 0, 4, 0b0111_0110]);
        f.extend_from_slice(&[0, 0, 1, 0, 0, 4, 0b1010_0010]);
        f
    }

    /// A hand-built MONOCHROME archive in the 12-byte flavour (SQ-0838), the
    /// shape the Macintosh's `Pic.data` has: header flags `0x0e` — `HF_EHUFF |
    /// HF_GHUFF` plus the `GF_MONO` bit — one global Huffman tree, and records
    /// that spend their twelfth byte on padding because there is no palette to
    /// point at.
    ///
    /// Its one picture is 4x2, rows `2222` then `3333`, encoded exactly as
    /// [`synthetic`]'s is: two colours out of the sixteen the codec can carry,
    /// which is the whole of what "monochrome" means to this format.
    fn synthetic_mono() -> Vec<u8> {
        const E: usize = ENTRY_SIZE_NO_PAL;
        let mut f = vec![0u8; 16];
        f[0] = 1; // part
        f[1] = HF_EHUFF | HF_GHUFF | EF_MONO as u8; // 0x0e, as Zork Zero's Mac disk reads
        let huff = 16 + E;
        f[2..4].copy_from_slice(&u16::try_from(huff / 2).unwrap().to_be_bytes()); // huffOff, in words
        f[5] = 1; // one picture
        f[8] = E as u8;
        let data_off = huff + HUFF_LEN;
        f.extend_from_slice(&[
            0, 7, // id 7
            0, 4, // width
            0, 2, // height
            0, (EF_MONO | EF_PHUFF) as u8,
            (data_off >> 16) as u8,
            (data_off >> 8) as u8,
            data_off as u8,
            0, // pad, not a palette pointer
        ]);
        let mut tree = vec![0u8; HUFF_LEN];
        tree[0] = 128 + 2; // `0`  -> symbol 2
        tree[1] = 1; // `1`  -> node 1
        tree[2] = 128 + 1; // `10` -> symbol 1
        tree[3] = 128 + 18; // `11` -> repeat 3 more
        f.extend_from_slice(&tree);
        f.extend_from_slice(&[0, 0, 1]); // minSize
        f.extend_from_slice(&[0, 0, 4]); // midSize: four symbols
        f.push(0b0111_0110);
        f
    }

    /// SQ-0838: the 12-byte monochrome flavour parses, and its pixels come out
    /// of the SAME codec the colour side uses.
    ///
    /// Both halves matter and they fail differently. Before this quest `parse`
    /// refused the container on its record size (`UnsupportedContainer`), and
    /// even with the size accepted `decode` refused every picture in it on
    /// `EF_MONO` (`UnsupportedCompression(0x0a)`) — the flag is set on every
    /// pixel-bearing record such an archive has.
    #[test]
    fn decodes_a_twelve_byte_monochrome_archive() {
        let pics = InfocomPics::parse(synthetic_mono()).unwrap();
        assert_eq!(pics.flavour(), Flavour::AmigaMac, "big-endian with a global tree, not PC");
        assert_eq!(pics.entries().len(), 1);

        let p = pics.decode(7).unwrap();
        assert_eq!((p.width, p.height), (4, 2));
        assert_eq!(p.indices, vec![2, 2, 2, 2, 3, 3, 3, 3], "one byte per pixel, not one bit");
        assert_eq!(p.transparent, None, "no `EF_TRANS` on this record");

        // A 12-byte record has nowhere to put a palette, so the picture is
        // adaptive and the archive answers with the two-colour hardware table.
        assert!(!pics.entry(7).unwrap().has_own_palette());
        assert!(p.palette.is_none());
        assert!(pics.is_monochrome());
        // …and it is the MACINTOSH's two-colour table, not the card's: the
        // container is what names the machine (SQ-0956).
        assert_eq!(pics.hardware_palette(), Some(MAC_MONO_PALETTE));

        // And the picture space is the standard Macintosh's, not the Amiga's —
        // even though bit 3 of `0x0e` is the same bit that means 640 on the PC.
        assert_eq!((pics.picture_space_width(), pics.picture_space_height()), (480, 300));
    }

    /// The twelfth byte of a mono record is padding, and reading it as the top
    /// of a palette pointer is the mistake the shared 14-byte layout invites.
    /// `ReadGFXEntry` skips it; so must this.
    #[test]
    fn a_monochrome_records_twelfth_byte_is_padding() {
        let mut f = synthetic_mono();
        f[16 + 11] = 0xff; // a byte no palette offset could survive
        let pics = InfocomPics::parse(f).expect("the pad byte is not read");
        assert!(!pics.entry(7).unwrap().has_own_palette());
        assert_eq!(pics.decode(7).unwrap().indices, vec![2, 2, 2, 2, 3, 3, 3, 3]);
    }

    /// SQ-0744. Shogun's Amiga archive declares `HF_EHUFF` without `HF_GHUFF`,
    /// which by `ReadGFXEntry` gives every picture its own Huffman tree in a
    /// 16-byte record. Both record sizes must read, and each picture must go
    /// through *its own* tree — decoding picture 9 with picture 7's would give
    /// `[2, 2, 2, 2, 3, 3, 3, 3]` here.
    #[test]
    fn decodes_a_sixteen_byte_archive_through_per_entry_huffman_trees() {
        let pics = InfocomPics::parse(synthetic_per_entry_huff()).unwrap();
        assert_eq!(pics.entries().len(), 2);

        let p = pics.decode(7).unwrap();
        assert_eq!(p.indices, vec![2, 2, 2, 2, 3, 3, 3, 3]);
        let q = pics.decode(9).unwrap();
        assert_eq!(q.indices, vec![4, 4, 4, 4, 2, 2, 2, 2]);
    }

    /// An Amiga/Mac record size must be the one its header flags imply: 16
    /// bytes exactly when `HF_EHUFF` stands alone, 14 otherwise. Neither size is
    /// accepted on its own say-so.
    #[test]
    fn the_record_size_must_match_the_header_flags() {
        let mut f = synthetic_per_entry_huff();
        f[8] = ENTRY_SIZE as u8;
        assert_eq!(InfocomPics::parse(f).err(), Some(PicError::UnsupportedContainer));

        let mut f = synthetic();
        f[8] = (ENTRY_SIZE + HUFF_PTR_SIZE) as u8;
        assert_eq!(InfocomPics::parse(f).err(), Some(PicError::UnsupportedContainer));

        // `HF_EHUFF | HF_GHUFF` — Zork Zero's 6 — is the global-tree flavour and
        // stays 14 bytes, so a header that sets both must not grow its records.
        let mut f = synthetic();
        f[1] = HF_EHUFF | HF_GHUFF;
        assert!(InfocomPics::parse(f).is_ok());
    }

    /// `ReadGFXEntry` binary-searches the directory, so a real archive's ids
    /// ascend. A container whose "ids" wander is not one of these files.
    #[test]
    fn rejects_a_directory_whose_ids_do_not_ascend() {
        let mut f = synthetic_per_entry_huff();
        f[16] = 0;
        f[17] = 9; // both records now claim id 9
        assert_eq!(InfocomPics::parse(f).err(), Some(PicError::UnsupportedContainer));
    }

    #[test]
    fn decodes_a_synthetic_picture() {
        let pics = InfocomPics::parse(synthetic()).unwrap();
        assert_eq!(pics.part(), 1);
        assert_eq!(pics.entries().len(), 1);
        let p = pics.decode(7).unwrap();
        assert_eq!((p.width, p.height), (4, 2));
        assert_eq!(p.indices, vec![2, 2, 2, 2, 3, 3, 3, 3]);
        assert_eq!(p.transparent, Some(0));
        assert!(p.palette.is_none());
        // No palette of its own, so `rgba` reads through the default table.
        let c = DEFAULT_PALETTE[2];
        assert_eq!(&p.rgba()[..4], &[c[0], c[1], c[2], 255]);
    }

    #[test]
    fn a_zero_palette_offset_marks_a_picture_adaptive() {
        // The synthetic archive's one picture names no palette, so it is
        // adaptive: it has no colours of its own and must be drawn through
        // whatever the caller supplies.
        let pics = InfocomPics::parse(synthetic()).unwrap();
        assert!(!pics.entry(7).unwrap().has_own_palette());
        assert_eq!(pics.adaptive_pictures(), vec![7]);
        assert_eq!(pics.palette_of(7), None, "an adaptive picture has no palette to hand out");

        let p = pics.decode(7).unwrap();
        let mut current = DEFAULT_PALETTE;
        current[2] = [1, 2, 3];
        assert_eq!(&p.rgba_with(&current)[..4], &[1, 2, 3, 255], "drawn through the supplied palette");
        assert_eq!(&p.rgba()[..4], &[0, 170, 0, 255], "and through the fallback without one");

        // A picture that DOES name a palette is not adaptive, and answers with
        // it — the per-picture distinction the format makes.
        let mut f = synthetic();
        let pal_off = f.len();
        f[16 + 11] = (pal_off >> 16) as u8;
        f[16 + 12] = (pal_off >> 8) as u8;
        f[16 + 13] = pal_off as u8;
        f.extend_from_slice(&[1, 9, 8, 7]); // one colour, landing at index 2
        let pics = InfocomPics::parse(f).unwrap();
        assert!(pics.entry(7).unwrap().has_own_palette());
        assert!(pics.adaptive_pictures().is_empty());
        assert_eq!(pics.palette_of(7).unwrap()[2], [9, 8, 7]);
    }

    #[test]
    fn a_placeholder_is_not_adaptive() {
        // No pixels means nothing to colour: a size-only entry is a rectangle
        // the game fills, not an adaptive picture.
        let mut f = synthetic();
        f[16 + 8] = 0;
        f[16 + 9] = 0;
        f[16 + 10] = 0;
        let pics = InfocomPics::parse(f).unwrap();
        assert!(!pics.entry(7).unwrap().has_own_palette());
        assert!(pics.adaptive_pictures().is_empty());
    }

    #[test]
    fn rejects_containers_it_cannot_read() {
        let mut f = synthetic();
        f[8] = 8; // the Apple II record size, which this module does not read
        assert_eq!(
            InfocomPics::parse(f).err(),
            Some(PicError::UnsupportedContainer)
        );
        assert_eq!(
            InfocomPics::parse(vec![0; 4]).err(),
            Some(PicError::Truncated)
        );
    }

    #[test]
    fn reports_missing_and_placeholder_pictures() {
        let pics = InfocomPics::parse(synthetic()).unwrap();
        assert_eq!(pics.decode(99), Err(PicError::NoSuchPicture));

        let mut f = synthetic();
        f[16 + 8] = 0; // clear the data offset: a size-only placeholder
        f[16 + 9] = 0;
        f[16 + 10] = 0;
        let pics = InfocomPics::parse(f).unwrap();
        assert!(!pics.entry(7).unwrap().has_pixels());
        assert_eq!(pics.decode(7), Err(PicError::NoPixelData));
    }

    /// One hand-built PC picture: what to put in the directory, and the LZW
    /// codes whose expansion is its pixels.
    struct PcPic {
        id: u16,
        w: u16,
        h: u16,
        flags: u16,
        codes: Vec<u16>,
        palette: Vec<u8>,
    }

    /// Pack LZW codes least-significant bit first at the starting width of 9.
    ///
    /// Deliberately *not* the decoder's bit reader run backwards: this is an
    /// independent encoder, and it stays at 9 bits, which every stream here is
    /// short enough to earn (the table would have to reach 512 to widen).
    fn pc_pack(codes: &[u16]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut acc = 0u32;
        let mut held = 0u32;
        for &c in codes {
            acc |= u32::from(c) << held;
            held += 9;
            while held >= 8 {
                out.push(acc as u8);
                acc >>= 8;
                held -= 8;
            }
        }
        if held > 0 {
            out.push(acc as u8);
        }
        out
    }

    /// Assemble a PC archive around those pictures. Records are 12 bytes (the
    /// EGA/CGA shape, a pad byte where the palette pointer would be) unless some
    /// picture carries a palette, in which case they are 14.
    fn pc_archive(pics: &[PcPic]) -> Vec<u8> {
        let paletted = pics.iter().any(|p| !p.palette.is_empty());
        let entry_size = if paletted {
            PC_ENTRY_SIZE_PAL
        } else {
            ENTRY_SIZE_NO_PAL
        };
        let dir_end = 16 + pics.len() * entry_size;
        let mut dir = Vec::new();
        let mut body: Vec<u8> = Vec::new();
        let off3 = |o: usize| [(o >> 16) as u8, (o >> 8) as u8, o as u8];
        for p in pics {
            let pal_off = if p.palette.is_empty() {
                0
            } else {
                let o = dir_end + body.len();
                body.extend_from_slice(&p.palette);
                o
            };
            let data_off = dir_end + body.len();
            body.extend_from_slice(&pc_pack(&p.codes));
            dir.extend_from_slice(&p.id.to_le_bytes());
            dir.extend_from_slice(&p.w.to_le_bytes());
            dir.extend_from_slice(&p.h.to_le_bytes());
            dir.extend_from_slice(&p.flags.to_le_bytes());
            dir.extend_from_slice(&off3(data_off));
            if paletted {
                dir.extend_from_slice(&off3(pal_off));
            } else {
                dir.push(0); // `mac/gfx.p`'s "single pad byte"
            }
        }
        let mut f = vec![0u8; 16];
        f[0] = 1; // part
        f[1] = 0x30; // neither HF_EHUFF nor HF_GHUFF: no Huffman tree anywhere
        f[4..6].copy_from_slice(&u16::try_from(pics.len()).unwrap().to_le_bytes());
        f[8] = entry_size as u8;
        f[12] = 1; // version
        f.extend_from_slice(&dir);
        f.extend_from_slice(&body);
        f
    }

    /// SQ-0735. The PC flavour, end to end on a file with no Infocom media in
    /// it: little-endian header and directory, MSB-first 3-byte offsets, and one
    /// bare LZW stream per picture that expands straight to pixels — no
    /// run-length stage and no XOR.
    #[test]
    fn decodes_a_synthetic_pc_picture() {
        let f = pc_archive(&[PcPic {
            id: 7,
            w: 4,
            h: 2,
            flags: EF_TRANS,
            // clear, then eight literals, then end-of-stream.
            codes: vec![LZW_CLEAR, 2, 2, 2, 2, 3, 3, 3, 3, LZW_END],
            palette: vec![],
        }]);
        let pics = InfocomPics::parse(f).unwrap();
        assert_eq!(pics.flavour(), Flavour::Pc);
        assert_eq!(pics.part(), 1);
        assert_eq!(pics.entries().len(), 1);

        let p = pics.decode(7).unwrap();
        assert_eq!((p.width, p.height), (4, 2));
        assert_eq!(p.indices, vec![2, 2, 2, 2, 3, 3, 3, 3]);
        assert_eq!(p.transparent, Some(0));
        // 12-byte records have nowhere to keep a palette, so every picture in an
        // EGA/CGA archive is adaptive.
        assert!(p.palette.is_none());
        assert_eq!(pics.adaptive_pictures(), vec![7]);
    }

    /// SQ-0801. The flag word says two things and EGA is the only rendition that
    /// uses either of them: **which** colour drops out, and whether one drops out
    /// at all.
    ///
    /// Every Amiga, MCGA and CGA archive in hand sets `EF_TRANS` on every picture
    /// and leaves the top nibble zero, so "the transparent colour is 0" passes
    /// there whether it is read or assumed. `zork0.eg1` does neither: 132 of its
    /// 396 pictures have `flags = 0` and are wholly opaque, and 128 of the rest
    /// name colour 1, 2 or 3. Both arms are load-bearing — an opaque picture that
    /// dropped colour 0 would show holes, and a picture transparent on 1 that was
    /// treated as transparent on 0 would paint its background over whatever it
    /// was placed on. Infocom's own YZIP says exactly this (`apple/yzip/pic.asm`:
    /// bit 0 clear → `TRANSCLR = $FF`, i.e. no colour matches; else the flag word
    /// shifted right four bits), and Frotz's `dos/bcpic.c` repeats it.
    #[test]
    fn a_pc_picture_names_its_transparent_colour_or_has_none() {
        let f = pc_archive(&[
            PcPic {
                id: 1,
                w: 4,
                h: 1,
                // EGA's shape: EF_TRANS, and colour 1 in the top nibble.
                flags: 0x1001,
                codes: vec![LZW_CLEAR, 0, 1, 2, 1, LZW_END],
                palette: vec![],
            },
            PcPic {
                id: 2,
                w: 4,
                h: 1,
                // …and the deliberately opaque one, which is the same 132 pictures
                // the card ranks and suits come from.
                flags: 0,
                codes: vec![LZW_CLEAR, 0, 1, 2, 1, LZW_END],
                palette: vec![],
            },
        ]);
        let pics = InfocomPics::parse(f).unwrap();

        let named = pics.decode(1).unwrap();
        assert_eq!(named.transparent, Some(1), "the top nibble names the colour that drops out");
        let pal = [[9u8, 9, 9]; 16];
        assert_eq!(
            named.rgba_with(&pal).as_chunks::<4>().0.iter().map(|p| p[3]).collect::<Vec<_>>(),
            vec![255, 0, 255, 0],
            "colour 1 and only colour 1 reaches the host as alpha 0"
        );

        let opaque = pics.decode(2).unwrap();
        assert_eq!(opaque.transparent, None, "bit 0 clear is no transparent colour at all");
        assert!(
            opaque.rgba_with(&pal).as_chunks::<4>().0.iter().all(|p| p[3] == 255),
            "an opaque picture paints its whole rectangle, colour 0 included"
        );
    }

    /// The LZW table itself: a code that stands for a string the stream taught
    /// the decoder, and the self-referential code that is legal one step before
    /// it is defined. Literals alone would leave both untested.
    #[test]
    fn expands_learned_and_self_referential_lzw_codes() {
        // 258 is assigned "1 2" by the pair before it, then used.
        let f = pc_archive(&[PcPic {
            id: 1,
            w: 4,
            h: 1,
            flags: 0,
            codes: vec![LZW_CLEAR, 1, 2, LZW_FIRST_CODE, LZW_END],
            palette: vec![],
        }]);
        let pics = InfocomPics::parse(f).unwrap();
        assert_eq!(pics.decode(1).unwrap().indices, vec![1, 2, 1, 2]);

        // 258 used in the very code that defines it: it can only mean the last
        // string plus that string's own first byte.
        let f = pc_archive(&[PcPic {
            id: 1,
            w: 3,
            h: 1,
            flags: 0,
            codes: vec![LZW_CLEAR, 5, LZW_FIRST_CODE, LZW_END],
            palette: vec![],
        }]);
        let pics = InfocomPics::parse(f).unwrap();
        assert_eq!(pics.decode(1).unwrap().indices, vec![5, 5, 5]);

        // A code that is neither known nor the next one to be assigned is not a
        // picture at all.
        let f = pc_archive(&[PcPic {
            id: 1,
            w: 3,
            h: 1,
            flags: 0,
            codes: vec![LZW_CLEAR, 5, LZW_FIRST_CODE + 1, LZW_END],
            palette: vec![],
        }]);
        let pics = InfocomPics::parse(f).unwrap();
        assert_eq!(pics.decode(1), Err(PicError::BadPixelData));
    }

    /// `EF_MONO` — "two-color picture" — is one BIT per pixel with rows padded
    /// out to whole bytes, and the high bit of each byte is the leftmost pixel.
    /// Twelve pixels wide exercises the padding: two bytes a row, four bits of
    /// each second byte thrown away.
    #[test]
    fn expands_a_one_bit_pc_picture() {
        let f = pc_archive(&[PcPic {
            id: 3,
            w: 12,
            h: 2,
            flags: EF_MONO,
            codes: vec![LZW_CLEAR, 0xA0, 0xF0, 0x0F, 0x00, LZW_END],
            palette: vec![],
        }]);
        let pics = InfocomPics::parse(f).unwrap();
        let p = pics.decode(3).unwrap();
        assert_eq!(p.indices.len(), 24);
        #[rustfmt::skip]
        assert_eq!(
            p.indices,
            vec![
                1, 0, 1, 0, 0, 0, 0, 0, 1, 1, 1, 1,
                0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0,
            ]
        );
        assert_eq!(p.transparent, None, "EF_MONO alone claims no transparent colour");
    }

    /// A 14-byte PC record keeps a palette pointer where the 12-byte one keeps a
    /// pad byte, and the palette is laid out exactly as the Amiga's: a count,
    /// that many RGB triples starting at colour index 2, then the stipple table.
    /// MCGA archives are the ones that use it.
    #[test]
    fn a_pc_archive_can_carry_palettes() {
        let f = pc_archive(&[PcPic {
            id: 4,
            w: 2,
            h: 1,
            flags: EF_TRANS,
            codes: vec![LZW_CLEAR, 2, 3, LZW_END],
            palette: vec![2, 9, 8, 7, 6, 5, 4, 0],
        }]);
        assert_eq!(usize::from(f[8]), PC_ENTRY_SIZE_PAL);
        let pics = InfocomPics::parse(f).unwrap();
        assert!(pics.entry(4).unwrap().has_own_palette());
        assert!(pics.adaptive_pictures().is_empty());

        let p = pics.decode(4).unwrap();
        let pal = p.palette.expect("a 14-byte record names a palette");
        assert_eq!(pal[2], [9, 8, 7]);
        assert_eq!(pal[3], [6, 5, 4]);
        assert_eq!(pal[4], DEFAULT_PALETTE[4], "the count stops at two colours");
        assert_eq!(&p.rgba()[..8], &[9, 8, 7, 255, 6, 5, 4, 255]);
    }

    /// SQ-0794. The one entry that separates the IBM EGA's default table from
    /// [`DEFAULT_PALETTE`], stated as a difference so the table cannot be
    /// "corrected" back into the arithmetic value without this failing.
    ///
    /// Index 6 is brown, `(170, 85, 0)`, not the dark yellow `(170, 170, 0)`
    /// that two thirds of green would give. Every other entry is the plain
    /// 0/85/170/255 arithmetic. Sources are named on [`EGA_PALETTE`]; the two
    /// that state RGB directly (bocfel's `ega_colormap`, ModdingWiki's *EGA
    /// Palette*, int10h.org's canonical sixteen) agree entry for entry, and the
    /// only dissent on record is the one bocfel itself flags — pix2gif, which
    /// carries the arithmetic value.
    #[test]
    fn the_ega_table_is_the_default_one_with_brown_at_index_six() {
        assert_eq!(EGA_PALETTE[6], [170, 85, 0], "EGA index 6 is brown");
        assert_eq!(DEFAULT_PALETTE[6], [170, 170, 0], "…and the fallback's is dark yellow");
        for i in 0..16 {
            if i == 6 {
                continue;
            }
            assert_eq!(EGA_PALETTE[i], DEFAULT_PALETTE[i], "entry {i} is common to both");
        }
        // The encoding the other fifteen come out of: two bits per channel,
        // times 85. Index 6 is the hardware's exception to it.
        for (i, c) in EGA_PALETTE.iter().enumerate() {
            for ch in c {
                assert!(
                    matches!(ch, 0 | 85 | 170 | 255),
                    "entry {i} channel {ch} is one of the four EGA levels"
                );
            }
        }
    }

    /// SQ-0794. A CGA rendition is two colours, because the only 640-wide mode
    /// the IBM CGA had was 640x200 mode 6 — and both witnesses read the archive's
    /// indices the same way: 2 is the lit state and 3 is black, with slot 1
    /// standing in for a set bit in the packed form.
    ///
    /// **SQ-0956 moved the lit state**, and this is the pin: `#AAAAAA`, the card's
    /// light grey, not pure white. `machine-screenshots/dos-zorkzero-cga.png`
    /// measures `#A0A0A0` for every lit pixel it has, artwork and text alike — a
    /// DOS emulator's rendering of EGA entry 7, which is the value
    /// [`EGA_PALETTE`] already carries at that index and the value
    /// `dos-hitchhiker.png` measures for its ink on the same emulator family.
    #[test]
    fn the_cga_table_is_the_cards_two_states() {
        assert_eq!(CGA_PALETTE[0], [0, 0, 0]);
        assert_eq!(CGA_PALETTE[1], [170, 170, 170], "a set bit in the packed form");
        assert_eq!(CGA_PALETTE[2], [170, 170, 170], "the archive's white, as the card showed it");
        assert_eq!(CGA_PALETTE[3], [0, 0, 0], "the archive's black");
        assert_eq!(CGA_PALETTE[1], EGA_PALETTE[7], "and it is EGA entry 7, stated once");
        for (i, c) in CGA_PALETTE.iter().enumerate().skip(4) {
            assert_eq!(*c, [0, 0, 0], "entry {i} is unused and stays black");
        }
    }

    /// SQ-0956. The Macintosh's mono plate is the OTHER two-colour table, and the
    /// reason there are two: `machine-screenshots/mac-zorkzero-game.png` is 77.2%
    /// `#FFFFFF` against 22.8% `#000000` inside the game window — a 1-bit capture,
    /// so those are the only two values in it — with the pillars dithered black on
    /// that white. Its white is a real white and the card's is not.
    ///
    /// The two tables must stay interchangeable at every index a caller can index,
    /// because `hardware_palette` hands one or the other to the same decoder.
    #[test]
    fn the_mac_mono_table_keeps_a_real_white() {
        assert_eq!(MAC_MONO_PALETTE[2], [255, 255, 255], "the Mac's page");
        assert_eq!(MAC_MONO_PALETTE[3], [0, 0, 0], "under its ink");
        assert_ne!(
            MAC_MONO_PALETTE[2], CGA_PALETTE[2],
            "the whole point: one EF_MONO flag, two machines, two lit states",
        );
        assert_eq!(MAC_MONO_PALETTE.len(), CGA_PALETTE.len());
        for (i, c) in MAC_MONO_PALETTE.iter().enumerate().skip(4) {
            assert_eq!(*c, [0, 0, 0], "entry {i} is unused and stays black");
        }
    }

    /// SQ-0794. Which hardware table an archive's pixels resolve through, read
    /// off the directory rather than the filename.
    ///
    /// Falsified by returning `EGA_PALETTE` unconditionally from
    /// `hardware_palette`: the CGA case then reports white where it must report
    /// green, which is precisely the defect on `zork0.cg1`.
    #[test]
    fn a_palette_less_pc_archive_names_its_hardware_table() {
        let ega = || PcPic {
            id: 1,
            w: 4,
            h: 1,
            flags: EF_TRANS,
            codes: vec![LZW_CLEAR, 6, 6, 12, 12, LZW_END],
            palette: vec![],
        };
        let pics = InfocomPics::parse(pc_archive(&[ega()])).unwrap();
        assert_eq!(pics.hardware_palette(), Some(EGA_PALETTE));
        // …and the picture itself still declares no palette of its own, because
        // its record genuinely has nowhere to keep one.
        assert!(pics.decode(1).unwrap().palette.is_none());
        assert_eq!(
            &pics.decode(1).unwrap().rgba_with(&EGA_PALETTE)[..8],
            &[170, 85, 0, 255, 170, 85, 0, 255],
            "index 6 comes out brown through the hardware table"
        );

        // Every pixel-bearing picture two-colour ⇒ CGA. Both of the format's CGA
        // packings, since a real `.CG1` mixes them.
        let cga = [
            PcPic { id: 1, w: 4, h: 1, flags: EF_MONO | EF_TRANS, codes: vec![LZW_CLEAR, 2, 3, 2, 3, LZW_END], palette: vec![] },
            PcPic { id: 2, w: 8, h: 1, flags: EF_MONO, codes: vec![LZW_CLEAR, 0b1010_1010, LZW_END], palette: vec![] },
        ];
        let pics = InfocomPics::parse(pc_archive(&cga)).unwrap();
        assert_eq!(pics.hardware_palette(), Some(CGA_PALETTE));
        assert_eq!(pics.decode(1).unwrap().indices, vec![2, 3, 2, 3]);
        assert_eq!(pics.decode(2).unwrap().indices, vec![1, 0, 1, 0, 1, 0, 1, 0]);

        // One picture that is not two-colour is enough to make the archive EGA:
        // no 16-colour rendition has a use for `EF_MONO`, so "all of them" is the
        // claim, not "any of them".
        let mixed = [
            PcPic { id: 1, w: 8, h: 1, flags: EF_MONO, codes: vec![LZW_CLEAR, 0xFF, LZW_END], palette: vec![] },
            PcPic { id: 2, w: 2, h: 1, flags: 0, codes: vec![LZW_CLEAR, 9, 10, LZW_END], palette: vec![] },
        ];
        assert_eq!(
            InfocomPics::parse(pc_archive(&mixed)).unwrap().hardware_palette(),
            Some(EGA_PALETTE)
        );

        // An archive whose pictures carry palettes fixes nothing in hardware —
        // MCGA and the Amiga/Mac. `any`, not `all`: an MCGA archive mixes
        // paletted pictures with genuinely adaptive ones.
        let mcga = [
            PcPic { id: 1, w: 2, h: 1, flags: EF_TRANS, codes: vec![LZW_CLEAR, 2, 3, LZW_END], palette: vec![2, 9, 8, 7, 6, 5, 4, 0] },
            PcPic { id: 2, w: 2, h: 1, flags: EF_TRANS, codes: vec![LZW_CLEAR, 2, 3, LZW_END], palette: vec![] },
        ];
        let pics = InfocomPics::parse(pc_archive(&mcga)).unwrap();
        assert_eq!(pics.hardware_palette(), None);
        assert_eq!(pics.adaptive_pictures(), vec![2], "the palette-less one is still adaptive");
    }

    /// A stream that keeps going past `width * height` is malformed, not big.
    #[test]
    fn rejects_a_pc_stream_that_overruns_its_picture() {
        let f = pc_archive(&[PcPic {
            id: 1,
            w: 2,
            h: 1,
            flags: 0,
            codes: vec![LZW_CLEAR, 1, 2, 3, 4, LZW_END],
            palette: vec![],
        }]);
        let pics = InfocomPics::parse(f).unwrap();
        assert_eq!(pics.decode(1), Err(PicError::BadPixelData));
    }

    /// Structural gates on the PC directory: ids ascend (the interpreter binary
    /// -searches it), and nothing may claim a Huffman tree in a file that names
    /// none.
    #[test]
    fn rejects_malformed_pc_directories() {
        let two = || {
            [1u16, 2].map(|id| PcPic {
                id,
                w: 2,
                h: 1,
                flags: 0,
                codes: vec![LZW_CLEAR, 1, 2, LZW_END],
                palette: vec![],
            })
        };
        assert!(InfocomPics::parse(pc_archive(&two())).is_ok());

        let mut f = pc_archive(&two());
        f[16 + ENTRY_SIZE_NO_PAL] = 1; // the second record now claims id 1 as well
        assert_eq!(
            InfocomPics::parse(f).err(),
            Some(PicError::UnsupportedContainer)
        );

        let mut f = pc_archive(&two());
        f[16 + 6] = EF_PHUFF as u8; // Huffman-coded, with no tree in the file
        assert_eq!(
            InfocomPics::parse(f).err(),
            Some(PicError::UnsupportedContainer)
        );

        let mut f = pc_archive(&two());
        f[8] = 16; // a record size no PC archive uses
        assert_eq!(
            InfocomPics::parse(f).err(),
            Some(PicError::UnsupportedContainer)
        );
    }

    /// The flavour is decided by whether the file names a Huffman tree, not by
    /// its record size — MCGA archives use the Amiga's 14 — and not by the flag
    /// byte, which reads `0x30` on four MCGA archives and `0x38` on a fifth.
    #[test]
    fn the_flavour_is_decided_by_whether_a_huffman_tree_is_named() {
        let mut f = pc_archive(&[PcPic {
            id: 1,
            w: 2,
            h: 1,
            flags: 0,
            codes: vec![LZW_CLEAR, 1, 2, LZW_END],
            palette: vec![1, 9, 8, 7, 0], // forces the 14-byte record
        }]);
        assert_eq!(f[8], PC_ENTRY_SIZE_PAL as u8, "the Amiga's record size");
        assert_eq!(InfocomPics::parse(f.clone()).unwrap().flavour(), Flavour::Pc);

        f[1] = 0x38;
        assert_eq!(
            InfocomPics::parse(f).unwrap().flavour(),
            Flavour::Pc,
            "the flag byte does not decide it — real MCGA archives read both"
        );

        // The Amiga's own archives, which do name a tree, keep reading as the
        // Amiga's.
        assert_eq!(
            InfocomPics::parse(synthetic()).unwrap().flavour(),
            Flavour::AmigaMac
        );
        assert_eq!(
            InfocomPics::parse(synthetic_per_entry_huff()).unwrap().flavour(),
            Flavour::AmigaMac
        );
    }

    fn zork0_pic() -> Option<Vec<u8>> {
        let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/zork0.pic");
        std::fs::read(p).ok()
    }

    /// Real-media smoke: the Amiga Zork Zero archive. `stories/` is gitignored,
    /// so this skips vacuously when the fixture is absent.
    #[test]
    fn reads_amiga_zork_zero() {
        let Some(bytes) = zork0_pic() else { return };
        let pics = InfocomPics::parse(bytes).unwrap();
        assert_eq!(pics.part(), 1);
        assert_eq!(pics.entries().len(), 495);

        let with_pixels: Vec<_> = pics
            .entries()
            .iter()
            .filter(|e| e.has_pixels())
            .copied()
            .collect();
        assert_eq!(with_pixels.len(), 388);
        // Everything else is a size-only placeholder, matching the 107 `Rect`
        // resources in the Blorb conversion exactly.
        assert_eq!(pics.entries().len() - with_pixels.len(), 107);

        // Every picture with pixels must decode to exactly its stated size, and
        // to exactly the bytes the decoder was validated against: 383 of these
        // 388 were compared pixel-for-pixel with the PNGs in the Blorb
        // conversion `Zork0.blb` and matched byte-for-byte. (The other five —
        // ids 5, 6, 7, 8 and 33 — are the ones whose two sources disagree on
        // dimensions; the Blorb holds a cropped or substituted version.) This
        // FNV-1a hash over the decoded indices of all 388, in file order, pins
        // that result without needing a PNG decoder here.
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for e in &with_pixels {
            let p = pics.decode(e.id).unwrap();
            assert_eq!(
                p.indices.len(),
                usize::from(e.width) * usize::from(e.height),
                "picture {} decoded to the wrong size",
                e.id
            );
            assert!(p.indices.iter().all(|&i| i < 16));
            for &i in &p.indices {
                hash = (hash ^ u64::from(i)).wrapping_mul(0x100_0000_01b3);
            }
        }
        assert_eq!(hash, 0xde67_de1e_0b55_f2fb);

        // 172 of those 388 name no palette at all and are therefore adaptive —
        // among them the 16 compass overlays, ids 9..=24. That set is, id for
        // id, the 172 numbers `Zork0.blb` lists in its `APal` chunk.
        let adaptive = pics.adaptive_pictures();
        assert_eq!(adaptive.len(), 172);
        assert!((9..=24).all(|id| adaptive.contains(&id)), "the compass overlays are adaptive");
        assert_eq!(pics.palette_of(10), None, "an adaptive picture has no palette of its own");

        // Picture 1 is the 320x200 title screen, on a 14-colour palette whose
        // first entry lands at colour index 2.
        let p = pics.decode(1).unwrap();
        assert_eq!((p.width, p.height), (320, 200));
        let pal = p.palette.expect("picture 1 carries a palette");
        assert_eq!(pal[2], [0xcc, 0xcc, 0xcc]);
        assert_eq!(pal[15], [0xee, 0xee, 0xee]);
        assert_eq!(p.rgba().len(), 320 * 200 * 4);
    }

    /// A file out of the gitignored `stories/`, or `None` with a SKIP note.
    fn fixture(name: &str) -> Option<Vec<u8>> {
        let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories")
            .join(name);
        match std::fs::read(&p) {
            Ok(b) => Some(b),
            Err(_) => {
                eprintln!("SKIP: gitignored fixture missing at {}", p.display());
                None
            }
        }
    }

    /// FNV-1a over every decoded picture's indices, in directory order.
    fn pixel_fingerprint(pics: &InfocomPics) -> (usize, usize, u64) {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut n = 0;
        for e in pics.entries() {
            if !e.has_pixels() {
                continue;
            }
            n += 1;
            let p = pics.decode(e.id).expect("a picture with pixels decodes");
            assert_eq!(
                p.indices.len(),
                usize::from(e.width) * usize::from(e.height),
                "picture {} decoded to the wrong size",
                e.id
            );
            for &i in &p.indices {
                h = (h ^ u64::from(i)).wrapping_mul(0x100_0000_01b3);
            }
        }
        (pics.entries().len(), n, h)
    }

    /// SQ-0735, real media: every PC archive on hand.
    ///
    /// All but the last row is original DOS media for four Infocom titles.
    /// Two files on hand are deliberately absent: `FMVPOKER.EG1` and
    /// `zorkzero.mg1` are byte-identical to `zork0.eg1` and `zork0.mg1` (the
    /// first because fmvpoker's readme tells players to rename the Zork Zero
    /// file), so they are copies rather than specimens.
    ///
    /// `beyondzo.mg1` is **not** Infocom media: it is the Atari ST release's
    /// title picture converted into the MCGA container by Stefan Jokisch, whose
    /// `bcpic.c` is one of this decoder's two witnesses. Beyond Zork's own DOS
    /// release shipped no artwork. It is kept as a fixture precisely because it
    /// came out of a different encoder, and it earns its keep: it decodes on the
    /// same rules as the authentic files. Nothing about the format is inferred
    /// from it — its header even carries the 640-wide flag byte (`0x38`) on a
    /// 320-wide archive, which no authentic `.MG1` does.
    ///
    /// The fingerprints are regression pins, not the proof of correctness; that
    /// is [`mcga_zork_zero_decodes_to_the_amiga_pixels`]. Each was computed by a
    /// separately written decoder before this one existed and matched it.
    ///
    /// [`mcga_zork_zero_decodes_to_the_amiga_pixels`]: mcga_zork_zero_decodes_to_the_amiga_pixels
    #[test]
    fn reads_the_pc_archives() {
        #[rustfmt::skip]
        let corpus: [(&str, u8, usize, usize, usize, u64); 15] = [
            // file             part  records  pixels  colours  fingerprint
            ("zork0.mg1",          1,     503,    396,      16, 0x2c3c_51eb_c815_51c8),
            ("zork0.eg1",          1,     503,    396,      16, 0x32d3_69e3_066a_58d6),
            ("zork0.cg1",          1,     503,    396,       4, 0x8de5_413b_5636_0cf8),
            ("arthur.mg1",         1,     171,    137,      16, 0x1e5f_de1a_8541_fe0e),
            ("arthur.eg1",         1,     125,     97,      16, 0xc5c5_4b8a_fbb7_1762),
            ("arthur.eg2",         2,     101,     77,      16, 0xdea4_ab1d_39c8_d0a1),
            ("arthur.cg1",         1,     170,    136,       4, 0x1c9a_843e_e318_15dc),
            ("journey.mg1",        1,     134,    134,      16, 0x17d0_5716_509d_75e0),
            ("journey.eg1",        1,      80,     80,      16, 0xd287_b2f3_5e8f_0f7a),
            ("journey.eg2",        2,      67,     67,      16, 0xa3cb_ba73_0ba7_92a5),
            ("journey.cg1",        1,     134,    134,       4, 0x2769_2a5a_147c_588e),
            ("shogun.mg1",         1,      48,     42,      16, 0x04f3_726e_9f03_3aaa),
            ("shogun.eg1",         1,      50,     44,      16, 0x9cf4_195e_0590_bbe7),
            ("shogun.cg1",         1,      50,     44,       4, 0x0e1b_af9a_9e34_93cb),
            // A third-party conversion, segregated on purpose. See above.
            ("beyondzo.mg1",       1,       1,      1,      16, 0x3b66_df38_f6d2_1e66),
        ];
        for (name, part, records, pixels, colours, want) in corpus {
            let Some(bytes) = fixture(name) else { continue };
            let pics = InfocomPics::parse(bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(pics.flavour(), Flavour::Pc, "{name}");
            assert_eq!(pics.part(), part, "{name} part number");
            assert_eq!(
                pixel_fingerprint(&pics),
                (records, pixels, want),
                "{name} decoded differently"
            );
            // Nothing may index past the palette it will be drawn through, and
            // the depth each rendition was drawn for shows in the indices: CGA
            // never exceeds 3, EGA and MCGA never exceed 15.
            for e in pics.entries().iter().filter(|e| e.has_pixels()) {
                let p = pics.decode(e.id).unwrap();
                assert!(
                    p.indices.iter().all(|&i| usize::from(i) < colours),
                    "{name} picture {} indexes past {colours} colours",
                    e.id
                );
            }
        }
    }

    /// SQ-0798: `.EG1` and `.EG2` are one archive in two files, and merged they
    /// are exactly the MCGA set.
    ///
    /// This is the measurement the whole fix rests on. EGA art is 640 wide and
    /// would not fit one 360K floppy, so Arthur's and Journey's EGA renditions
    /// were split — and the split overlaps, because each disk had to stand alone
    /// for its stretch of the game. Reading part 1 only is not "most of the art";
    /// it is 97 of Arthur's 137 pictures and 80 of Journey's 135.
    ///
    /// The union is checked against the **MCGA** archive of the same game rather
    /// than against a number typed here, because `arthur.mg1` is the same
    /// artwork shipped undivided: if the merge were wrong in either direction —
    /// dropping a picture or inventing one — the two directories would stop
    /// agreeing. Journey is the one deliberate exception and it is asserted as
    /// such below.
    #[test]
    fn a_two_part_ega_set_merges_into_exactly_the_mcga_set() {
        for (game, want_ids, want_pixels, extra_ega_ids) in [
            ("arthur", 171usize, 137usize, &[][..]),
            // Journey's EGA release carries one picture MCGA does not: id 59, a
            // 220x126 rectangle of solid colour index 0 — the only single-colour
            // plate in the archive, and the size of a scene illustration. It is
            // an EGA-only "blank the picture window" plate; `journey.cg1` and
            // `journey.mg1` (134 ids each) have no such entry, and it appears in
            // BOTH EGA parts, as a plate wanted on either disk would.
            ("journey", 135, 135, &[59u16][..]),
        ] {
            let (Some(p1), Some(p2), Some(mg)) = (
                fixture(&format!("{game}.eg1")),
                fixture(&format!("{game}.eg2")),
                fixture(&format!("{game}.mg1")),
            ) else {
                continue;
            };
            let mut set = InfocomPics::parse(p1).unwrap();
            assert_eq!((set.part(), set.parts(), set.next_part()), (1, 1, 2));
            set.append_part(InfocomPics::parse(p2).unwrap()).expect("eg2 continues eg1");
            assert_eq!((set.part(), set.parts(), set.next_part()), (1, 2, 3));

            assert_eq!(set.entries().len(), want_ids, "{game}: merged directory size");
            assert_eq!(
                set.entries().iter().filter(|e| e.has_pixels()).count(),
                want_pixels,
                "{game}: merged pictures carrying pixels"
            );
            assert!(
                set.entries().windows(2).all(|w| w[0].id < w[1].id),
                "{game}: a merged directory is still ascending and duplicate-free"
            );

            let mg = InfocomPics::parse(mg).unwrap();
            let mut only_ega: Vec<u16> = set
                .entries()
                .iter()
                .filter(|e| mg.entry(e.id).is_none())
                .map(|e| e.id)
                .collect();
            only_ega.sort_unstable();
            assert_eq!(only_ega, extra_ega_ids, "{game}: ids EGA has and MCGA does not");
            assert!(
                mg.entries().iter().all(|e| set.entry(e.id).is_some()),
                "{game}: the merged EGA set covers every MCGA id"
            );

            // Every picture the merge kept must still DECODE — the offsets of
            // part 2's records are rebased into the appended bytes, and a
            // rebasing error would land inside part 1's data and expand to
            // garbage rather than to an error.
            for e in set.entries().iter().filter(|e| e.has_pixels()) {
                let pic = set.decode(e.id).unwrap_or_else(|err| panic!("{game} id {}: {err:?}", e.id));
                assert_eq!((pic.width, pic.height), (e.width, e.height));
            }
        }
    }

    /// SQ-0798: the shared ids really are the same picture, verified rather than
    /// inferred — which is what licenses "the earlier part wins".
    ///
    /// The overlap is large (55 ids in Arthur, 12 in Journey) and the merge rule
    /// throws one copy of each away. The quest recorded that the two agree on
    /// dimensions and *assumed* they agree on pixels; this decodes both copies
    /// and compares them, so the assumption is either a measurement or a failure.
    #[test]
    fn a_picture_stored_on_both_disks_is_the_same_picture_on_both() {
        for (game, want_shared) in [("arthur", 55usize), ("journey", 12usize)] {
            let (Some(a), Some(b)) =
                (fixture(&format!("{game}.eg1")), fixture(&format!("{game}.eg2")))
            else {
                continue;
            };
            let (p1, p2) = (InfocomPics::parse(a).unwrap(), InfocomPics::parse(b).unwrap());
            let mut shared = 0;
            for x in p1.entries() {
                let Some(y) = p2.entry(x.id) else { continue };
                shared += 1;
                assert_eq!(
                    (x.width, x.height, x.flags),
                    (y.width, y.height, y.flags),
                    "{game} id {}: the two copies disagree on shape",
                    x.id
                );
                // Including the "neither has pixels" case: a placeholder in one
                // part and artwork in the other would make first-wins lossy, and
                // there is none.
                assert_eq!(
                    p1.decode(x.id).ok(),
                    p2.decode(x.id).ok(),
                    "{game} id {}: the two copies decode differently",
                    x.id
                );
            }
            assert_eq!(shared, want_shared, "{game}: ids stored on both disks");
        }
    }

    /// SQ-0798: a file that does not continue the set is refused, and refusing
    /// leaves the set exactly as it was.
    ///
    /// The part number is in-band, so it is checked rather than trusted — the
    /// alternative is merging whatever happens to sit under the next filename,
    /// which is the auto-pairing failure the tier policy exists to prevent,
    /// reached by a side door.
    #[test]
    fn a_file_that_does_not_continue_the_set_is_refused_and_changes_nothing() {
        let (Some(eg1), Some(eg2), Some(mg1)) =
            (fixture("arthur.eg1"), fixture("arthur.eg2"), fixture("arthur.mg1"))
        else {
            return;
        };
        let before = InfocomPics::parse(eg1.clone()).unwrap();

        // Wrong part number: `arthur.mg1` says part 1, and part 2 is what is
        // wanted. (It is also a whole other rendition, which is exactly the kind
        // of file that could sit under a mistaken name.)
        let mut set = InfocomPics::parse(eg1.clone()).unwrap();
        let err = set.append_part(InfocomPics::parse(mg1).unwrap()).unwrap_err();
        assert!(matches!(err, PicError::NotAContinuation(_)), "got {err:?}");
        assert_eq!(set.entries().len(), before.entries().len(), "a refusal changes nothing");
        assert_eq!(set.parts(), 1);

        // A different codec, even at the right part number, is not a continuation.
        if let Some(pic) = fixture("arthur.pic") {
            let mut amiga = InfocomPics::parse(pic).unwrap();
            assert_eq!(amiga.flavour(), Flavour::AmigaMac);
            let err = amiga.append_part(InfocomPics::parse(eg2.clone()).unwrap()).unwrap_err();
            assert!(matches!(err, PicError::NotAContinuation(_)), "got {err:?}");
        }

        // And a genuine part 2 that adds nothing, because it has already been
        // absorbed: a second identical append is refused rather than doubling
        // the directory.
        let mut set = InfocomPics::parse(eg1).unwrap();
        set.append_part(InfocomPics::parse(eg2.clone()).unwrap()).unwrap();
        let merged = set.entries().len();
        let err = set.append_part(InfocomPics::parse(eg2).unwrap()).unwrap_err();
        // It fails on the part number first — the set now wants part 3 — which is
        // the cheaper of the two checks and the more informative message.
        assert!(matches!(err, PicError::NotAContinuation(_)), "got {err:?}");
        assert_eq!(set.entries().len(), merged);
    }

    /// SQ-0734: which picture space each rendition's coordinates live in.
    ///
    /// This is not cosmetic. `zork0.mg1` and `zork0.eg1` hold the SAME artwork
    /// and disagree on its dimensions by a factor of two on the horizontal axis
    /// alone — EGA and CGA addressed 640 columns with half-width pixels. A host
    /// that plots either 1:1 without asking gets one of them wrong.
    ///
    /// The measured split, which is what [`InfocomPics::picture_space_width`]
    /// reads from header byte 1 bit 3: every authentic `.MG1` reads flags
    /// `0x30`, every `.EG1`/`.EG2`/`.CG1` reads `0x38` or `0x39`, and
    /// `zork0.pic` reads `0x06`.
    #[test]
    fn each_rendition_declares_its_picture_space() {
        #[rustfmt::skip]
        let corpus: [(&str, u16, u16); 9] = [
            // file            space  widest picture in it
            ("zork0.pic",        320,   320),
            ("zork0.mg1",        320,   320),
            ("zork0.eg1",        640,   640),
            ("zork0.cg1",        640,   640),
            ("arthur.mg1",       320,   320),
            ("arthur.eg1",       640,   640),
            ("journey.mg1",      320,   320),
            ("journey.cg1",      640,   640),
            ("shogun.mg1",       320,   320),
        ];
        for (name, space, widest) in corpus {
            let Some(bytes) = fixture(name) else { continue };
            let pics = InfocomPics::parse(bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(pics.picture_space_width(), space, "{name} picture space");
            // Cross-check the declaration against the directory it describes: no
            // picture may be wider than the space its coordinates are in, and the
            // widest one reaches the full width in each of these archives.
            let max = pics.entries().iter().map(|e| e.width).max().unwrap();
            assert_eq!(max, widest, "{name} widest record");
            assert!(max <= space, "{name} declares {space} but stores a {max}-wide picture");
        }
    }

    /// SQ-0794: which hardware table each rendition in `stories/` resolves
    /// through, and the corpus split that lets the directory alone decide it.
    ///
    /// The two questions [`InfocomPics::hardware_palette`] asks are checked here
    /// against every real archive in hand, because the rule is a measurement and
    /// not a reading of any specification — no published source says "a `.CG1`
    /// sets `EF_MONO` everywhere", it is simply what Infocom shipped. Both
    /// reference interpreters sidestep the question by knowing which card they
    /// are on before they open a file.
    ///
    /// The pixels are asserted alongside the flags, because the two are
    /// independent evidence for the same claim: a two-colour rendition should
    /// stay inside the four indices its packings use, and a sixteen-colour one
    /// should reach past them.
    #[test]
    fn each_palette_less_rendition_names_its_hardware_table() {
        #[rustfmt::skip]
        let corpus: [(&str, Option<[Rgb; 16]>); 14] = [
            ("zork0.eg1",    Some(EGA_PALETTE)),
            ("arthur.eg1",   Some(EGA_PALETTE)),
            ("journey.eg1",  Some(EGA_PALETTE)),
            ("shogun.eg1",   Some(EGA_PALETTE)),
            ("FMVPOKER.EG1", Some(EGA_PALETTE)),
            ("zork0.cg1",    Some(CGA_PALETTE)),
            ("arthur.cg1",   Some(CGA_PALETTE)),
            ("journey.cg1",  Some(CGA_PALETTE)),
            ("shogun.cg1",   Some(CGA_PALETTE)),
            // MCGA and the Amiga carry their colours per picture. `beyondzo.mg1`
            // is the one that would fool a rule keyed on the picture space: its
            // flag byte reads 640-wide where every other `.MG1` reads 320.
            ("zork0.mg1",    None),
            ("arthur.mg1",   None),
            ("shogun.mg1",   None),
            ("beyondzo.mg1", None),
            ("zork0.pic",    None),
        ];
        for (name, want) in corpus {
            let Some(bytes) = fixture(name) else { continue };
            let pics = InfocomPics::parse(bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(pics.hardware_palette(), want, "{name} hardware table");

            // The pixels, independently. A hardware-table archive's indices must
            // fit the table it was handed; a CGA one must stay two-colour in both
            // of its packings.
            let ceiling = if want == Some(CGA_PALETTE) { 4 } else { 16 };
            let mut reached = 0usize;
            for e in pics.entries().iter().filter(|e| e.has_pixels()) {
                for &i in &pics.decode(e.id).unwrap().indices {
                    reached = reached.max(usize::from(i) + 1);
                }
            }
            assert!(reached <= ceiling, "{name} reaches index {reached} of {ceiling}");
            if want == Some(EGA_PALETTE) {
                assert_eq!(reached, 16, "{name} is a sixteen-colour rendition");
            }
        }
    }

    /// SQ-0735's oracle, and the reason the LZW path can be trusted at all:
    /// **two unrelated codecs agree, picture for picture**.
    ///
    /// `zork0.mg1` (DOS, MCGA) and `zork0.pic` (the Amiga `Pic.data`) hold the
    /// same artwork at the same 320x200 on the same 12-bit palettes. The Amiga
    /// side is Huffman + run-length + per-line XOR and was verified against
    /// Infocom's own sources and against `Zork0.blb`'s PNGs; the PC side is LZW
    /// and shares not one line of code with it. For every id whose two
    /// directories agree on dimensions, the decoded pixels are identical.
    ///
    /// That is also what rules out a hidden stage on the PC side: an
    /// un-run-length or un-XOR step would have to be the identity on 383
    /// different pictures.
    #[test]
    fn mcga_zork_zero_decodes_to_the_amiga_pixels() {
        let (Some(pc), Some(amiga)) = (fixture("zork0.mg1"), fixture("zork0.pic")) else {
            return;
        };
        let pc = InfocomPics::parse(pc).unwrap();
        let amiga = InfocomPics::parse(amiga).unwrap();
        assert_eq!(pc.flavour(), Flavour::Pc);
        assert_eq!(amiga.flavour(), Flavour::AmigaMac);

        let (mut same, mut differ, mut sized_differently) = (0, 0, Vec::new());
        for e in pc.entries().iter().filter(|e| e.has_pixels()) {
            let Some(a) = amiga.entry(e.id).filter(|a| a.has_pixels()) else {
                continue;
            };
            if (a.width, a.height) != (e.width, e.height) {
                sized_differently.push(e.id);
                continue;
            }
            let (p, q) = (pc.decode(e.id).unwrap(), amiga.decode(e.id).unwrap());
            if p.indices == q.indices {
                same += 1;
            } else {
                differ += 1;
                assert!(differ < 4, "picture {} differs between the two codecs", e.id);
            }
        }
        assert_eq!((same, differ), (383, 0));
        // The only ids the two directories size differently are the five
        // SQ-0713 already catalogued, where the Amiga release holds a full
        // decorative frame and the other sources a cropped band.
        assert_eq!(sized_differently, vec![5, 6, 7, 8, 33]);

        // Same palettes, too — the MCGA archive stores the Amiga's 12-bit
        // values, which is why the pixels can be identical in the first place.
        // Pinned absolutely as well as relatively, so that the shared claim
        // "the stored triples start at colour 2" is answerable by real media on
        // this side and not only by a hand-built archive.
        assert_eq!(pc.palette_of(1), amiga.palette_of(1));
        let pal = pc.palette_of(1).expect("the MCGA title screen carries a palette");
        assert_eq!(pal[2], [0xcc, 0xcc, 0xcc]);
        assert_eq!(pal[15], [0xee, 0xee, 0xee]);
    }

    /// Zork Zero shipped 503 pictures in every PC rendition, and the three
    /// directories are the same directory: same ids, and the same 396 of them
    /// carry pixels while the same 107 are size-only placeholders. That holds
    /// without decoding anything, so it is an independent check on the
    /// little-endian directory reading.
    #[test]
    fn the_pc_renditions_of_zork_zero_share_a_directory() {
        let renditions: Vec<InfocomPics> = ["zork0.mg1", "zork0.eg1", "zork0.cg1"]
            .iter()
            .filter_map(|n| fixture(n))
            .map(|b| InfocomPics::parse(b).unwrap())
            .collect();
        if renditions.len() < 3 {
            return;
        }
        let shape = |p: &InfocomPics| -> Vec<(u16, bool)> {
            p.entries().iter().map(|e| (e.id, e.has_pixels())).collect()
        };
        assert_eq!(shape(&renditions[0]).len(), 503);
        assert_eq!(shape(&renditions[1]), shape(&renditions[0]));
        assert_eq!(shape(&renditions[2]), shape(&renditions[0]));

        // MCGA is the only PC rendition with palettes of its own; EGA and CGA
        // records are 12 bytes with nowhere to put one.
        assert!(renditions[0].adaptive_pictures().len() < 396);
        assert_eq!(renditions[1].adaptive_pictures().len(), 396);
        assert_eq!(renditions[2].adaptive_pictures().len(), 396);
    }

    /// Arthur's, Journey's and Shogun's CGA archives are the only ones that
    /// carry `EF_MONO` artwork — one bit per pixel, rows padded to whole bytes.
    /// Zork Zero's CGA archive carries none, so a suite that only looked at
    /// Zork Zero would never exercise the unpacking at all.
    #[test]
    fn the_cga_archives_carry_bit_packed_artwork() {
        for (name, packed) in [
            ("arthur.cg1", 92),
            ("journey.cg1", 111),
            ("shogun.cg1", 25),
            ("zork0.cg1", 0),
        ] {
            let Some(bytes) = fixture(name) else { continue };
            let pics = InfocomPics::parse(bytes).unwrap();
            let mono: Vec<&PicEntry> = pics
                .entries()
                .iter()
                .filter(|e| e.has_pixels() && e.flags & EF_MONO != 0 && e.flags & EF_TRANS == 0)
                .collect();
            assert_eq!(mono.len(), packed, "{name}");
            for e in mono {
                let p = pics.decode(e.id).unwrap();
                assert!(
                    p.indices.iter().all(|&i| i < 2),
                    "{name} picture {} is not two-colour after unpacking",
                    e.id
                );
            }
        }
    }

    /// The picture archive off an Amiga release floppy, or `None` (with a SKIP
    /// note) when the gitignored disk image is not there.
    fn adf_pictures(image: &str) -> Option<InfocomPics> {
        let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(image);
        let Ok(bytes) = std::fs::read(&p) else {
            eprintln!("SKIP: gitignored disk image missing at {}", p.display());
            return None;
        };
        let adf = crate::adf::Adf::mount(bytes).expect("an Amiga release floppy mounts");
        Some(adf.pictures().expect("the floppy carries a picture archive").1)
    }

    /// FNV-1a over every decoded picture's indices *and* its resolved palette,
    /// in directory order — one number that moves if any pixel or any colour of
    /// an archive changes.
    fn fingerprint(pics: &InfocomPics) -> (usize, usize, u64) {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut n = 0;
        let feed = |h: &mut u64, b: u8| *h = (*h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        for e in pics.entries() {
            if !e.has_pixels() {
                continue;
            }
            n += 1;
            let p = pics.decode(e.id).expect("a picture with pixels decodes");
            assert_eq!(
                p.indices.len(),
                usize::from(e.width) * usize::from(e.height),
                "picture {} decoded to the wrong size",
                e.id
            );
            for &i in &p.indices {
                feed(&mut h, i);
            }
            for c in p.palette.unwrap_or(DEFAULT_PALETTE) {
                for b in c {
                    feed(&mut h, b);
                }
            }
        }
        (pics.entries().len(), n, h)
    }

    /// Arthur's Apple press, or `None` (with a SKIP note) when the gitignored
    /// image is not there.
    ///
    /// Through `ProDos::packed_pictures`, which is the door a host will use once
    /// the picture-space question `ProDos::pictures` documents is settled. The
    /// chain from a `.2mg` on disk to decoded pixels is otherwise complete, and
    /// these tests walk all of it.
    fn apple_arthur() -> Option<InfocomPics> {
        let bytes = fixture("Arthur Quest 4 Excalibur.2mg")?;
        let fs = crate::prodos::ProDos::mount(bytes).expect("the 2mg mounts");
        let (name, pics) = fs.packed_pictures().expect("Arthur's Apple press carries artwork");
        assert_eq!(name, "ARTHUR.1/ARTHUR.D1", "named for the segment carrying the index");
        Some(pics)
    }

    /// SQ-0863, real media: *Arthur* release 63 / serial 890622, off
    /// `Arthur Quest 4 Excalibur.2mg` — the Apple flavour, which this reader
    /// used to refuse outright on its 8-byte record.
    ///
    /// `stories/` is gitignored, so this skips vacuously on CI.
    #[test]
    fn reads_the_apple_arthur_archives() {
        let Some(pics) = apple_arthur() else { return };
        assert_eq!(pics.flavour(), Flavour::Apple);
        // Four floppies carry art; `PHFID` is the disk number, so the set starts
        // at 2 rather than at 1.
        assert_eq!(pics.part(), 2, "the first archive is disk 2's");
        assert_eq!(pics.parts(), 4, "disks 2, 3, 4 and 5 all carry artwork");

        // 51 + 26 + 53 + 38.
        assert_eq!(pics.entries().len(), 168);
        let with_pixels = pics.entries().iter().filter(|e| e.has_pixels()).count();
        assert_eq!(with_pixels, 135, "the other 33 are size-only placeholders");

        // The parts PARTITION the id space — unlike the PC's `.EG1`/`.EG2`
        // split, where 55 of Arthur's ids are deliberately on both floppies.
        // Nothing was dropped by the merge, so the totals add up exactly.
        let mut ids: Vec<u16> = pics.entries().iter().map(|e| e.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 168, "no id appears in two parts");

        // `apple.equ`: MAXWIDTH EQU 140 ; 560 / 4, MAXHEIGHT EQU 192.
        assert_eq!(pics.picture_space_width(), 140);
        assert_eq!(pics.picture_space_height(), 192);
        let widest = pics.entries().iter().map(|e| e.width).max().unwrap();
        let tallest = pics.entries().iter().map(|e| e.height).max().unwrap();
        assert_eq!((widest, tallest), (140, 192), "the full-screen plates fill it exactly");

        // No archive stores a palette (`PHFPAL`), so every picture is adaptive
        // and the colours come from the hardware table.
        assert!(pics.entries().iter().all(|e| !e.has_own_palette()));
        assert_eq!(pics.hardware_palette(), Some(APPLE_DHGR_PALETTE));
        assert!(!pics.is_monochrome(), "sixteen colours, not two");

        // Every picture decodes to exactly its declared size, in range.
        for e in pics.entries().iter().filter(|e| e.has_pixels()) {
            let p = pics.decode(e.id).expect("a picture with pixels decodes");
            assert_eq!(
                p.indices.len(),
                usize::from(e.width) * usize::from(e.height),
                "picture {} decoded to the wrong size",
                e.id
            );
            assert!(p.indices.iter().all(|&i| i < 16), "picture {} left the palette", e.id);
        }
        // One number over every pixel and every resolved colour.
        assert_eq!(fingerprint(&pics), (168, 135, 0x826e_44f4_17af_3169));
    }

    /// SQ-0863: the three-byte length prefix is an oracle, and this is it.
    ///
    /// `decode_apple` steps over the prefix exactly as `pic.asm` does and never
    /// reads it, so it is an independent statement of where each RLE stream
    /// ends. The pictures pack contiguously from the byte after the directory,
    /// which means `dataOff + 3 + declared` must land precisely on the next
    /// picture's `dataOff` — 135 times, with nothing left over. A decoder that
    /// stopped in the wrong place could not produce a gapless tiling.
    #[test]
    fn the_apple_length_prefix_tiles_every_archive_exactly() {
        let Some(pics) = apple_arthur() else { return };
        let mut offsets: Vec<usize> =
            pics.entries().iter().filter(|e| e.has_pixels()).map(|e| e.data).collect();
        offsets.sort_unstable();
        assert_eq!(offsets.len(), 135);

        let mut contiguous = 0;
        let mut seams = 0;
        for pair in offsets.windows(2) {
            let (here, next) = (pair[0], pair[1]);
            let declared = be24(&pics.data, here);
            // A picture's declared length may never reach into the next one.
            assert!(here + APPLE_PREFIX + declared <= next, "picture at {here:#x} overruns");
            if here + APPLE_PREFIX + declared == next {
                contiguous += 1;
            } else {
                seams += 1;
            }
        }
        // Four archives were merged, so exactly three of the 134 gaps are seams
        // between one part's last picture and the next part's first — every
        // other pair is nose to tail with not one byte between them.
        assert_eq!((contiguous, seams), (131, 3), "the declared lengths tile each archive");
    }

    /// SQ-0863: the transparent colour is the flag byte's TOP nibble.
    ///
    /// `pic.asm` tests bit 0 and then shifts the same byte right four
    /// (`APPLE_EF_TRANS`). Getting the shift wrong is invisible in a
    /// size-and-range check and shows up only as a hole in the wrong colour, so
    /// it is pinned against the real directory: Arthur names colours 1, 2, 3
    /// and 4, and 119 of its 135 pictures name none.
    #[test]
    fn the_apple_transparent_colour_is_the_flag_bytes_top_nibble() {
        let Some(pics) = apple_arthur() else { return };
        let mut named = std::collections::BTreeMap::<u8, usize>::new();
        let mut opaque = 0usize;
        for e in pics.entries().iter().filter(|e| e.has_pixels()) {
            match pics.decode(e.id).unwrap().transparent {
                Some(c) => {
                    assert_eq!(u16::from(c), e.flags >> 4, "picture {}", e.id);
                    assert_ne!(e.flags & APPLE_EF_TRANS, 0);
                    *named.entry(c).or_default() += 1;
                }
                None => {
                    assert_eq!(e.flags & APPLE_EF_TRANS, 0, "picture {}", e.id);
                    opaque += 1;
                }
            }
        }
        assert_eq!(named, [(1, 3), (2, 8), (3, 4), (4, 1)].into_iter().collect());
        assert_eq!(opaque, 119);
        // A transparent index really does drop out of the RGBA expansion.
        let (&colour, _) = named.iter().next().unwrap();
        let id = pics
            .entries()
            .iter()
            .find(|e| e.has_pixels() && e.flags & APPLE_EF_TRANS != 0 && e.flags >> 4 == 1)
            .unwrap()
            .id;
        let pic = pics.decode(id).unwrap();
        let rgba = pic.rgba();
        for (i, &ix) in pic.indices.iter().enumerate() {
            assert_eq!(rgba[i * 4 + 3], if ix == colour { 0 } else { 255 }, "pixel {i}");
        }
    }

    /// SQ-0863: a segment is not an archive, and `parse` must keep saying so.
    ///
    /// An Apple archive's offsets are positions in the SEGMENT, so handing the
    /// segment to `parse` — which looks for a header at byte 0 — has to fail:
    /// byte 0 of `ARTHUR.D2` is story page data, not `PHFID`. This is the guard
    /// that the new 8-byte branch did not turn `parse` into something that
    /// accepts arbitrary story bytes.
    #[test]
    fn parse_still_refuses_a_bare_apple_segment() {
        let Some(bytes) = fixture("Arthur Quest 4 Excalibur.2mg") else { return };
        let disk = crate::medium::MountedDisk::mount(bytes).expect("the 2mg mounts");
        let mut seen = 0;
        for (name, seg) in disk.contents() {
            if !name.to_ascii_uppercase().contains("ARTHUR.D") {
                continue;
            }
            seen += 1;
            assert!(
                InfocomPics::parse(seg).is_err(),
                "{name} parsed as an archive from byte 0, which it is not"
            );
        }
        assert_eq!(seen, 5, "five segments");
    }

    /// SQ-0744, real media: Shogun's Amiga floppy, the 16-byte flavour.
    ///
    /// Before this reader learned the flavour, `parse` rejected the whole file
    /// on its record size and Shogun booted with no artwork at all — no title
    /// screen. `stories/` is gitignored, so this skips vacuously.
    #[test]
    fn reads_amiga_shogun() {
        let Some(pics) = adf_pictures("James Clavell's Shogun.adf") else { return };
        assert_eq!(pics.entries().len(), 48);

        // 42 records carry pixels; the other 6 are size-only placeholders, and
        // they are id for id the 6 `Rect` resources in `Shogun.blb` — the same
        // 1:1 correspondence SQ-0713 measured for Zork Zero's 107.
        let placeholders: Vec<u16> =
            pics.entries().iter().filter(|e| !e.has_pixels()).map(|e| e.id).collect();
        assert_eq!(placeholders, vec![2, 45, 46, 47, 48, 49]);

        // Every one decodes, and to the bytes the Blorb oracle validated:
        // 34 of the 39 pictures `Shogun.blb` also holds are byte-exact
        // (`crates/app/tests/v6_shogun_native_archive.rs` runs that comparison).
        assert_eq!(fingerprint(&pics), (48, 42, 0x85ce_26b1_aa1f_20db));

        // SQ-0743's correspondence, checked on this flavour: every record with
        // pixels names a palette of its own, so nothing here is adaptive — and
        // `Shogun.blb` carries no `APal` chunk at all. The two agree, as they do
        // for Zork Zero's 172.
        assert!(pics.adaptive_pictures().is_empty());

        // Picture 1 is the title screen the report is about.
        let p = pics.decode(1).unwrap();
        assert_eq!((p.width, p.height), (320, 200));
        assert_eq!(p.palette.expect("the title screen carries a palette")[2], [0x00, 0x00, 0xff]);
    }

    /// The 14-byte flavour must not move. Zork Zero, Journey and Arthur declare
    /// `HF_EHUFF | HF_GHUFF` and share one global Huffman tree; these
    /// fingerprints were taken before this quest touched `parse` and are
    /// byte-identical after it.
    #[test]
    fn the_global_huffman_tree_archives_are_unmoved() {
        for (image, want) in [
            ("Zork Zero - The Revenge of Megaboz.adf", (495, 388, 0x6c01_a84b_8143_80bfu64)),
            ("Journey - The Quest Begins.adf", (134, 134, 0x3c7e_9a34_6ab8_41f2)),
            ("Arthur - The Quest for Excalibur.adf", (169, 135, 0x4f95_fb95_3640_f18f)),
        ] {
            let Some(pics) = adf_pictures(image) else { continue };
            assert_eq!(fingerprint(&pics), want, "{image} decoded differently");
        }
    }

    /// Both archives off the Macintosh Zork Zero disk (v6 r296 s881019), or
    /// `None` with a SKIP note when the gitignored image is not there.
    fn mac_zork_zero() -> Option<(InfocomPics, InfocomPics)> {
        let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/Zork Zero Disk.image");
        let Ok(bytes) = std::fs::read(&p) else {
            eprintln!("SKIP: Macintosh disk image missing at {}", p.display());
            return None;
        };
        let hfs = crate::hfs::Hfs::mount(bytes).expect("the Macintosh floppy mounts");
        let colour = InfocomPics::parse(hfs.read_named("CPic.data")?).expect("CPic.data parses");
        let mono = InfocomPics::parse(hfs.read_named("Pic.data")?).expect("Pic.data parses");
        Some((mono, colour))
    }

    /// SQ-0838's oracle: **the two Macintosh archives are the same catalogue.**
    ///
    /// `Pic.data` and `CPic.data` ship on one disk and hold one game's artwork
    /// for the two screens Apple sold. The colour side is already verified — its
    /// 386 pictures decode through the Amiga path and were checked against
    /// `Zork0.blb` — so it is a reference the monochrome side can be held to
    /// without appealing to any external file or any remembered constant. What
    /// the two must agree on is the DIRECTORY: the same 483 records, the same
    /// ids in the same order, and the same 386 of them carrying pixels.
    ///
    /// **What they must NOT agree on is dimensions, and that is a finding rather
    /// than a loosened assertion.** The monochrome archive is drawn for a 480×300
    /// screen (`mac/gfx.p`: "this pic is mono, and scaled for a 480x300 screen
    /// (std Mac)") and the colour one for 320×200, so 480 of the 483 records
    /// differ. The three that "agree" agree only because they are placeholders
    /// with a zero axis — ids 361 (0×0), 410 (0×15) and 412 (15×0) — so not one
    /// picture with pixels matches, which is the opposite of luck. Full-screen
    /// plates come out at exactly 1.5× (480×300 against 320×200); the sprites do
    /// not, because they are separately drawn artwork and not a scaled copy —
    /// id 2 is 60×50 against 42×35, id 9 is 61×59 against 45×40.
    #[test]
    fn the_two_macintosh_archives_are_one_catalogue() {
        let Some((mono, colour)) = mac_zork_zero() else { return };

        let shape = |p: &InfocomPics| -> Vec<(u16, bool)> {
            p.entries().iter().map(|e| (e.id, e.has_pixels())).collect()
        };
        assert_eq!(shape(&mono).len(), 483);
        assert_eq!(shape(&mono), shape(&colour), "same ids, same pixel-bearing records");
        assert_eq!(mono.entries().iter().filter(|e| e.has_pixels()).count(), 386);

        // Dimensions differ everywhere they can. Only the three records with a
        // zero axis coincide, and none of them has pixels.
        let same: Vec<u16> = mono
            .entries()
            .iter()
            .zip(colour.entries())
            .filter(|(a, b)| (a.width, a.height) == (b.width, b.height))
            .map(|(a, _)| a.id)
            .collect();
        assert_eq!(same, vec![361, 410, 412]);
        assert!(same.iter().all(|id| !mono.entry(*id).unwrap().has_pixels()));

        // The full-screen plates, where the two picture spaces line up exactly.
        for id in [1u16, 5, 6, 7] {
            let (m, c) = (mono.entry(id).unwrap(), colour.entry(id).unwrap());
            assert_eq!((m.width, m.height), (480, 300), "mono plate {id}");
            assert_eq!((c.width, c.height), (320, 200), "colour plate {id}");
        }
    }

    /// SQ-0838, the pixels: every monochrome picture decodes, and to a shape and
    /// a colour range only this codec can produce.
    ///
    /// The per-picture length check is the strong one. Nothing about the entry
    /// says how many bytes its stream should yield — the run-length stage decides
    /// that as it runs — so `width * height` coming out 386 times in a row is not
    /// something a wrong packing survives. A bit-per-pixel reading would land
    /// eight times short every time.
    #[test]
    fn every_macintosh_monochrome_picture_decodes() {
        let Some((mono, _)) = mac_zork_zero() else { return };
        assert_eq!(mono.flavour(), Flavour::AmigaMac, "big-endian, one global Huffman tree");
        assert!(mono.is_monochrome());
        assert_eq!(mono.hardware_palette(), Some(MAC_MONO_PALETTE), "an AmigaMac container");
        assert_eq!((mono.picture_space_width(), mono.picture_space_height()), (480, 300));
        // No 12-byte record can carry a palette, so all 386 are adaptive and the
        // hardware table above is the only thing that colours them.
        assert_eq!(mono.adaptive_pictures().len(), 386);

        let (mut decoded, mut opaque, mut transparent) = (0usize, 0usize, 0usize);
        let mut seen = [false; 16];
        for e in mono.entries().iter().filter(|e| e.has_pixels()) {
            let p = mono.decode(e.id).unwrap_or_else(|err| panic!("picture {}: {err:?}", e.id));
            assert_eq!(
                p.indices.len(),
                usize::from(e.width) * usize::from(e.height),
                "picture {} decoded to the wrong size",
                e.id
            );
            for &i in &p.indices {
                seen[usize::from(i) & 15] = true;
            }
            match p.transparent {
                Some(t) => {
                    assert_eq!(t, 0, "colour 0 is the transparent one on this side");
                    transparent += 1;
                }
                None => {
                    assert!(
                        !p.indices.contains(&0),
                        "picture {} is opaque yet uses the transparent colour",
                        e.id
                    );
                    opaque += 1;
                }
            }
            decoded += 1;
        }
        assert_eq!((decoded, opaque, transparent), (386, 258, 128));

        // The colour numbers, and they are the two-colour ones: 2 and 3 (white
        // and black, per bocfel's shared Mac-B/W-and-CGA table), plus 0 where a
        // picture declares a transparent colour. Index 1 — this decoder's
        // rendering of a PC bit-packed set bit — never appears, which is what
        // says the Macintosh never uses that packing.
        assert_eq!(seen[..4], [true, false, true, true]);
        assert!(seen[4..].iter().all(|&s| !s), "nothing reaches past the four low indices");
    }

    /// The whole monochrome archive, in one number, so that a change to any
    /// pixel or any resolved colour of it fails loudly (SQ-0838).
    #[test]
    fn the_macintosh_monochrome_archive_is_pinned() {
        let Some((mono, colour)) = mac_zork_zero() else { return };
        assert_eq!(fingerprint(&mono), (483, 386, 0x61bf_7af0_03c5_ffb4));
        // And the colour archive beside it has not moved: this quest touched the
        // record-size rule every Amiga/Mac archive goes through.
        assert_eq!(fingerprint(&colour), (483, 386, 0xb855_8076_4f4a_7aec));
    }
}
