//! The machine table: what each ZMSD §11.1.3 interpreter number *is* (SQ-0872).
//!
//! Header byte `$1E` names a machine, and a story that reads it expects the rest
//! of the machine to match — its default page and ink (`$2C`/`$2D`, §8.3.3), the
//! palette its colour numbers resolve through (§8.3.1.1), and the screen rules
//! §8.3 gives that machine by name. Setting the byte alone produces a machine
//! that says one thing and looks like another, which is exactly what `zvm-cli`
//! did until this module existed: it told *Beyond Zork* off a ProDOS disk that it
//! was an Apple IIgs while the header still carried zvm's own §8.3.2 seed.
//!
//! # Why the table lives here, and what deliberately does not
//!
//! This is zvm's domain already. The screen model has carried a per-machine rule
//! for one machine — the Amiga's global colour pens — since SQ-0740, expressed as
//! a special case beside a lone `AMIGA_INTERPRETER_NUMBER`. A second machine
//! arrived (the Macintosh, SQ-0846) and landed in `app::session` instead, because
//! that is where the profile was; one concept, two crates, and `zvm-cli` able to
//! see only one of them. Making the special case a table fixes both directions at
//! once: `zvm` has an empty `[dependencies]` and both front-ends already depend
//! on it, so the CLI gets the whole bundle with no new crate anywhere.
//!
//! **The table is keyed by the interpreter NUMBER**, not by an enum, and that is
//! the same choice `blorb::medium` made for the medium→machine question. A number
//! is a compact, published, stable encoding that needs no shared type, which is
//! what lets three crates that must not depend on each other talk about the same
//! machine. `blorb` says which machine a DISK implies; this says what that machine
//! IS; `app::interpreter` is the only place that maps a path to either, because
//! reading a file is I/O policy and zvm has none.
//!
//! So what stays in `app::interpreter` stays there for a reason:
//!
//! - `InterpreterProfile::resolve` reads the medium off disk — I/O policy, against
//!   zvm's charter.
//! - the art-flavour preference needs `blorb::infocom_pics::Flavour`, and zvm takes
//!   zero external dependencies.
//! - `std_window` is a Version 6 picture space stated by an ARCHIVE rather than by
//!   the machine, which is why `AppleIIgs`'s is `None` (SQ-0857) while its number,
//!   page and palette are all right here.
//!
//! # Sourcing
//!
//! Every value below is quoted at its constant, from ZMSD §11.1.3 and from
//! Infocom's own interpreters in `github.com/erkyrath/infocom-zcode-terps`. The
//! Atari ST row is the standard the rest are held to: it sources its 5 from the
//! standard *and* from `st/stx1.s`'s `INTWRD DC.B 5 * MACHINE ID FOR ATARI ST`.
//! Where a value cannot be sourced it is declined outright — [`MACHINES`] carries
//! only what is known, and [`machine`] answers `None` for a number no row states,
//! so a front-end can say "I do not model that machine" instead of quietly
//! dressing it as an IBM PC.

use crate::memory::Memory;
use crate::screen::{Palette, V6Cell, ZColour};

// ---------------------------------------------------------------------------
// §11.1.3 interpreter numbers
// ---------------------------------------------------------------------------

/// Apple IIe, from the ZMSD §11.1.3 interpreter-number table: *"1 DECSystem-20,
/// **2 Apple IIe**, 3 Macintosh, 4 Amiga, 5 Atari ST, 6 IBM PC …"* — and
/// corroborated by Infocom's own Apple II YZIP, which picks its own identity at
/// boot. `apple/yzip/rel.15/bsubs.asm`'s `MACHINE:` routine chooses between the
/// family's three:
///
/// ```text
///   IIeID  EQU 2   ; ][e Yzip
///   IIcID  EQU 9   ; ][c Yzip
///   IIgsID EQU 10  ; ][gs Yzip
/// ```
///
/// The three machines share the entire Apple bundle and differ only in this byte
/// (SQ-0857 established that and scoped itself to the IIgs; SQ-0872 filled the
/// other two in). Which one a ProDOS *disk* implies is `blorb::medium`'s question
/// and it still answers 10 — the medium cannot name the press, and of the three
/// the one a modern terminal resembles is the IIgs. This constant exists so that
/// a player who names 2 outright gets the Apple, not the IBM PC.
pub const APPLE_IIE_INTERPRETER_NUMBER: u8 = 2;

/// Macintosh, from the same §11.1.3 table (SQ-0838). Read from the standard, not
/// recalled: *"1 DECSystem-20, 2 Apple IIe, **3 Macintosh**, 4 Amiga, 5 Atari ST,
/// 6 IBM PC …"*.
pub const MACINTOSH_INTERPRETER_NUMBER: u8 = 3;

/// Amiga, from the same §11.1.3 table. This is the byte §8.3's own Amiga rule
/// tests the header against — see [`MachineProfile::global_colour_pens`].
pub const AMIGA_INTERPRETER_NUMBER: u8 = 4;

/// Atari ST, from the same §11.1.3 table (SQ-0835) — and the first of the rows
/// the machine's own interpreters corroborate directly, `st/stx1.s`:
///
/// ```text
///   INTWRD  DC.B 5   * MACHINE ID FOR ATARI ST
/// ```
///
/// No version arm, no condition: where the IBM PC's honest number is a rule
/// ([`crate::screen::default_interpreter_number`]) the ST's is a flat constant.
pub const ATARI_ST_INTERPRETER_NUMBER: u8 = 5;

/// IBM PC, from the same §11.1.3 table. The machine whose number is a **rule**
/// rather than a constant — [`crate::screen::default_interpreter_number`] is
/// Frotz's 6-for-Version-6, 1-otherwise, and it is the reason
/// [`MachineProfile::default_colours`] is `None` on this row: an IBM PC in a
/// terminal is the player's terminal, so "default" should mean what the player
/// actually sees.
pub const IBM_PC_INTERPRETER_NUMBER: u8 = 6;

/// The IBM PC's default page: §8.3.1's **blue** (SQ-0928).
///
/// Observed rather than read out of Infocom's source, and the observation is
/// unusually well supported. Three DOS captures — *Shogun* r322 at its menu,
/// *Arthur* r74 mid-game, *Zork Zero* at the Banquet Hall — plus the report that
/// Zork Zero's BOOT sequence is blue/white before the game runs, i.e. before it
/// can have set anything. Against that, a trace of every `@set_colour` each game
/// issues: Arthur (941 screen ops) and Journey (532) name no colour at all and
/// are blue; Shogun names one, on a 548x32 status strip, and is blue everywhere
/// else; Zork Zero names one on a window the size of the screen and is white.
/// Four games, one rule, no exceptions.
///
/// **What is NOT being claimed is a shade.** These are §8.3.1 colour NUMBERS and
/// they resolve through the palette and the player's theme, so unlike the period
/// look ([`PeriodLook`]) there is no emulator-dependent RGB here to be wrong about.
///
/// **And stating it is not the same as applying it.** The gate is the app's
/// `Config::machine_default_colours`: this pair belongs to the IBM PC as a machine
/// a medium named, not to the fallback every story with no medium falls through to.
pub const IBM_PC_DEFAULT_BACKGROUND: u8 = 6;

/// The IBM PC's default ink: §8.3.1's **white**. See [`IBM_PC_DEFAULT_BACKGROUND`].
pub const IBM_PC_DEFAULT_FOREGROUND: u8 = 9;

/// The IBM PC's page when its display is showing **two colours**: §8.3.1's
/// **black** (SQ-0956).
///
/// The card the machine is showing is not the machine. [`IBM_PC_DEFAULT_BACKGROUND`]
/// is blue, measured off three DOS captures of the full-colour renditions, and it
/// stands for those. Put a CGA plate on the same machine and the screen inverts:
/// `machine-screenshots/dos-zorkzero-cga.png` — Zork Zero r393 at the Banquet Hall,
/// a DOS emulator in CGA mode running `zork0.cg1` — censuses **48.3% `#000000`**
/// page under **8.8% `#A0A0A0`** ink, 161 distinct colours from video scaling and
/// no second hue anywhere in the frame. Row parity was checked first, because an
/// interlaced capture censuses backwards (SQ-0933): even rows 39,252 black /
/// 7,135 grey, odd rows 38,391 / 6,968 — they agree, so the whole-frame census is
/// the honest one.
///
/// **A colour NUMBER, not a shade**, exactly as the constant above. The ink is
/// white 9 — `#A0A0A0` is the same value `dos-hitchhiker.png` measures for its
/// ink, and the IBM PC row already resolves white through EGA entry 7
/// ([`crate::screen::ega_true_colour`]) — so **one channel moves**: the page,
/// from blue 6 to black 2. That single difference is the whole discriminator
/// `app::graphics::PictSource::declines_game_colours` reads; see
/// `app::interpreter::InterpreterProfile::two_colour_colours`.
pub const IBM_PC_TWO_COLOUR_BACKGROUND: u8 = 2;

/// Commodore 128, from the same §11.1.3 table (SQ-0869)/// Commodore 128, from the same §11.1.3 table (SQ-0869) — the one row
/// corroborated by a DISK rather than by an interpreter source tree.
/// `TRINITY1.D64` opens with the Commodore 128's `CBM` autoboot signature and
/// boots an interpreter that touches the C128's own MMU register `$FF00` forty
/// times, which no Commodore 64 has. `blorb::medium` shows the evidence and
/// argues why the `.d64` row answers 7 where the family's other number is 8.
pub const COMMODORE_128_INTERPRETER_NUMBER: u8 = 7;

/// Apple IIc, from the same §11.1.3 table and the same `MACHINE:` routine as
/// §11.1.3's Commodore 64.
///
/// No medium selects it — a `.d64` is a 1541 image both Commodore machines read —
/// so it is reached by asking for it, exactly as the Apple IIe and IIc are on a
/// ProDOS volume that cannot name which of the family pressed it.
pub const COMMODORE_64_INTERPRETER_NUMBER: u8 = 8;

/// [`APPLE_IIE_INTERPRETER_NUMBER`] — `IIcID EQU 9 ; ][c Yzip`.
pub const APPLE_IIC_INTERPRETER_NUMBER: u8 = 9;

/// Apple IIgs, from the same §11.1.3 table (SQ-0857) — and the second of the
/// rows the machine's own interpreter corroborates directly, `IIgsID EQU 10 ;
/// ][gs Yzip`. `blorb::medium` quotes the Apple II YZIP in full and shows the
/// byte is neither a flat constant (as the ST's is) nor a version rule (as the
/// IBM PC's is) but a **runtime machine detection** across the family's three
/// numbers.
pub const APPLE_IIGS_INTERPRETER_NUMBER: u8 = 10;

// ---------------------------------------------------------------------------
// §8.3.3 default colour pairs
// ---------------------------------------------------------------------------

/// The Amiga's default background: standard colour **12**, dark grey (`$444`).
///
/// **Do not "correct" this back to 11 on the strength of `amiga/yzip.h`** — that
/// file is a development snapshot, its own `#define DEF_BACK 11 /*6*/` carries the
/// scar of a previous edit, and the value that shipped is 12 (SQ-0822).
///
/// The authority is the interpreter on the release floppy, which is the program
/// that painted the screen. `set_back()` in `amiga/yzip3.c` opens
/// `if (id == 1) id = DEF_BACK;` — "colour 1 means the default" — and `set_fore()`
/// opens with the same line on `DEF_FORE`. Both compile to a `cmpi.w #1` and a
/// `moveq`, and both appear, once each and in that order, in every Amiga Version 6
/// interpreter in `stories/`:
///
/// ```text
///   0c 47 00 01   cmpi.w #1,d7      0c 47 00 01   cmpi.w #1,d7
///   66 02         bne.s  .+2        66 02         bne.s  .+2
///   7e 09         moveq  #9,d7      7e 0c         moveq  #12,d7
///   … set_fore: DEF_FORE = 9        … set_back: DEF_BACK = 12
/// ```
///
/// and `set_color()`'s `return ((DEF_BACK << 8) | DEF_FORE)` assembles to
/// `30 3c 0c 09` — `move.w #$0C09,d0` — in all four. `$0B09` occurs in none of
/// them. Offsets, per interpreter binary extracted from its floppy: Arthur
/// (release 54) 18958/19094/18742, Journey (release 30) 17816/17952/17792,
/// Zork Zero (release 366) and Shogun (release 295) 17820/17956/17796.
///
/// It is also what the screen shows. Real Amiga captures of Journey release 30
/// (lemonamiga.com's gallery, 640×512) tally 173,994 pixels of `#444444` under
/// 25,878 of `#FFFFFF`, and MobyGames' Arthur church capture is `#444444` page,
/// `#FFFFFF` ink, with the status bar reversed to `#444444` ON `#FFFFFF` — which
/// is pen 0 and pen 1 swapped, so the page really is the text background register
/// and not artwork. `$444` is Infocom's colour 12; `$777`, colour 11, appears in
/// neither. `app::colors` resolves 12 through the Amiga palette to `(66,66,66)`
/// — two units off `#444444` only because a 4-bit Amiga channel has to pass
/// through the Z-machine's 5-bit true-colour word on the way.
///
/// This does not disturb SQ-0740's window-0 gate: the evidence for that was that
/// Journey is *not black*, and a dark-grey page is not a black one.
pub const AMIGA_DEFAULT_BACKGROUND: u8 = 12;

/// The Amiga's default foreground: standard colour 9, white.
///
/// Source: `set_fore()`'s `if (id == 1) id = DEF_FORE;` in the interpreter on every
/// Infocom Amiga release floppy — `moveq #9` — agreeing with
/// `#define DEF_FORE 9  /* default Amiga foreground = white */` in `amiga/yzip.h`.
/// See [`AMIGA_DEFAULT_BACKGROUND`], where the two sources do *not* agree.
pub const AMIGA_DEFAULT_FOREGROUND: u8 = 9;

/// The Macintosh's default background: standard colour **9**, white.
///
/// One line of `mac/xzip.lst` settles both this and [`MAC_DEFAULT_FOREGROUND`],
/// and it is the same `(back << 8) | fore` idiom the Amiga interpreter uses
/// (see [`AMIGA_DEFAULT_BACKGROUND`], where the sources had to be weighed):
///
/// ```text
///   FUNCTION SetColor (fore, back: INTEGER): INTEGER;  { return back/fore defaults }
///   BEGIN
///     SetColor := (zWHITE*256) + zBLACK;   { Mac defaults: white under black }
/// ```
///
/// with `zWHITE = 9` and `zBLACK = 2` declared eleven lines above it. The same
/// function's fallbacks say it twice more — `IF fore = 1 THEN mcid :=
/// blackColor { default }` and `IF back = 1 THEN mcid := whiteColor` — because
/// colour 1 means "the default" (ZMSD §8.3.1), so asking for the default
/// foreground gets black and the default background white.
///
/// A white page is the Macintosh's whole visual signature and the opposite of
/// the Amiga's dark grey, so `honor_game_colours` is doing real work on this
/// profile: turning it off returns the user's theme, as ever.
pub const MAC_DEFAULT_BACKGROUND: u8 = 9;

/// The Macintosh's default foreground: standard colour 2, black. Same line,
/// same source — see [`MAC_DEFAULT_BACKGROUND`].
pub const MAC_DEFAULT_FOREGROUND: u8 = 2;

/// The Atari ST's default background: standard colour **9**, white.
///
/// Sourced exactly as the Macintosh's was, and it is the same `(back << 8) |
/// fore` idiom a third time — `st/xzip.c`, the interface half of Infocom's ST
/// XZIP, whose modification history ends "17 Sep 87 dbb … FROZEN Version A":
///
/// ```text
///   #define USE_DEF  1     /* use default ST color */
///   #define DEF_FORE 2     /* default ST foreground id = black */
///   #define DEF_BACK 9     /* default ST background id = white */
/// ```
///
/// The comments name the colours outright, so this needs no decoding, and
/// `_op_color` says it twice more the way the Amiga's and the Mac's do — colour
/// 1 means "the default" (ZMSD §8.3.1), so the file resolves it:
///
/// ```text
///   if (id1 == USE_DEF) id1 = DEF_FORE;
///   if (id2 == USE_DEF) id2 = DEF_BACK;
///   …
///   return ((DEF_BACK << 8) | DEF_FORE);   /* (used by 68K init) */
/// ```
///
/// **XZIP is the right interpreter to have read**: it is Infocom's Version 5
/// interpreter, so this is the program that actually painted *Beyond Zork* on an
/// Atari ST — the one story in the ST corpus whose behaviour this profile moves.
/// Its "Version A" is corroborated from inside the game, which answers VERSION
/// with "Atari ST Color Version A" once it is told 5.
pub const ST_DEFAULT_BACKGROUND: u8 = 9;

/// The Atari ST's default foreground: standard colour 2, black. Same file, same
/// three lines — see [`ST_DEFAULT_BACKGROUND`].
pub const ST_DEFAULT_FOREGROUND: u8 = 2;

/// The Apple II family's default background: standard colour **2, black**.
///
/// Sourced from Infocom's own Version 6 interpreter for the machine, the Apple
/// II YZIP — `apple/yzip/rel.15/zboot.asm`, which seeds the header before the
/// story can read it. `ZCLRWD EQU 44` is decimal 44, `$2C`, and ZMSD §8.3.3 puts
/// the default BACKGROUND at `$2C` and the foreground at `$2D`:
///
/// ```text
///   apple/yzip/rel.15/zboot.asm:54   lda #9   ; the color white is the foreground color
///   apple/yzip/rel.15/zboot.asm:55   sta ZBEGIN+ZCLRWD+1   ; show Z game too
///   apple/yzip/rel.15/zboot.asm:56   lda #2   ; black is the background color
///   apple/yzip/rel.15/zboot.asm:57   sta ZBEGIN+ZCLRWD     ; tell game about it
/// ```
///
/// The comments name the colours outright, so this needs no decoding — and
/// `machine.asm`'s `ZCOLOR` says it twice more, the way the Amiga's, the Mac's
/// and the ST's do, because colour 1 means "the default" (ZMSD §8.3.1) and the
/// file has to resolve it:
///
/// ```text
///   ; just do the background color - foreground is always white/black
///   …
///   ldx #1   ; use black as default back color
///   …
///   ldx #8   ; use white as default fore color
/// ```
///
/// (Both are pre-`dex` indices into `ZIPCOLOR`, which is zero-based from colour
/// 2: `#1` resolves to index 0, colour 2, and `#8` to index 7, colour 9.)
///
/// **One page for three machines.** `zboot.asm` is the whole family's boot path —
/// the `MACHINE:` routine picks the IIe's 2, the IIc's 9 or the IIgs's 10 *after*
/// these four lines run — so the IIe and IIc rows state this same pair from the
/// same source rather than from an analogy (SQ-0872).
///
/// **A genuinely BLACK page, and it is the only one here.** The Amiga's dark
/// grey (`$444`, see [`AMIGA_DEFAULT_BACKGROUND`]) is deliberately not black —
/// that distinction carried SQ-0740's window-0 gate — while the Macintosh and
/// the Atari ST both boot white.
pub const APPLE_DEFAULT_BACKGROUND: u8 = 2;

/// The Apple II family's default foreground: standard colour 9, white. Same four
/// lines, same source — see [`APPLE_DEFAULT_BACKGROUND`].
pub const APPLE_DEFAULT_FOREGROUND: u8 = 9;

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

/// Everything zvm knows about one ZMSD §11.1.3 machine.
///
/// The shape a machine's own interpreter drew its input cursor as (SQ-0873).
///
/// Three forms stored across the nine machines, and **none of them is what "invert the cell
/// under the cursor" gives** — which is what a terminal front-end draws by default
/// and what lanthorn drew before this. Measured off the captures in
/// `machine-screenshots/`, cell sizes included, so the proportions are the
/// machine's rather than a guess at them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CursorShape {
    /// One pixel wide and a line tall, in the gap AFTER the last glyph rather than
    /// over a cell — the Macintosh insertion caret.
    ///
    /// Measured on three captures across two Mac models: `mac-zork1.jpg` (Mac Plus,
    /// 16 rows — a JPEG, so that count is soft) and `mac-arthur.png` /
    /// `mac-zorkzero.png` (colour press, **15 rows**). A scan for isolated 1px-wide
    /// vertical runs finds exactly one candidate per frame — x=41 and x=99
    /// respectively, both 15 tall.
    ///
    /// Those two used to be JPEGs and used to read 13 rows; the PNGs that replaced
    /// them are 1:1 and read 15, which is one full `lineHeight` (SQ-0917). The
    /// caret is exactly one cell tall, and it is a fourth independent confirmation
    /// of that cell after the inverse PROLOGUE bar, the topic-row pitch and the
    /// colour prose.
    Bar,
    /// A solid block filling the cell, or nearly: 7x14 of a 7x16 cell on the Apple
    /// II (`appleiie-planetfall.png`), a full 8x16 on the Amiga
    /// (`amiga-spellbreaker.png`, `amiga-lurking.png`).
    Block,
    /// One scanline on the cell's bottom row — 8x1 on both Commodores
    /// (`c64-hitchhiker.png`, `c128-trinity.png`).
    Underscore,
    /// A full cell in the reverse of **whatever pair is on screen** — the caret
    /// the Version 6 interpreters draw on the Amiga and the IBM PC (SQ-0947).
    ///
    /// The one shape here that states no colour, and it is a claim about the
    /// interpreter rather than about the machine's palette. The other three name a
    /// [`PeriodLook::cursor_colour`] because the caret was that colour whatever the
    /// story did; this one tracks the story, so no RGB can be stored for it.
    ///
    /// **Two Amiga captures of two v6 games settle that it tracks**, which one
    /// could not: `amiga-zorkzero.png` draws the caret BLACK, an 8x15 block after
    /// `[MORE]` on Zork Zero's own grey `#A3A0A3` page, and `amiga-shogun.png`
    /// draws it WHITE, an 8x16 block after the `>` on Shogun's dark `#3A3C3A`. Two
    /// pairs, two carets, each the reverse of the pair beside it — and neither is
    /// the `#FF7E1C` orange the same machine's v3 interpreter draws (exactly 128
    /// pixels of it, one 8x16 cell, in both `amiga-spellbreaker.png` and
    /// `amiga-lurking.png`).
    ///
    /// `dos-arthur.png` is the IBM PC's, and it changes SHAPE rather than colour:
    /// a solid white 18x36 cell after `>exam` on the EGA blue page, where the same
    /// machine's v3 interpreter draws [`Self::Underscore`] across the bottom
    /// quarter of the cell (`dos-hitchhiker.png`).
    ///
    /// **The Macintosh is the control and does NOT change** — `mac-zorkzero.png`
    /// and `mac-shogun.jpg` draw the same [`Self::Bar`] its v3 capture does, which
    /// is why this is applied per machine in [`period_look_for`] rather than to
    /// every row at Version 6. Machines with no v6 capture decline, as everywhere
    /// else in this table.
    ReverseSpace,
}

/// How a machine's own interpreter set the status line apart from the story
/// (SQ-0873).
///
/// **Nine machines, four measured behaviours, and not one of them derives from the body
/// pair** — which is the finding that shaped [`PeriodLook`]. A field carrying only
/// a page and an ink could express none of the last three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StatusBand {
    /// The body pair swapped, across the full width. The Apple II and the
    /// Commodore 128.
    FullReverse,
    /// The body pair swapped, but only behind each RUN of text — the page shows
    /// between them.
    ///
    /// The Amiga, and the reason this variant exists. Row-censused across
    /// `amiga-spellbreaker.png` at mid-band: 47..182 white behind "Council
    /// Chamber", **376 px of page**, then 559..680 white behind "Score: 0/0".
    /// `dos-hitchhiker.png` is the same shape: 611 contiguous px of page run
    /// through the middle of its status row.
    ///
    /// **No row uses it, by the user's ruling** — both machines that measure this
    /// way draw a full-width reverse instead, because a band broken into pieces
    /// reads as damage in a terminal where it read as design on the original
    /// monitor. Kept for the same reason [`Self::Own`] is: the measurement is real
    /// and does not stop being real because we chose not to draw it.
    PerRun,
    /// A ground and ink of its own, which are neither the body pair nor its
    /// reverse.
    ///
    /// The Commodore 64: a black ground under grey characters where its body is
    /// white on grey. A true reverse would be a white ground with grey characters,
    /// and black is a colour the body never uses as a ground at all.
    Own {
        /// The band's ground.
        ground: (u8, u8, u8),
        /// The band's characters.
        ink: (u8, u8, u8),
    },
    /// No distinguishing ground; the band is separated from the story by rules.
    ///
    /// The Macintosh, which draws the status line in the body's own white-under-
    /// black and puts solid black rules above and below it.
    Ruled,
}

/// What a machine's screen LOOKED LIKE, for a story that has no opinion about it.
///
/// # Why this is a separate field and not more of `default_colours`
///
/// **Colour arrives with Version 5.** `set_colour` and the `$2C`/`$2D` header bytes
/// are v5+, so a v1-v4 story has no colour concept at all: it never sets, never
/// reads, never branches. Anything shown for one is the interpreter's presentation.
/// [`MachineProfile::default_colours`] is the opposite kind of claim — a fact the
/// story can read, sourced from Infocom's own code (`mac/xzip.lst`, `zboot.asm`,
/// `st/stx1.s`) — and the two must not be confused. Hence two fields.
///
/// **And the two genuinely differ.** The Amiga's row reports 12/9, a grey page under
/// white ink, from its v5-era interpreter; its v3 interpreter draws a BLUE page — the
/// one machine of the nine whose two answers are different colours, and the case this
/// field was split out for. The Commodore 64 is the same problem from the other side:
/// the grey page of its 1984 capture cannot be expressed as a §8.3.1 colour number at
/// all (2..9 contain no grey, and `clamp_default_colour` accepts 10..12 only for v6),
/// so reusing `default_colours` would have silently clamped it to black.
///
/// # Provenance, and the standard it does NOT meet
///
/// Every value here is **observed from an emulator capture in
/// `machine-screenshots/`**, row-censused rather than sampled, not read out of
/// Infocom's source. The rest of this table meets a documented-source standard and
/// this field does not, by the user's explicit decision (SQ-0873) to go with the
/// captures in hand rather than chase emulator names, versions and palette settings.
///
/// **One row's body pair is resolved rather than observed**, and the row says so
/// instead of storing one: see [`MachineLook::Resolved`]. A value that arrives here
/// from [`period_look_for`] is therefore the screen, whichever way it was reached.
///
/// What that costs is bounded and worth stating: the Amiga's `#074BA1` and the
/// Commodore 64's `#6C6C6C` are values a palette choice can move — `#074BA1` is not
/// a bit-replicated OCS value (those widen to `0x00, 0x11, ... 0xFF`), so the
/// underlying register is almost certainly Workbench's `$05A`, which replicates to
/// `#0055AA`; what is recorded is what the emulator drew. The other rows cannot
/// move: the Mac Plus is 1-bit, the C128's VDC is RGBI (0/85/170/255 exactly), and
/// the Apple II is monochrome — though "monochrome" there means a WHITE-phosphor
/// rendering, and green and amber monitors were as common.
///
/// # The Commodore 64 is the row this field argued into existence
///
/// Interpreter 8 had no row when it was first measured; see [`MACHINES`] for why
/// that was wrong. Its values come from `c64-zork1-solidgold.png`, whose banner
/// reads "Interpreter 8 Version J" — the machine naming its own number.
///
/// **Two C64 captures disagree, and the row follows the later one.**
/// `c64-hitchhiker.png` (1984) is a GREY page `#6C6C6C` under white ink with a
/// status band of `#6C6C6C` on `#000000` — neither the body pair nor its reverse,
/// which is the case [`StatusBand::Own`] exists for and the only measured instance
/// of it. The Solid Gold press (release 52, serial 871125) is black under white
/// with a plain full-width reverse. One machine, one publisher, two looks three
/// years apart: the period look is a property of the interpreter BUILD as much as
/// the machine, and this table has one row per machine. `Own` is kept because the
/// earlier build is evidence that a band need not be derivable, whether or not the
/// row that reported it still does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodLook {
    /// The body's ground.
    pub page: (u8, u8, u8),
    /// The body's characters.
    pub ink: (u8, u8, u8),
    /// How the status line was set apart. Never derivable from the pair above.
    pub status: StatusBand,
    /// The input cursor's shape.
    pub cursor_shape: CursorShape,
    /// The input cursor's colour, which on **one of the nine machines is neither
    /// the page nor the ink** — the Amiga's `#FF7E1C` orange, drawn over a blue
    /// page under white ink. Every other row's caret is that row's own ink, which
    /// is what a consumer would guess; the Amiga is why it cannot be derived.
    ///
    /// Says nothing under [`CursorShape::ReverseSpace`], which is a caret with no
    /// colour to state; [`period_look_for`] parks the ink here so a consumer that
    /// reads the field without the shape gets the pair's own reverse rather than
    /// the previous generation's orange.
    pub cursor_colour: (u8, u8, u8),
}

/// What a row STORES about its screen, which on one machine cannot be a
/// [`PeriodLook`] at all (SQ-0983).
///
/// **A machine whose body pair is derived must have nowhere to write a second
/// one.** The IBM PC's row used to store `#0000AA` under `#AAAAAA` *and* have
/// [`period_look_for`] resolve the pair afresh out of
/// [`MachineProfile::default_colours`], overwriting both — so the stored constant
/// was never on anybody's screen, and correcting its numbers would only have left
/// the same trap for the next reader. This enum removes the field instead of
/// fixing the value: a [`Self::Resolved`] row states the two decisions a palette
/// cannot make, and no page, ink or cursor colour can be written beside them.
///
/// **The pair is unrepresentable as a constant anyway**, which is what settles it.
/// Infocom's two IBM interpreters disagree about white — XZIP resolves colour 9 to
/// `#ADADAD` and YZIP to `#FFFFFF` — so the machine's ink depends on the story's
/// Version, and one stored value cannot be true for both. The stored pair was not
/// merely stale; it was answering a question that has two answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MachineLook {
    /// Every value read off a capture in `machine-screenshots/`, which is what a
    /// machine whose screen is simply a fact states. Eight of the nine rows.
    Measured(PeriodLook),
    /// The body pair is this machine's own palette resolving the pair the row
    /// already states in [`MachineProfile::default_colours`] — one table lookup,
    /// not a second measurement to drift from the first — and the caret is that
    /// same ink, which is how the capture measured it.
    ///
    /// So the screen a v1–v5 story is painted on and the colour a v6 story gets
    /// from `@set_colour(9)` cannot disagree: they are the same lookup, through the
    /// palette [`palette_for`] picks for the story's Version.
    ///
    /// **The Amiga is why this is not what every row does.** Its `default_colours`
    /// report 12/9, a GREY page, from its v5-era interpreter, while its v3
    /// interpreter draws a BLUE one — the divergence
    /// `the_period_look_is_not_the_default_pair` exists to pin. Deriving its look
    /// from its pair would overwrite a measured blue screen with a grey one, so it
    /// stays [`Self::Measured`], and the choice is now the ROW's rather than a
    /// palette match buried in a function.
    ///
    /// A row that states this must state `default_colours` too, and both numbers
    /// must resolve in its palette; `a_resolved_row_states_the_pair_it_resolves`
    /// is what holds that.
    Resolved {
        /// How the status line was set apart — measured, and never derivable from
        /// the pair.
        status: StatusBand,
        /// The input cursor's shape — measured. Its COLOUR is the resolved ink.
        cursor_shape: CursorShape,
    },
}

/// The Apple II family's period look, shared by interpreters 2, 9 and 10.
///
/// Measured on `appleiie-planetfall.png` (Planetfall r29, v3, 560x384 = 80 columns
/// at 1x2). Shared across the three rows for the same reason they already share
/// [`APPLE_DEFAULT_BACKGROUND`] and [`APPLE_DEFAULT_FOREGROUND`]: one Infocom
/// interpreter, one 80-column text screen. The capture cannot say which of the
/// three machines ran it, and treating that as three separate declines would lose a
/// measurement to a distinction the interpreter itself does not draw.
///
/// White on black is the white-monitor rendering; see [`PeriodLook`].
const APPLE_PERIOD_LOOK: PeriodLook = PeriodLook {
    page: (0x00, 0x00, 0x00),
    ink: (0xFF, 0xFF, 0xFF),
    status: StatusBand::FullReverse,
    cursor_shape: CursorShape::Block,
    cursor_colour: (0xFF, 0xFF, 0xFF),
};


/// How a machine renders ZMSD §8.7.1's **Italic** style bit (SQ-1028).
///
/// The standard leaves this open in as many words: *"An interpreter need not
/// provide Bold or Italic (even for font 1) and is free to interpret them broadly.
/// (For example, rendering bold-face by changing the colour, or rendering italic
/// with underlining.)"* — §8.7.1, verified against
/// <https://inform-fiction.org/zmachine/standards/z1point1/sect08.html>. So neither
/// answer is a compliance question; the only question is what the machine DID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum V6Emphasis {
    /// A rule along the bottom of the cell, the text's own colour, abutting the
    /// glyphs with no gap.
    ///
    /// **Both machines Infocom shipped a Version 6 interpreter for**, measured on
    /// the same frame of the same game. `machine-screenshots/amiga-shogun-game.png`
    /// draws `Erasmus` in "This is the bridge of the Erasmus, a Dutch merchant" with
    /// a solid rule under that word and nothing under the words beside it; row by
    /// row at the capture's 2x, the glyph ink runs 336..349 and the rule is 350..351
    /// against a 16-row line pitch — the cell's LAST ROW, directly below the
    /// letters. `mac-shogun.jpg` underlines the same word on the same frame, and
    /// that machine had real italics available and did not use them.
    Underline,
    /// A synthesised slope — the top of the glyph sheared one column right.
    ///
    /// What lanthorn has always drawn, and what every row keeps until a capture says
    /// otherwise. No PC capture in `machine-screenshots/` shows an emphasised run —
    /// `dos-shogun.png` is a title screen — so the IBM PC is UNMEASURED rather than
    /// known to slope, and a bare story file has no machine to be faithful to
    /// anyway.
    Slope,
}

/// What a Version 6 window does with text that reaches its right margin — see
/// [`V6WrapRegime`], which is where the machines disagree about how to choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum V6TextFlow {
    /// Break after the last WORD that fits, and start the next line at the left
    /// margin (ZMSD §8.8.3.1.2.2).
    WordWrap,
    /// Break after the last CHARACTER that fits (ZMSD §8.8.3.1.2.2).
    CharWrap,
    /// *"characters will be printed until no more can be fitted in without hitting
    /// the right margin, at which point the cursor will move to the right margin
    /// and stay there, so that any further text will be ignored"* — §8.8.3.1.1.
    ///
    /// The margin is the WINDOW's, not the screen's: §8.8.3 says *"all text and
    /// graphics plotting is always clipped to the current window"*.
    CharClip,
}

/// How a machine decides which [`V6TextFlow`] a window is in (SQ-1071).
///
/// # The two machines Infocom shipped a Version 6 interpreter for both ignore the
/// window attributes
///
/// §8.8.3.1.2.2's commentary tabulates what Infocom's own interpreters did, and
/// it is not what §8.8.3.1.1 prescribes — verified against
/// <https://inform-fiction.org/zmachine/standards/z1point1/sect08.html>:
///
/// ```text
///                   Apple II      MSDOS         Macintosh   Amiga        Standard
/// A0 off,  A3 off   char clip(LR) char clip()   ---         ---          char clip(LR)
/// A0 off,  A3 on    char clip(LR) char clip(LR) ---         ---          char clip(LR)
/// A0 on,   A3 off   word wrap     char wrap     ---         ---          char wrap
/// A0 on,   A3 on    word wrap     word wrap     ---         ---          word wrap
/// buffer_mode off   ---           ---           char wrap   char clip(L) ---
/// buffer_mode on    ---           ---           word wrap   word wrap    ---
/// ```
///
/// *"Here `---` means that the interpreter **ignores** the given state."* So on
/// the Macintosh and the Amiga attributes 0 and 3 say nothing at all, and the
/// `buffer_mode` opcode — which defaults ON, and which the V6 story files touch
/// exactly once, to trickle out a "Please wait..." — decides instead. Both
/// machines therefore WORD WRAP whatever the window's wrapping attribute says.
///
/// Measured, on the frame that prompted this: Shogun's InvisiClues clears window
/// 0's wrapping attribute (`@window_style(win=0, flags=0b0001, op=2)`) and prints
/// a clue longer than the 500-px window it just declared.
/// `machine-screenshots/amiga-shogun-hintshown.png` and
/// `machine-screenshots/mac-shogun-hintshown.png` both show it **word-wrapped
/// onto a second line at the left margin**, at each machine's own break point —
/// the Amiga after `from`, the Macintosh after `you`, which is its proportional
/// Geneva in a wider box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum V6WrapRegime {
    /// §8.8.3.1.1 and §8.8.3.1.2.2 as written: attribute 0 decides whether text
    /// breaks at all, attribute 3 whether it breaks by word.
    ///
    /// The row every machine outside the table above keeps, and the row the
    /// **Apple II** measurably has. The **IBM PC** keeps it too rather than the
    /// MSDOS column, because the standard names that column's two departures as
    /// BUGS and nothing in `machine-screenshots/` reaches either — reproducing a
    /// defect we cannot observe would be reputation, not evidence.
    Attributes,
    /// Attributes 0 and 3 are ignored; the `buffer_mode` opcode decides.
    /// **Macintosh and Amiga.**
    ///
    /// `unbuffered` is the flow with buffering OFF, and it rides along because the
    /// two machines disagree about it — the Macintosh char-wraps, the Amiga
    /// clips. Nothing in the corpus reaches it (buffer_mode is on for every frame
    /// measured), so it is stated from the table rather than measured.
    ///
    /// The Amiga's entry is `char clip(L)` — the LEFT margin respected and the
    /// right one not — which the standard itself calls a probable bug. We state
    /// [`V6TextFlow::CharClip`], clipping at both, for the same reason the IBM PC
    /// keeps `Attributes`: an unobservable bug is not worth a variant.
    BufferMode {
        /// The flow this machine uses when `buffer_mode` is off.
        unbuffered: V6TextFlow,
    },
}

impl V6WrapRegime {
    /// Which flow a window with these `attributes` (ZMSD §8.8.3.1) is in, on a
    /// machine whose `buffer_mode` (§15) is as given.
    pub fn flow(self, attributes: u16, buffer_mode: bool) -> V6TextFlow {
        match self {
            V6WrapRegime::Attributes => match (attributes & 0b0001 != 0, attributes & 0b1000 != 0) {
                (false, _) => V6TextFlow::CharClip,
                (true, false) => V6TextFlow::CharWrap,
                (true, true) => V6TextFlow::WordWrap,
            },
            V6WrapRegime::BufferMode { unbuffered } => {
                if buffer_mode {
                    V6TextFlow::WordWrap
                } else {
                    unbuffered
                }
            }
        }
    }
}

/// The coordinate space a machine's own TYPEFACE bitmaps are authored in — which
/// is not always the space its ARTWORK is authored in (SQ-1039).
///
/// # Why this is a separate fact from `art_scale`
///
/// `art_scale` is the ARCHIVE's: how many native pixels one picture pixel becomes
/// (SQ-0790). The Version 6 cell is the MACHINE's: what the story is told
/// (SQ-0917, SQ-1013). On most presses those two never meet, because the face IS
/// the cell and neither is scaled. A machine that ships a real proportional
/// typeface makes them meet, and then one native pixel means two different things
/// in one frame — see `CLAUDE.md`'s art-versus-text density table.
///
/// It is unobservable on any row that draws with no face at all, and those rows
/// state [`Self::Native`] because that is the answer that changes nothing, NOT
/// because anything measured them. Where a face IS admitted the space governs
/// every one of them — a proportional face's declared line and advances, and a
/// FIXED face's blit, which is how topaz 8's eight rows fill the Amiga's
/// sixteen-row cell (SQ-1053).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum V6FaceSpace {
    /// The face is drawn in the ARCHIVE's picture space, so one face pixel is one
    /// art pixel and scales with the artwork.
    ///
    /// **The Amiga's own RELEASES**, and measured rather than assumed: Arthur's
    /// `char.data` advance table averages 5.21 face px per character while
    /// `machine-screenshots/amiga-arthur-text.png` measures 4.70 ART px per
    /// character — which agree at 1:1 and are out by a factor of two at 2:1. Its
    /// ten face rows are a ten-row text pitch in the machine's own 320x200 frame
    /// and twenty in the 2x captures, so on a press that doubles onto the 640x400
    /// unit screen the declared line really is 20.
    Art,
    /// The face is drawn in NATIVE device pixels, whatever the artwork does around
    /// it.
    ///
    /// **The Macintosh**, where the split is not academic: the colour press draws
    /// `CPic.data` at 320x200 with `art_scale` (2, 2) while painting text at one
    /// native pixel per face pixel. Scaling a face by the art scale there declares
    /// Geneva 12 — fifteen rows — as thirty, and the monochrome press hides it
    /// because `Pic.data` is 480x300 at (1, 1) and 15 x 1 is still 15.
    Native,
    /// The face is drawn in the machine's own **hires text space**: one face pixel
    /// is one native pixel across, and as tall as the artwork's own doubling makes
    /// a row.
    ///
    /// **The Amiga's system topaz**, and the reason a face space cannot be read off
    /// the machine's row alone (SQ-1053). A game's face and the operating system's
    /// are authored in different spaces on the same machine: *Arthur* draws
    /// `char.data` in its 320-wide PICTURE space ([`Self::Art`]), while topaz is a
    /// `FONTS:`/ROM face drawn in the 640x200 hires mode the interpreter ran in.
    ///
    /// Measured on `machine-screenshots/amiga-shogun-game.png`, over `Erasmus` in
    /// "This is the bridge of the Erasmus": the glyph band holds **10 distinct
    /// scanlines across a 20-row pitch** — every face row drawn twice — and the
    /// underline spans 60 px over 7 characters, so **~8 native pixels per
    /// character** across. An 8x8 face at (1, 2) lands exactly on the 8x16 cell
    /// this machine declares.
    ///
    /// The vertical two is the artwork's own, not a second constant: the frame is
    /// 200 rows and a square-pixel screen doubles it to 400, which is precisely
    /// what `art_scale.1` already says. An undoubled rendition therefore answers
    /// (1, 1) here and needs no special case.
    Hires,
}

impl V6FaceSpace {
    /// Native pixels per FACE pixel, given the archive's art scale.
    pub fn text_scale(self, art_scale: (u32, u32)) -> (u32, u32) {
        match self {
            V6FaceSpace::Art => art_scale,
            V6FaceSpace::Native => (1, 1),
            V6FaceSpace::Hires => (1, art_scale.1),
        }
    }
}

/// How a machine's own SYSTEM body face is named on that machine's boot media
/// (SQ-1037).
///
/// # Why a machine states a face it did not ship with the game
///
/// The two machines Infocom wrote a Version 6 interpreter for both drew body text
/// with a face that lives on the OPERATING SYSTEM, not on the game disk.
/// `mac/xzip.lst` says `ZSTD: TextFont (stdFont)` with `stdFont := geneva`, and
/// Geneva is in the System file on every Macintosh and on no Infocom platter
/// (SQ-1036); the Amiga's topaz is in ROM and in a Workbench `FONTS:` drawer, not
/// on Arthur's floppy. So the release's own medium can answer for the ALTERNATE
/// — the Macintosh ships `FONT` 524, Monaco 12, which is exactly its 7x15 cell —
/// and cannot answer for the body face at all.
///
/// This names what to go looking for when the player supplies the machine's own
/// boot media. It is a NAME and nothing more: whether the face that comes back may
/// actually be drawn is one question asked in one place, the host's `fit`.
///
/// # `None` is the answer for every other row
///
/// Not "unmeasured" — no other machine in this table has a Version 6 interpreter
/// whose body face could be recovered from a boot disk, so there is nothing to
/// name. A row that states `None` reads a supplied disk and finds nothing, which
/// is the same outcome as having no disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum V6SystemFace {
    /// A Macintosh font FAMILY number. A `FONT` resource id is
    /// `family * `[`MAC_FONT_FAMILY_STRIDE`]` + point size`, so a family names a
    /// RUN of ids rather than one, and the size is the host's to choose against
    /// the machine's declared line height.
    ///
    /// Geneva is family 3 — `FONT` 396 is Geneva 12, against the 524 (family 4,
    /// Monaco 12) the games ship.
    MacFamily(i16),
    /// An AmigaDOS `FONTS:` drawer name, as `fonts/<name>/<size>` spells it.
    ///
    /// `topaz`, which is the Amiga's system face and what *Shogun* and *Zork Zero*
    /// took on that machine, neither having shipped one of their own.
    ///
    /// It is also the drawer a **Kickstart ROM** spells, because `blorb` names a
    /// ROM face `<face>/<size>` for exactly this reason: the machine's real topaz
    /// 8 is in ROM and on no floppy, and a face out of ROM must be ranked by the
    /// same rule as a face out of `FONTS:` rather than by a second one (SQ-1053).
    AmigaDrawer(&'static str),
}

impl V6SystemFace {
    /// The space a face found THIS way is authored in — which is not always the
    /// space the same machine's RELEASE faces are authored in.
    ///
    /// # Why the provenance decides and not the row
    ///
    /// SQ-1053. The Amiga has two faces wanting two different scales at once. Its
    /// releases author theirs in the picture space — *Arthur*'s `char.data`, whose
    /// ten face rows are the twenty-row line the captures measure — while the
    /// operating system's topaz is drawn in the 640x200 hires mode the interpreter
    /// ran in and wants (1, 2). One number per machine could express only one of
    /// them, and the one it expressed would silently mis-scale the other; see
    /// [`V6FaceSpace::Hires`] for the measurement.
    ///
    /// The Macintosh's two agree, which is why this went unnoticed until a machine
    /// had a system face to read: Geneva out of a System file and Monaco off a
    /// game disk are both painted at one native pixel per face pixel.
    pub fn face_space(self) -> V6FaceSpace {
        match self {
            V6SystemFace::MacFamily(_) => V6FaceSpace::Native,
            V6SystemFace::AmigaDrawer(_) => V6FaceSpace::Hires,
        }
    }
}

/// A Macintosh `FONT`/`NFNT` resource id is `family * 128 + point size`.
///
/// Stated here because it is what makes [`V6SystemFace::MacFamily`] a family
/// rather than an id; the arithmetic itself belongs to whoever reads the resource
/// fork.
pub const MAC_FONT_FAMILY_STRIDE: i16 = 128;

/// Geneva — the Macintosh's system body face, `stdFont` in `mac/xzip.lst`.
pub const MAC_GENEVA_FONT_FAMILY: i16 = 3;
/// A row is the machine's *bundle*: the byte it writes into `$1E`, the page and
/// ink it reports in `$2C`/`$2D`, the palette its colour numbers resolve through,
/// and the §8.3 screen rules the standard gives it by name. Any member a row
/// cannot source is declined (`None` / `false`) rather than guessed — see the
/// module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MachineProfile {
    /// The §11.1.3 number this machine writes into header `$1E`.
    pub number: u8,
    /// The machine's name, as §11.1.3 spells it. Diagnostic (`zvm-cli` names the
    /// machine it is presenting as); never shown to the story.
    pub name: &'static str,
    /// The `(background, foreground)` standard colour numbers this machine
    /// reports in header `$2C`/`$2D` (ZMSD §8.3.3), or `None` to report the host
    /// terminal's own colours.
    ///
    /// `None` is a *decline*, and it means two different things on two rows. The
    /// IBM PC declines because a PC in a terminal IS the player's terminal, so
    /// "default" should mean what the player sees. The Commodore 128 declines
    /// because nothing in hand states the pair Infocom's Commodore interpreter
    /// reported for a v5 story (SQ-0869).
    ///
    /// That second decline stands, but its old wording no longer does. It used to
    /// read that "the machine's famous light-blue-on-blue boot screen is the
    /// hardware's reputation rather than the interpreter's evidence"; SQ-0873 then
    /// measured the machine, and Infocom's interpreter draws light CYAN on BLACK,
    /// which is neither the reputation nor anything like it. What that measurement
    /// cannot do is fill this field, because it was taken on *Trinity* — a v4
    /// story, which has no colour concept for a default pair to be the default OF.
    /// It lives in [`MachineProfile::period_look`] instead, which is the whole
    /// point of that field being separate.
    pub default_colours: Option<(u8, u8)>,
    /// The palette this machine's colour NUMBERS resolve to true colours through
    /// (ZMSD §8.3.1.1, which makes it an interpreter choice rather than a law).
    ///
    /// Only the Amiga loaded a palette of its own. The Macintosh, the Atari ST
    /// and the Apple II each asked for §8.3.1's eight colours and meant them —
    /// the readings are quoted on [`Palette`] and in `app::interpreter`. Where a
    /// machine's hardware palette is famous but its interpreter never states one
    /// (the ST's 512, the Apple's double hi-res, the C64's sixteen), this stays
    /// [`Palette::Standard`]: reputation is not evidence.
    pub palette: Palette,
    /// §8.3's Amiga rule: this machine has **one pair of text pens for the whole
    /// screen**, so a `set_colour` moves the screen rather than a window.
    ///
    /// ZMSD §8.3 states it of the Amiga by name, and Infocom's own `amiga/yzip3.c`
    /// says the two text colours "are now 'global', meaning they *can't* be
    /// changed for a single word on the screen, or for a certain window".
    /// [`crate::screen::amiga_global_colour_pair`] carries the full rule; this
    /// flag is the machine half of it, and no other row sets it.
    pub global_colour_pens: bool,
    /// The Version 6 CHARACTER CELL this machine declares — header `$26`/`$27`,
    /// `(width, height)` in pixels (SQ-0917, SQ-1013).
    ///
    /// **A DECLARED metric, not a drawn advance.** It is what the STORY IS TOLD,
    /// and a machine that painted proportionally still declared a fixed one: the
    /// Macintosh's `mac/xzip.lst` sets `colWidth := 7; lineHeight := 15` while
    /// drawing Geneva 12, and a host that declared anything else would be lying to
    /// the story. How ink actually lands is the HOST's business — see
    /// [`crate::screen::V6Cell`] for that boundary.
    ///
    /// It lives here because it is machine knowledge, and an embedder that gets
    /// this machine's interpreter number from zvm should not have to rediscover
    /// its cell somewhere else.
    pub v6_cell: V6Cell,
    /// The space the typefaces this machine's own RELEASES ship are authored in —
    /// see [`V6FaceSpace`], which is where the reasoning and the measurements are.
    ///
    /// **The release's, not every face this machine can draw** (SQ-1053). A
    /// machine's OPERATING SYSTEM face is a separate claim and the Amiga answers
    /// the two differently: *Arthur* authors `char.data` in the picture space and
    /// topaz is drawn in the hires text space. That half lives on
    /// [`V6SystemFace::face_space`], beside the name it belongs to.
    ///
    /// It sits beside [`Self::v6_cell`] because it is the same kind of claim: an
    /// embedder that gets this machine's interpreter number and cell from zvm
    /// should not have to rediscover, somewhere else, whether a face it admits
    /// scales with the artwork.
    pub v6_release_face_space: V6FaceSpace,
    /// What this machine's own SYSTEM body face is called on its boot media, or
    /// `None` where the machine has none to name — see [`V6SystemFace`].
    ///
    /// Beside the cell and the face space for the third time and the same reason:
    /// an embedder holding this machine's interpreter number should not have to
    /// rediscover, somewhere else, which typeface the machine actually painted
    /// prose with.
    pub v6_system_face: Option<V6SystemFace>,
    /// How this machine chooses what a window does with text that reaches its
    /// right margin — see [`V6WrapRegime`], which carries §8.8.3.1.2.2's table of
    /// what Infocom's own interpreters did and the captures that confirm it.
    ///
    /// Beside the cell and the face spaces for the same reason they are beside
    /// each other: it is machine knowledge, and an embedder holding this
    /// machine's interpreter number should not have to rediscover elsewhere
    /// whether this machine reads the window's wrapping attribute at all.
    pub v6_wrap_regime: V6WrapRegime,
    /// How this machine draws §8.7.1's Italic bit — see [`V6Emphasis`], which holds
    /// the measurements and the standard's own licence to choose.
    pub v6_emphasis: V6Emphasis,
    /// The Version 6 standard window this machine presents, in pixels, or `None`
    /// where the machine states none.
    ///
    /// **This is the machine's own answer, and it is the LAST link of a chain the
    /// host resolves** — a story's container and its artwork both outrank it. The
    /// standard Macintosh's monochrome `Pic.data` states the 480x300 screen it was
    /// drawn for, so this 320x200 never applies to that press (SQ-0838). Reading
    /// resource files is the host's business; knowing what an Amiga presented is
    /// this table's.
    pub v6_std_window: Option<(u16, u16)>,
    /// This machine's [`default_colours`](Self::default_colours) are not advice
    /// about a terminal — they **are** the Version 6 screen, the ground every
    /// window that names no colour of its own is read on.
    ///
    /// Two machines answer, and this flag is what
    /// [`crate::screen::machine_screen_pair`] reads (SQ-0846, SQ-0872). The Amiga
    /// for §8.3's own reason — one pair of pens for the whole screen, shared and
    /// unmoving — and the Macintosh for a plainer one: a white page under black
    /// ink was what a Mac window WAS, and `mac/xzip.lst` states it outright.
    ///
    /// The Apple II rows do **not** set it, and that is scope rather than a
    /// finding: their black page is as real as the Mac's white one, but no Apple
    /// v6 frame has been measured against it, and SQ-0846's evidence was a
    /// measured screen rather than a plausible one. Flipping this flag is what
    /// adding them would take.
    pub v6_screen_page: bool,
    /// This machine shows the whole screen through **one palette**, so loading a
    /// picture's colours recolours everything already drawn — border art
    /// included, not just the pictures an archive declares adaptive.
    ///
    /// A v6 framebuffer holds palette INDICES. What that means for the screen
    /// depends on how many colour registers the machine has, and the corpus
    /// splits cleanly:
    ///
    /// * **The Amiga has one set.** Measured on `James Clavell's Shogun.adf`
    ///   (release 295, serial 890321), whose `Pic.data` declares NO adaptive
    ///   pictures and gives all 48 a palette of their own: picture 3 is the
    ///   ornate border and its own table is gold (`#CCAA66`), picture 7 the
    ///   storm on deck (blues — `#99BBDD`, `#334499`) and picture 8 below decks
    ///   (reds and creams — `#CC0000`, `#CCAA88`). On the real machine the
    ///   border is drawn once and is blue-and-white in the storm and red-on-cream
    ///   below decks, because the scene's palette is the screen's palette.
    /// * **The MCGA does not, and the SAME GAME proves it.** Shogun's DOS press
    ///   leaves its side panels one colour throughout, where the Amiga's follow
    ///   the scene — one story, one border, two machines, two behaviours, which
    ///   is as clean a control as this corpus offers. The reason is the hardware:
    ///   the MCGA's DAC has 256 entries and Infocom used them. Arthur's map
    ///   screen lays down seven pictures carrying THREE distinct palettes at
    ///   once and each keeps its own; forcing one onto all seven is what SQ-0881
    ///   fixed — grey ground, rainbow scrolls.
    /// * **EGA and CGA never ask**: a hardware table outranks any loaded palette
    ///   (SQ-0794), so the question does not arise for them.
    ///
    /// **Only the Amiga sets it, and the others are DECLINED rather than
    /// assumed** — the same standard as the rest of this table. The Macintosh
    /// and the Atari ST plausibly had one screen palette too, but no frame of
    /// either has been measured showing a border following a scene, and a
    /// plausible-looking inference is exactly what this table refuses. Turning
    /// one on is one measurement's work: find a title whose border art and scene
    /// art carry different palettes, and look.
    pub one_screen_palette: bool,
    /// What this machine's screen LOOKED like, for a story with no opinion about
    /// it — presentation, not a fact the story can read. See [`PeriodLook`],
    /// which is also where the provenance and its limits are stated.
    ///
    /// `None` is a decline for want of a capture, not a claim that the machine had
    /// no look — and **no row declines any more**. The IBM PC's arrived with
    /// `dos-hitchhiker.png` (SQ-0873/SQ-0928) and the Atari ST's, the last one,
    /// with `st-zork1.png` (SQ-0933). The variant stays because the next machine
    /// added to this table will start out unmeasured, and stating a look by
    /// inference is what `the_machines_with_no_capture_decline` exists to stop.
    ///
    /// A row states a [`MachineLook`] rather than a [`PeriodLook`] because one
    /// machine's body pair is not a stored value at all — see that type.
    pub period_look: Option<MachineLook>,
}

/// Every §11.1.3 machine zvm models, in number order.
///
/// **The gaps are deliberate and each is a decline.** 1 (DECSystem-20) is absent
/// because it is what declining a number already falls through to, and whether it
/// deserves a bundle of its own or is honestly "a terminal, the same as the IBM
/// PC" is a decision rather than a datum. 11 (Tandy Color) is absent because there
/// is no fixture and no sourced constant, and anything written here would be
/// guesswork — better absent than invented.
///
/// **8 (Commodore 64) used to be absent and should not have been.** The stated
/// reason was that a `.d64` is a 1541 image both Commodore machines read, so the
/// medium cannot choose between 7 and 8 — which is true, and is not a reason for a
/// row to be missing. ProDOS cannot choose between 2, 9 and 10 either, and all
/// three of those have rows; a medium that names a family rather than a machine
/// means the number is asked for rather than inferred, not that the machine goes
/// unmodelled. The other half of the old reason — no Infocom Commodore interpreter
/// read for a v5 pair — is equally true of the Commodore 128, which has a row and
/// declines [`MachineProfile::default_colours`]; declining is what a row is FOR.
/// SQ-0873 then measured the C64's period look, giving it more evidence than
/// several rows that already existed.
///
/// [`machine`] answering `None` is therefore meaningful: it says "this number
/// names a machine I do not model", which a front-end can report instead of
/// silently dressing the story as an IBM PC.
/// The Macintosh's Version 6 character cell — `mac/xzip.lst`'s
/// `colWidth := 7; lineHeight := 15 {16}` (SQ-0917).
///
/// **A declared metric, not a drawn advance.** The machine painted proportional
/// Geneva 12 and still told the story 7; see [`MachineProfile::v6_cell`].
pub const MACINTOSH_V6_CELL: V6Cell = V6Cell::new(7, 15);

/// The Macintosh's Version 6 standard window, `GFXAM_X`/`GFXAM_Y` doubled.
///
/// The standard Macintosh's OTHER window — 480x300, `GFXMAC_X`/`GFXMAC_Y`, "1.5 x
/// Amiga sizes" — belongs to the monochrome archive and arrives with it, ahead of
/// this (SQ-0838).
pub const MACINTOSH_STD_WINDOW: (u16, u16) = (320, 200);

/// The Amiga's Version 6 standard window: 320x200 art on a 640x200 **hi-res,
/// non-interlaced** screen, which a square-pixel display shows as 640x400.
///
/// Infocom's own `amiga/yzip1.c` opens one `CUSTOMSCREEN` with `ViewModes = HIRES`
/// and `AM_XSIZ 640` / `AM_YSIZ 200`; `LACE`/`INTERLACE` appear nowhere in any
/// Amiga source in that repository and the literal `400` never occurs. A hi-res
/// Amiga pixel is half as wide as it is tall, so the display doubles the art
/// horizontally and a modern square-pixel screen doubles it vertically as well —
/// which is why the host's `art_scale` for this machine is (2, 2) and not (2, 1)
/// (SQ-1023).
///
/// It is also the resolution every Infocom Blorb's `Reso` chunk declares, those
/// Blorbs being Amiga conversions — so asserting it restores exactly the scaling a
/// Blorb-sourced copy of the same game already gets.
pub const AMIGA_STD_WINDOW: (u16, u16) = (320, 200);

pub const MACHINES: &[MachineProfile] = &[
    MachineProfile {
        number: APPLE_IIE_INTERPRETER_NUMBER,
        name: "Apple IIe",
        default_colours: Some((APPLE_DEFAULT_BACKGROUND, APPLE_DEFAULT_FOREGROUND)),
        palette: Palette::Standard,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: false,
        v6_cell: V6Cell::DEFAULT,
        v6_release_face_space: V6FaceSpace::Native,
        v6_system_face: None,
        v6_wrap_regime: V6WrapRegime::Attributes,
        v6_emphasis: V6Emphasis::Slope,
        v6_std_window: None,
        period_look: Some(MachineLook::Measured(APPLE_PERIOD_LOOK)),
    },
    MachineProfile {
        number: MACINTOSH_INTERPRETER_NUMBER,
        name: "Macintosh",
        default_colours: Some((MAC_DEFAULT_BACKGROUND, MAC_DEFAULT_FOREGROUND)),
        palette: Palette::Standard,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: true,
        v6_cell: MACINTOSH_V6_CELL,
        v6_release_face_space: V6FaceSpace::Native,
        // `mac/xzip.lst`: `ZSTD: TextFont (stdFont)` with `stdFont := geneva`.
        // The games ship Monaco (family 4) as their ZMONO alternate and no
        // Geneva at all, so this is only ever answered by a System disk the
        // player supplies (SQ-1036, SQ-1037).
        v6_system_face: Some(V6SystemFace::MacFamily(MAC_GENEVA_FONT_FAMILY)),
        // Measured on `mac-shogun.jpg`, and the interesting row: this machine HAD
        // real italics and underlined anyway.
        // §8.8.3.1.2.2: this machine ignores attributes 0 and 3 and follows
        // `buffer_mode`; unbuffered it CHAR-wraps. `mac-shogun-hintshown.png`.
        v6_wrap_regime: V6WrapRegime::BufferMode { unbuffered: V6TextFlow::CharWrap },
        v6_emphasis: V6Emphasis::Underline,
        v6_std_window: Some(MACINTOSH_STD_WINDOW),
        // mac-zork1.jpg: Zork I r88/840726 on a Mac Plus, screen 512x342. A 1-bit
        // screen, so no palette can move these; the status line is set apart by
        // rules rather than by ground, and the caret is 1px by the line height.
        period_look: Some(MachineLook::Measured(PeriodLook {
            page: (0xFF, 0xFF, 0xFF),
            ink: (0x00, 0x00, 0x00),
            status: StatusBand::Ruled,
            cursor_shape: CursorShape::Bar,
            cursor_colour: (0x00, 0x00, 0x00),
        })),
    },
    MachineProfile {
        number: AMIGA_INTERPRETER_NUMBER,
        name: "Amiga",
        default_colours: Some((AMIGA_DEFAULT_BACKGROUND, AMIGA_DEFAULT_FOREGROUND)),
        palette: Palette::Amiga,
        global_colour_pens: true,
        one_screen_palette: true,
        v6_screen_page: true,
        v6_cell: V6Cell::DEFAULT,
        // The one row whose RELEASES are not Native, and the one row with a
        // typeface to measure — see `V6FaceSpace::Art`. Its SYSTEM face is a
        // different space again (`V6SystemFace::face_space`), which is the whole
        // of SQ-1053.
        v6_release_face_space: V6FaceSpace::Art,
        // The Amiga's system face, in ROM and in a Workbench `FONTS:` drawer.
        // *Shogun* and *Zork Zero* shipped no face of their own on that machine
        // and took this one; *Arthur* ships `char.data` and outranks it.
        v6_system_face: Some(V6SystemFace::AmigaDrawer("topaz")),
        // §8.8.3.1.2.2: this machine ignores attributes 0 and 3 and follows
        // `buffer_mode`; unbuffered it CLIPS. `amiga-shogun-hintshown.png`.
        v6_wrap_regime: V6WrapRegime::BufferMode { unbuffered: V6TextFlow::CharClip },
        v6_emphasis: V6Emphasis::Underline,
        v6_std_window: Some(AMIGA_STD_WINDOW),
        // amiga-spellbreaker.png (r87/860904) and amiga-lurking.png, both v3 and
        // both giving the identical palette in 7-8 exact colours. Note the page
        // is BLUE where default_colours above reports 12/9, a grey — the v3 and
        // v5 interpreters differ, which is the whole reason this field exists.
        period_look: Some(MachineLook::Measured(PeriodLook {
            page: (0x07, 0x4B, 0xA1),
            ink: (0xFF, 0xFF, 0xFF),
            // SQ-0873: the capture reverses PER RUN — 376 px of page show between
            // "Council Chamber" and "Score: 0/0" on `amiga-spellbreaker.png` — and
            // a full-width reverse is what we draw, on the user's ruling. A band
            // broken into pieces reads as damage in a terminal where it read as
            // design on a 1989 monitor. The measurement stands in
            // `StatusBand::PerRun`'s own doc; only the rendering is simplified.
            status: StatusBand::FullReverse,
            cursor_shape: CursorShape::Block,
            cursor_colour: (0xFF, 0x7E, 0x1C),
        })),
    },
    MachineProfile {
        number: ATARI_ST_INTERPRETER_NUMBER,
        name: "Atari ST",
        default_colours: Some((ST_DEFAULT_BACKGROUND, ST_DEFAULT_FOREGROUND)),
        palette: Palette::Standard,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: false,
        v6_cell: V6Cell::DEFAULT,
        v6_release_face_space: V6FaceSpace::Native,
        v6_system_face: None,
        v6_wrap_regime: V6WrapRegime::Attributes,
        v6_emphasis: V6Emphasis::Slope,
        v6_std_window: None,
        // SQ-0933, from `machine-screenshots/st-zork1.png` — Zork I revision 88 /
        // serial 840726, the same release `stories/` carries. A v3 story, so it has
        // no colour concept at all and every pixel is the interpreter's own
        // presentation: this is a period-look capture in the same sense
        // `dos-hitchhiker.png` is, and it is the one that empties
        // `the_machines_with_no_capture_decline`.
        //
        // **READ THE CAPTURE'S NOTES BEFORE TRUSTING A NUMBER OFF IT.** The
        // emulator applied a scanline filter, and it does not merely dim the frame
        // — it INVERTS the answer. Censused whole, st-zork1.png is 53.6% black
        // against 45.9% near-white, which reads as a black page. Split by row
        // parity, the even rows are 99.2% pure black and the odd rows 92.0%
        // #EBEBEB: the black is the gap between scanlines, and the page is white.
        // Only the odd rows are picture.
        //
        // Corroborated from the other direction, which is why this row is stated
        // rather than hedged: the machine's own v5 pair is already here from
        // Infocom's `st/` interpreter — DEF_BACK 9 (white) under DEF_FORE 2 (black)
        // — and the v3 capture shows the same two. What a v5 story is TOLD and what
        // a v3 story is SHOWN agree, exactly as they did for the IBM PC.
        period_look: Some(MachineLook::Measured(PeriodLook {
            // WHITE, not the capture's #EBEBEB, for the same reason the IBM PC row
            // takes EGA's own entries over a screenshot's #0F009E: the dimming belongs
            // to the emulator's scanline filter, and standard colours 9 and 2 are
            // white and black with no shade to be wrong about.
            page: (0xFF, 0xFF, 0xFF),
            ink: (0x00, 0x00, 0x00),
            // Measured edge to edge on all eight rows of the band — x=0 and x=639
            // both inked, the last row solid across — with the labels reversed out
            // of it in the page colour.
            status: StatusBand::FullReverse,
            // A filled 8x8 cell immediately right of the `>` prompt.
            cursor_shape: CursorShape::Block,
            cursor_colour: (0x00, 0x00, 0x00),
        })),
    },
    MachineProfile {
        number: IBM_PC_INTERPRETER_NUMBER,
        name: "IBM PC",
        // SQ-0928: blue under white, observed from DOS captures and corroborated
        // by a trace of which games name colours and which do not. This row used
        // to decline, on the ground that "an IBM PC in a terminal is the player's
        // terminal" — which was right about the LAUNCH and wrong about the
        // MACHINE. The two are separated now: the machine states its pair here,
        // and the app presents it only when a medium named this machine (see
        // `IBM_PC_DEFAULT_BACKGROUND`). A bare story file still gets the player's
        // own terminal, which is what that reasoning was protecting.
        default_colours: Some((IBM_PC_DEFAULT_BACKGROUND, IBM_PC_DEFAULT_FOREGROUND)),
        palette: Palette::IbmXzip,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: false,
        v6_cell: V6Cell::DEFAULT,
        v6_release_face_space: V6FaceSpace::Native,
        v6_system_face: None,
        v6_wrap_regime: V6WrapRegime::Attributes,
        v6_emphasis: V6Emphasis::Slope,
        v6_std_window: None,
        // SQ-0873/SQ-0928, from `machine-screenshots/dos-hitchhiker.png` — the last
        // period-look capture bar the Atari ST's. Hitchhiker's r47/840914 under a
        // CGA colour display, and **Version 3** is what makes it a period look
        // rather than a default pair: colour arrives with v5, so nothing on that
        // screen was asked for by the story.
        //
        // Row- and column-censused: page blue at 92.4% of the frame, light grey
        // ink, and a cursor at x 44..58 by y 717..724 — 15px of a 14.5px cell
        // wide and 8px of a ~29px cell tall, so one cell wide across its bottom
        // quarter, in the ink colour. A DOS underscore.
        //
        // **THE RGB IS THE HARDWARE'S, NOT THE CAPTURE'S**, and this row is the
        // one place in the period-look table where that distinction can be made.
        // The capture measures #0F009E and #A0A0A0; the EGA/CGA palette is DIGITAL
        // and exactly specified, so the honest values are its entries 1 and 7 —
        // `blorb::infocom_pics::EGA_PALETTE`, verified there against four sources
        // that agree entry for entry. The gap is video scaling, and it shows in
        // the corpus: `dos-shogun.png` and `dos-arthur.png` measure #0000A1..A6
        // with no red tint at all, while this capture's #0F009E carries 0x0F of
        // red that no EGA colour has.
        //
        // Every other row here records what an emulator drew, because those
        // machines' palettes are analogue (the VIC-II) or the register mapping is
        // in doubt (the Amiga's #074BA1 against a bit-replicated $05A = #0055AA).
        // This one does not have to guess.
        //
        // A PC's look was its display adapter's answer, which is why this row
        // declined for so long; what is recorded is the CGA colour rendition, the
        // same way the Apple's records the white-monitor one.
        //
        // **THE STATUS LINE IS SIMPLIFIED, DELIBERATELY.** The capture shows
        // `PerRun` structure — 611 contiguous px of page run through the middle of
        // the row — but its runs are #A2000D RED on grey, a colour in neither
        // channel of the body pair and appearing nowhere else in the frame. That is
        // `Own` colours applied per run, which `StatusBand` cannot express: its
        // variants carry a structure or a pair, never both. Recorded as a plain
        // `PerRun` on the user's instruction rather than growing the type for one
        // glyph colour on one row. The red is real and is not modelled; see
        // `machine-screenshots/info.txt`.
        //
        // **AND THE PAIR IS NOT STORED HERE**, which is the one row in this table
        // that states no page and no ink (SQ-0983). The adapter's entries 1 and 7
        // are #0000AA and #AAAAAA, as above — that is the truth about the hardware
        // and it does not change. What a STORY is shown is those numbers through
        // the Z-machine's own 15-bit colour space, where 0xAA truncates to 21/31
        // and comes back bit-replicated as 0xAD: #0000AD under #ADADAD. The row
        // resolves 6 and 9 through its palette rather than restating them, so the
        // default page and a story's own `@set_colour(6)` are one lookup and cannot
        // abut each other three parts in 255 apart. See `MachineLook::Resolved`;
        // the ink is also the one value in this table that depends on the story's
        // Version, so there is no constant to correct.
        period_look: Some(MachineLook::Resolved {
            // Per-run in the capture, full-width here — the same ruling as the
            // Amiga's above, and the same reason.
            status: StatusBand::FullReverse,
            cursor_shape: CursorShape::Underscore,
        }),
    },
    MachineProfile {
        number: COMMODORE_128_INTERPRETER_NUMBER,
        name: "Commodore 128",
        // Declined: no Infocom Commodore interpreter has been read for a v5
        // story's default pair (SQ-0869). The measured look is period_look below.
        default_colours: None,
        palette: Palette::Standard,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: false,
        v6_cell: V6Cell::DEFAULT,
        v6_release_face_space: V6FaceSpace::Native,
        v6_system_face: None,
        v6_wrap_regime: V6WrapRegime::Attributes,
        v6_emphasis: V6Emphasis::Slope,
        v6_std_window: None,
        // c128-trinity.png: Trinity (v4) at the first prompt, two colours exactly.
        // #55FFFF is RGBI light cyan on the nose, so this row is as close to
        // palette-independent as an emulator capture gets.
        period_look: Some(MachineLook::Measured(PeriodLook {
            page: (0x00, 0x00, 0x00),
            ink: (0x55, 0xFF, 0xFF),
            status: StatusBand::FullReverse,
            cursor_shape: CursorShape::Underscore,
            cursor_colour: (0x55, 0xFF, 0xFF),
        })),
    },
    MachineProfile {
        number: COMMODORE_64_INTERPRETER_NUMBER,
        name: "Commodore 64",
        // Declined for the same reason the C128's is: no Infocom Commodore
        // interpreter has been read for a v5 story's default pair (SQ-0869).
        default_colours: None,
        palette: Palette::Standard,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: false,
        v6_cell: V6Cell::DEFAULT,
        v6_release_face_space: V6FaceSpace::Native,
        v6_system_face: None,
        v6_wrap_regime: V6WrapRegime::Attributes,
        v6_emphasis: V6Emphasis::Slope,
        v6_std_window: None,
        // c64-zork1-solidgold.png: Zork I release 52 / serial 871125, whose own
        // banner reads "Interpreter 8 Version J" — the machine naming itself, which
        // is as direct a statement of the number as this table has anywhere.
        //
        // Black page, white ink, and the status line a plain full-width reverse
        // running x 37..522 with no interior gap wider than a glyph.
        period_look: Some(MachineLook::Measured(PeriodLook {
            page: (0x00, 0x00, 0x00),
            ink: (0xFF, 0xFF, 0xFF),
            status: StatusBand::FullReverse,
            cursor_shape: CursorShape::Underscore,
            cursor_colour: (0xFF, 0xFF, 0xFF),
        })),
    },
    MachineProfile {
        number: APPLE_IIC_INTERPRETER_NUMBER,
        name: "Apple IIc",
        default_colours: Some((APPLE_DEFAULT_BACKGROUND, APPLE_DEFAULT_FOREGROUND)),
        palette: Palette::Standard,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: false,
        v6_cell: V6Cell::DEFAULT,
        v6_release_face_space: V6FaceSpace::Native,
        v6_system_face: None,
        v6_wrap_regime: V6WrapRegime::Attributes,
        v6_emphasis: V6Emphasis::Slope,
        v6_std_window: None,
        period_look: Some(MachineLook::Measured(APPLE_PERIOD_LOOK)),
    },
    MachineProfile {
        number: APPLE_IIGS_INTERPRETER_NUMBER,
        name: "Apple IIgs",
        default_colours: Some((APPLE_DEFAULT_BACKGROUND, APPLE_DEFAULT_FOREGROUND)),
        palette: Palette::Standard,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: false,
        v6_cell: V6Cell::DEFAULT,
        v6_release_face_space: V6FaceSpace::Native,
        v6_system_face: None,
        v6_wrap_regime: V6WrapRegime::Attributes,
        v6_emphasis: V6Emphasis::Slope,
        v6_std_window: None,
        period_look: Some(MachineLook::Measured(APPLE_PERIOD_LOOK)),
    },
];

/// The machine header `$1E` value `number` names, or `None` for a §11.1.3 number
/// this table does not model. See [`MACHINES`] for which are absent and why.
pub fn machine(number: u8) -> Option<&'static MachineProfile> {
    MACHINES.iter().find(|m| m.number == number)
}

/// This machine's period look for a story of Version `zversion` (SQ-0939).
///
/// **Almost every row answers its stored measurement unchanged**, and this exists
/// for the one that cannot. A [`PeriodLook`]'s page and ink are RGB read off a
/// capture, which is right for a machine whose screen is simply a fact — but the
/// IBM PC's white MOVED between Infocom's two interpreters ([`Palette::IbmXzip`]
/// versus [`Palette::IbmYzip`]), so one stored pair cannot be true for both.
///
/// For that machine the body pair is not an independent measurement at all: it is
/// its own palette's resolution of the pair the row already states in
/// [`MachineProfile::default_colours`]. Deriving it means the screen a v1–v5 story
/// is painted on and the colour a v6 story gets from `@set_colour(9)` cannot
/// disagree — they are the same table lookup.
///
/// **The row says which it is, and this function no longer guesses** (SQ-0983). It
/// used to derive for any machine whose palette was one of the IBM pair and quietly
/// overwrite whatever that row had stored, which left the IBM PC carrying a page and
/// ink nothing ever read. A [`MachineLook::Resolved`] row now stores no pair to be
/// overwritten, and a [`MachineLook::Measured`] one is answered untouched — the
/// Amiga's blue included, which is the case that argued the gate into existence and
/// is now enforced by the row's own shape rather than by a palette match here.
///
/// # The caret moved with the interpreter generation too (SQ-0947)
///
/// The same argument the palette makes, made by the cursor. Every stored
/// `cursor_shape` here was measured on a v1–v5 capture, and on two machines the v6
/// interpreter draws a different caret: the Amiga swaps its fixed `#FF7E1C` orange
/// block for the pair's own reverse, and the IBM PC swaps its underscore for a full
/// reversed cell. Both become [`CursorShape::ReverseSpace`], whose docs carry the
/// captures. The Macintosh is the control — its v6 frames draw the same bar its v3
/// one does — and every machine with no v6 capture declines rather than guessing,
/// which is this table's rule everywhere else.
pub fn period_look_for(number: u8, zversion: Option<u8>) -> Option<PeriodLook> {
    let m = machine(number)?;
    let mut look = match m.period_look? {
        MachineLook::Measured(l) => l,
        MachineLook::Resolved { status, cursor_shape } => {
            // Declining here is unreachable and stays a decline rather than a
            // panic: a row that states `Resolved` must state the pair it resolves,
            // which `a_resolved_row_states_the_pair_it_resolves` holds.
            let (bg, fg) = m.default_colours?;
            let p = palette_for(number, zversion);
            let rgb = |n: u8| crate::screen::true_colour_in(p, n).map(crate::screen::rgb15_to_888);
            let ink = rgb(fg)?;
            PeriodLook {
                page: rgb(bg)?,
                ink,
                status,
                cursor_shape,
                // The capture measures the caret in the ink (`dos-hitchhiker.png`,
                // one cell wide across its bottom quarter), and a resolved row has
                // no way to say anything else — which is the point: the stored
                // `#AAAAAA` used to sit three parts in 255 from the ink beside it.
                cursor_colour: ink,
            }
        }
    };
    // After the pair, so the ink parked in `cursor_colour` is the one this VERSION
    // resolves — the IBM PC's v6 white is `#FFFFFF` through YZIP, not XZIP's
    // `#AAAAAA`.
    if zversion == Some(6) && matches!(number, AMIGA_INTERPRETER_NUMBER | IBM_PC_INTERPRETER_NUMBER) {
        look.cursor_shape = CursorShape::ReverseSpace;
        look.cursor_colour = look.ink;
    }
    Some(look)
}

/// The palette this machine resolves §8.3.1 colour numbers through, for a story
/// of Version `zversion` (SQ-0939).
///
/// **Almost every machine answers the same for every version**, and this exists for
/// the one that does not. Infocom shipped two IBM interpreters with two different
/// mappings of colour numbers onto EGA attributes, and they differ in exactly one
/// entry: XZIP (v1–v5) sends WHITE to attribute 7, `#AAAAAA`, and YZIP (v6) sends it
/// to 15, `#FFFFFF`. Both are corroborated by a capture — see
/// [`crate::screen::ega_true_colour`], which carries the tables and the evidence.
///
/// Asked at boot, before the story runs and before the host resolves a single
/// colour, and then carried on the machine ([`crate::cpu::exec::Machine::palette`],
/// SQ-1393). A version-dependent palette that were asked LATER, or asked twice on
/// two paths, would mean one colour number looking like two colours on one screen
/// — which is why the machine holds ONE answer that every consumer reads.
pub fn palette_for(number: u8, zversion: Option<u8>) -> Palette {
    match machine(number).map(|m| m.palette) {
        Some(Palette::IbmXzip) if zversion == Some(6) => Palette::IbmYzip,
        Some(p) => p,
        None => Palette::Standard,
    }
}

/// The machine a story's header currently claims to be running on, or `None`
/// when `$1E` names one this table does not model.
///
/// Read back out of the HEADER rather than held as a field, which is what makes
/// every rule built on it survive a `@restart`, a Quetzal `@restore` and a host
/// Save State without anybody carrying it.
pub fn machine_of(mem: &Memory) -> Option<&'static MachineProfile> {
    machine(mem.read_byte(0x1E))
}

/// The `(foreground, background)` pair the header currently publishes, as
/// §8.3.1 standard colour numbers — `$2D` over `$2C` (ZMSD §8.3.3).
///
/// Shared by [`crate::screen::amiga_screen_pair`] and
/// [`crate::screen::machine_screen_pair`], which differ only in which machines
/// they ask it of.
pub(crate) fn header_pair(mem: &Memory) -> (ZColour, ZColour) {
    (ZColour::Standard(mem.read_byte(0x2D)), ZColour::Standard(mem.read_byte(0x2C)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_reachable_by_its_own_number_and_the_table_is_sorted() {
        let mut last = 0u8;
        for m in MACHINES {
            assert!(m.number > last, "{} out of order at {}", m.name, m.number);
            last = m.number;
            assert_eq!(machine(m.number).map(|r| r.name), Some(m.name));
        }
    }

    /// The gaps, asserted as gaps. A future row is welcome; a future row arriving
    /// *by accident* — say by widening a match — is what this catches.
    #[test]
    fn the_unmodelled_numbers_answer_none_rather_than_a_substitute() {
        for n in [0u8, 1, 11, 12, 255] {
            assert!(machine(n).is_none(), "interpreter {n} is not modelled and must say so");
        }
    }

    #[test]
    fn the_amiga_is_the_only_machine_with_global_colour_pens() {
        let pens: Vec<_> =
            MACHINES.iter().filter(|m| m.global_colour_pens).map(|m| m.number).collect();
        assert_eq!(pens, vec![AMIGA_INTERPRETER_NUMBER], "ZMSD §8.3 names the Amiga and no other");
        // …and a machine with global pens necessarily states the page they paint.
        for m in MACHINES.iter().filter(|m| m.global_colour_pens) {
            assert!(m.v6_screen_page, "{} has pens but states no page", m.name);
            assert!(m.default_colours.is_some(), "{} has pens but no pair", m.name);
        }
    }

    /// SQ-0846's two machines, and only those two. The Apple rows state a page
    /// as real as the Mac's and deliberately do not claim it — see
    /// [`MachineProfile::v6_screen_page`].
    #[test]
    fn two_machines_state_the_version_6_page() {
        let pages: Vec<_> = MACHINES.iter().filter(|m| m.v6_screen_page).map(|m| m.number).collect();
        assert_eq!(pages, vec![MACINTOSH_INTERPRETER_NUMBER, AMIGA_INTERPRETER_NUMBER]);
    }

    /// The Apple family is one bundle with three numbers — `bsubs.asm`'s
    /// `MACHINE:` picks between `IIeID 2`, `IIcID 9` and `IIgsID 10` at boot, and
    /// `zboot.asm` seeds the same `$2C`/`$2D` for all three before it runs.
    #[test]
    fn the_three_apples_differ_only_in_their_number() {
        let apples: Vec<_> = [
            APPLE_IIE_INTERPRETER_NUMBER,
            APPLE_IIC_INTERPRETER_NUMBER,
            APPLE_IIGS_INTERPRETER_NUMBER,
        ]
        .into_iter()
        .map(|n| *machine(n).expect("modelled"))
        .collect();
        for a in &apples {
            assert_eq!(a.default_colours, Some((2, 9)), "zboot.asm: black page, white ink");
            assert_eq!(a.palette, Palette::Standard, "ZIPCOLOR IS §8.3.1's eight");
            assert_eq!(a.global_colour_pens, apples[0].global_colour_pens);
            assert_eq!(a.v6_screen_page, apples[0].v6_screen_page);
        }
        // …and the numbers really are three, so this is a family and not a copy.
        assert_eq!(apples.iter().filter(|a| a.number == apples[0].number).count(), 1);
    }

    /// Sourced values, one assertion per machine, quoted where the constant is.
    #[test]
    fn the_pairs_are_the_sourced_ones() {
        let pair = |n| machine(n).expect("modelled").default_colours;
        assert_eq!(pair(AMIGA_INTERPRETER_NUMBER), Some((12, 9)), "the floppies' DEF_BACK/DEF_FORE");
        assert_eq!(pair(MACINTOSH_INTERPRETER_NUMBER), Some((9, 2)), "SetColor := zWHITE*256 + zBLACK");
        assert_eq!(pair(ATARI_ST_INTERPRETER_NUMBER), Some((9, 2)), "DEF_BACK 9 / DEF_FORE 2");
        assert_eq!(pair(APPLE_IIGS_INTERPRETER_NUMBER), Some((2, 9)), "zboot.asm lda #2 / lda #9");
        // SQ-0928: the one pair here that is OBSERVED rather than read out of
        // Infocom's source — but observed twice over, from DOS captures and from a
        // trace of which games name colours and which do not. And what is observed
        // is a §8.3.1 NUMBER, not a shade, so unlike `PeriodLook` there is no
        // emulator-dependent RGB to be wrong about.
        //
        // It used to decline, "the player's terminal" — which was right about the
        // LAUNCH and wrong about the MACHINE. The two are separated now: the row
        // states the machine, and a front-end presents it only when a medium named
        // this machine. A bare story file still gets the player's own terminal.
        assert_eq!(pair(IBM_PC_INTERPRETER_NUMBER), Some((6, 9)), "blue under white");
        assert_eq!(pair(COMMODORE_128_INTERPRETER_NUMBER), None, "no source read");
        // Two machines resolve colour NUMBERS through something other than
        // §8.3.1's recommended table, and each is read out of the program that
        // painted them: the Amiga's `colortable[]` and the IBM's
        // `Zip_to_ega`/`zip_to_ibm_color` (SQ-0939). Every other row keeps the
        // standard's own values, because reputation is not evidence — the Atari
        // ST's famous 512 and the Commodore's sixteen are not in here for exactly
        // that reason.
        let own: Vec<_> =
            MACHINES.iter().filter(|m| m.palette != Palette::Standard).map(|m| m.number).collect();
        assert_eq!(own, vec![AMIGA_INTERPRETER_NUMBER, IBM_PC_INTERPRETER_NUMBER]);
    }

    /// **A period look is not a `default_colours` pair**, and the two rows where
    /// they diverge are the reason the field exists (SQ-0873).
    ///
    /// The Amiga reports 12/9 — a GREY page — from its v5-era interpreter, and its
    /// v3 interpreter draws a blue one. The Commodore 128 reports nothing at all and
    /// yet has a measured look. If a later change ever makes one derivable from the
    /// other, this case is what should stop it.
    #[test]
    fn the_period_look_is_not_the_default_pair() {
        let amiga = machine(AMIGA_INTERPRETER_NUMBER).expect("modelled");
        assert_eq!(amiga.default_colours, Some((12, 9)), "grey page, white ink, from v5");
        // SQ-0983: the Amiga is `Measured` and must stay so — a `Resolved` row would
        // paint its v3 screen with its v5 pair's grey, which is this case's whole
        // point, now stated in the row's own shape.
        let Some(MachineLook::Measured(look)) = amiga.period_look else {
            panic!("the Amiga's look is measured on two v3 floppies, never resolved");
        };
        assert_eq!(look.page, (0x07, 0x4B, 0xA1), "and the v3 interpreter's page is BLUE");
        // Grey is 10..12 in §8.3.1; whatever blue resolves to, it is not that.
        assert_ne!(look.page.0, look.page.1, "a grey has equal channels; this does not");

        let c128 = machine(COMMODORE_128_INTERPRETER_NUMBER).expect("modelled");
        assert_eq!(c128.default_colours, None, "no v5 source was ever read");
        assert!(c128.period_look.is_some(), "which did not stop the machine being measured");
    }

    /// The three sub-decisions are independent — the finding that fixed this
    /// field's shape. Nine machines produced four measured status behaviours and
    /// three stored cursor shapes, and neither follows from the body pair.
    #[test]
    fn the_status_band_and_cursor_do_not_follow_from_the_body_pair() {
        let look = |n| period_look_for(n, None).expect("measured");
        let (apple, mac, amiga, c128) = (
            look(APPLE_IIE_INTERPRETER_NUMBER),
            look(MACINTOSH_INTERPRETER_NUMBER),
            look(AMIGA_INTERPRETER_NUMBER),
            look(COMMODORE_128_INTERPRETER_NUMBER),
        );
        // Four measured machines, three distinct status behaviours between them.
        assert_eq!(apple.status, StatusBand::FullReverse);
        assert_eq!(c128.status, StatusBand::FullReverse);
        assert_eq!(mac.status, StatusBand::Ruled, "rules, not a ground");
        // SQ-0873: the Amiga's capture reverses PER RUN and it draws a full-width
        // reverse, on the user's ruling — see `StatusBand::PerRun`. What this case
        // is really about survives the change: the four measured behaviours are
        // still not derivable from the body pair, and the Macintosh's is the proof.
        assert_eq!(amiga.status, StatusBand::FullReverse, "drawn whole, measured per run");

        // Three cursor shapes, and the Macintosh's bar is the one no cell-inverting
        // front-end would ever draw.
        assert_eq!(mac.cursor_shape, CursorShape::Bar);
        assert_eq!(apple.cursor_shape, CursorShape::Block);
        assert_eq!(amiga.cursor_shape, CursorShape::Block);
        assert_eq!(c128.cursor_shape, CursorShape::Underscore);

        // And on one machine the cursor's colour is neither the page nor the ink,
        // so it cannot be dropped and recomputed.
        assert!(
            amiga.cursor_colour != amiga.page && amiga.cursor_colour != amiga.ink,
            "the Amiga's cursor is orange on a blue page under white ink",
        );
        assert_eq!(c128.cursor_colour, c128.ink, "…but the C128's is simply its ink");

        // **The COUNT is the part that went stale** (SQ-0970): this field's rustdoc
        // said "two of five machines", naming the Commodore 64's black as the
        // second — which IS its ink, on a black page under white — while the table
        // had grown from five machines to nine. Sampling two rows is what let that
        // happen, so census every row instead: the claim the doc makes is a
        // property of the whole table and only the whole table can falsify it.
        let odd: Vec<&str> = MACHINES
            .iter()
            .filter_map(|m| period_look_for(m.number, None).map(|l| (m.name, l)))
            .filter(|(_, l)| l.cursor_colour != l.page && l.cursor_colour != l.ink)
            .map(|(name, _)| name)
            .collect();
        assert_eq!(odd, ["Amiga"], "exactly one caret is neither its page nor its ink");
        assert_eq!(MACHINES.len(), 9, "…out of nine machines");
        assert_eq!(
            MACHINES.iter().filter(|m| m.period_look.is_some()).count(),
            9,
            "every one of which is measured, so none of them is excused from the census",
        );
    }

    /// The IBM PC resolves colour numbers through EGA, and its WHITE depends on
    /// which of Infocom's two interpreters the story would have run under.
    ///
    /// `Zip_to_ega` (yzip, v6) sends white to attribute 15 and `zip_to_ibm_color`
    /// (xzip, v1–v5) sends it to 7. Every other colour is identical between them,
    /// which is what makes this worth a whole second palette rather than a fudge:
    /// one entry, sourced from the two programs that painted it, and corroborated
    /// by one capture each.
    #[test]
    fn the_ibm_pcs_white_is_the_one_colour_its_two_interpreters_disagree_on() {
        use crate::screen::ega_true_colour;
        for n in [2u8, 3, 4, 5, 6, 7, 8] {
            assert_eq!(
                ega_true_colour(n, true),
                ega_true_colour(n, false),
                "colour {n} is the same attribute in both interpreters",
            );
        }
        assert_ne!(ega_true_colour(9, true), ega_true_colour(9, false), "…and white is not");
        assert_eq!(ega_true_colour(9, true), Some(0x7FFF), "yzip: EGA 15, #FFFFFF");
        assert_eq!(ega_true_colour(9, false), Some(0x56B5), "xzip: EGA 7, #AAAAAA");
        // The page both agree on, and the one every DOS capture measures.
        assert_eq!(ega_true_colour(6, true), Some(0x5400), "blue is EGA 1, #0000AA");
        // 10..=12 are deliberately unmapped; see `ega_true_colour`'s docs.
        for n in [10u8, 11, 12] {
            assert_eq!(ega_true_colour(n, true), None, "the v6 greys are not guessed at");
        }
    }

    /// The palette is chosen from the machine AND the version, once, before the
    /// story runs — `palette_for` is that choice, and it is the only place the
    /// version enters.
    #[test]
    fn the_palette_is_the_machines_except_where_the_version_moves_it() {
        let ibm = IBM_PC_INTERPRETER_NUMBER;
        assert_eq!(palette_for(ibm, Some(6)), Palette::IbmYzip, "v6 is YZIP's machine");
        for v in [1u8, 2, 3, 4, 5, 7, 8] {
            assert_eq!(palette_for(ibm, Some(v)), Palette::IbmXzip, "v{v} is not");
        }
        assert_eq!(palette_for(ibm, None), Palette::IbmXzip, "and no version byte is not either");
        // No other machine moves with the version.
        for n in [AMIGA_INTERPRETER_NUMBER, MACINTOSH_INTERPRETER_NUMBER, ATARI_ST_INTERPRETER_NUMBER] {
            let base = machine(n).expect("modelled").palette;
            for v in 1..=8 {
                assert_eq!(palette_for(n, Some(v)), base, "machine {n} has one palette");
            }
        }
        // An unmodelled number is not dressed as anything.
        assert_eq!(palette_for(200, Some(6)), Palette::Standard);
    }

    /// The caret moved with the interpreter generation on exactly two machines
    /// (SQ-0947), and this is the case that falsifies the reported symptom.
    ///
    /// Reported by eye: "amiga zork-zero is wrong color -- orange, dos v6 games
    /// should be reversed space". Both halves are stored measurements applied a
    /// version too far — the Amiga's `#FF7E1C` block and the IBM PC's underscore
    /// are v1–v5 captures, and `machine-screenshots/` has three v6 frames that
    /// disagree with them (`amiga-zorkzero.png`, `amiga-shogun.png`,
    /// `dos-arthur.png`; see [`CursorShape::ReverseSpace`]).
    ///
    /// The Macintosh is asserted alongside because it is the CONTROL: its own v6
    /// frames draw the same bar its v3 one does, so a rule that changed every
    /// machine at Version 6 would be wrong about it, and a rule that changed none
    /// would be wrong about the other two.
    #[test]
    fn the_caret_the_version_six_interpreters_draw_is_the_pair_reversed() {
        let caret = |n, v| period_look_for(n, Some(v)).expect("measured").cursor_shape;
        let orange = period_look_for(AMIGA_INTERPRETER_NUMBER, Some(3)).expect("measured");
        assert_eq!(orange.cursor_colour, (0xFF, 0x7E, 0x1C), "the stored v3 measurement");

        for (n, stored) in [
            (AMIGA_INTERPRETER_NUMBER, CursorShape::Block),
            (IBM_PC_INTERPRETER_NUMBER, CursorShape::Underscore),
        ] {
            assert_eq!(caret(n, 6), CursorShape::ReverseSpace, "machine {n} at v6");
            for v in 1..=5 {
                assert_eq!(caret(n, v), stored, "machine {n} at v{v} keeps its capture");
            }
            // …and the colour goes with it, so a reader that takes the field
            // without the shape gets the pair's reverse, never the old orange.
            let v6 = period_look_for(n, Some(6)).expect("measured");
            assert_eq!(v6.cursor_colour, v6.ink, "machine {n}: the ink, this version's");
        }

        // The IBM PC's v6 ink is YZIP's white, which is what makes the ordering
        // inside `period_look_for` load-bearing: the caret takes the pair AFTER the
        // palette has resolved it, so this is #FFFFFF and not XZIP's #AAAAAA.
        let dos6 = period_look_for(IBM_PC_INTERPRETER_NUMBER, Some(6)).expect("measured");
        assert_eq!(dos6.cursor_colour, (0xFF, 0xFF, 0xFF), "EGA 15, not EGA 7");
        // …where a v3 story gets XZIP's instead, so the underscore is the grey
        // `dos-hitchhiker.png` was censused for. Asserted against that version's own
        // INK rather than against a stored constant, which is the only thing a
        // resolved row can be checked against — and which the row used to contradict
        // by three parts in 255 (SQ-0983).
        let dos3 = period_look_for(IBM_PC_INTERPRETER_NUMBER, Some(3)).expect("measured");
        assert_eq!(dos3.cursor_colour, dos3.ink, "the underscore is drawn in the ink");
        assert_ne!(dos3.cursor_colour, dos6.cursor_colour, "…and the two whites differ");

        // The control: mac-zorkzero.png and mac-shogun.jpg draw a bar, so the
        // Macintosh does not move — and neither does a machine with no v6 capture.
        for n in [
            MACINTOSH_INTERPRETER_NUMBER,
            APPLE_IIGS_INTERPRETER_NUMBER,
            ATARI_ST_INTERPRETER_NUMBER,
            COMMODORE_64_INTERPRETER_NUMBER,
            COMMODORE_128_INTERPRETER_NUMBER,
        ] {
            let row = machine(n).expect("modelled").period_look.expect("measured");
            let MachineLook::Measured(stored) = row else {
                panic!("machine {n} states a measured look")
            };
            for v in 1..=6 {
                assert_eq!(
                    period_look_for(n, Some(v)),
                    Some(stored),
                    "machine {n} has no v6 capture to move it",
                );
            }
        }
    }

    /// **The IBM PC's body pair is a lookup, and the row stores none** (SQ-0983).
    ///
    /// The row used to carry `#0000AA` under `#AAAAAA` — EGA attributes 1 and 7,
    /// which is the truth about the ADAPTER — while `period_look_for` resolved the
    /// pair afresh and overwrote both. What a story is shown is those same entries
    /// through the Z-machine's 15-bit colour space, where `0xAA` truncates to 21/31
    /// and comes back bit-replicated as `0xAD`, so the stored constant was three
    /// parts in 255 from anything on screen and nothing read it.
    ///
    /// **And it could not have been corrected, only removed.** The ink is
    /// version-dependent: XZIP resolves colour 9 to `#ADADAD` and YZIP to
    /// `#FFFFFF`, so no single constant is true for both, which is the half of the
    /// argument that a corrected number would not have answered. The pair a v1–v5
    /// story is painted on must also be what its own `@set_colour(6)` resolves to,
    /// or a default page and a story-set one would abut at a window edge three
    /// parts apart.
    #[test]
    fn the_ibm_pcs_period_look_is_its_palette_and_never_a_stored_pair() {
        let ibm = machine(IBM_PC_INTERPRETER_NUMBER).expect("modelled");
        assert!(
            matches!(ibm.period_look, Some(MachineLook::Resolved { .. })),
            "the row states the two decisions a palette cannot make, and no pair",
        );

        // v1–v5: XZIP, and the pair is EGA 1 and EGA 7 through the 15-bit space.
        for v in 1..=5 {
            let look = period_look_for(IBM_PC_INTERPRETER_NUMBER, Some(v)).expect("resolved");
            assert_eq!(look.page, (0x00, 0x00, 0xAD), "v{v}: EGA 1 through 15-bit");
            assert_eq!(look.ink, (0xAD, 0xAD, 0xAD), "v{v}: EGA 7 through 15-bit");
            assert_eq!(look.cursor_colour, look.ink, "v{v}: the underscore is drawn in the ink");
            assert_eq!(look.cursor_shape, CursorShape::Underscore, "v{v}: dos-hitchhiker.png");
            assert_eq!(look.status, StatusBand::FullReverse, "v{v}: the row's own ruling");
            // The adapter's own bytes are never what a story is shown.
            assert_ne!(look.page, (0x00, 0x00, 0xAA), "v{v}: not the adapter's byte");
            assert_ne!(look.ink, (0xAA, 0xAA, 0xAA), "v{v}: nor its grey");
        }

        // v6: YZIP, and white moves — which is why no constant could hold it.
        let v6 = period_look_for(IBM_PC_INTERPRETER_NUMBER, Some(6)).expect("resolved");
        assert_eq!(v6.ink, (0xFF, 0xFF, 0xFF), "YZIP sends white to EGA 15");
        assert_eq!(v6.page, (0x00, 0x00, 0xAD), "…and blue is the one both agree on");
        assert_ne!(
            v6.ink,
            period_look_for(IBM_PC_INTERPRETER_NUMBER, Some(5)).expect("resolved").ink,
            "one machine, two inks: a stored pair cannot be true for both",
        );

        // The pair is the SAME lookup a story's own colour numbers take, which is
        // the consistency the derivation exists for: a default page and a page the
        // story asks for by number cannot come out different blues.
        for (v, p) in [(3u8, Palette::IbmXzip), (6, Palette::IbmYzip)] {
            let (bg, fg) = ibm.default_colours.expect("the row reports a pair");
            let look = period_look_for(IBM_PC_INTERPRETER_NUMBER, Some(v)).expect("resolved");
            let rgb = |n| crate::screen::true_colour_in(p, n).map(crate::screen::rgb15_to_888);
            assert_eq!(Some(look.page), rgb(bg), "v{v}: the page IS colour {bg}");
            assert_eq!(Some(look.ink), rgb(fg), "v{v}: the ink IS colour {fg}");
        }
    }

    /// **Every other machine answers its stored measurement, unchanged** — asked of
    /// the table rather than transcribed from it, so a row that starts deriving its
    /// look has to say so here (SQ-0983).
    ///
    /// The restructuring that emptied the IBM PC's pair could have moved any row's,
    /// and a hand-copied table of expected colours would drift with the rows it was
    /// copied from. This censuses `MACHINES` instead: a `Measured` row is what
    /// `period_look_for` hands back for every version below 6, byte for byte, and
    /// exactly one row is not `Measured`.
    #[test]
    fn every_other_machines_period_look_is_the_one_its_row_stores() {
        let mut measured = 0;
        let mut resolved: Vec<&str> = Vec::new();
        for m in MACHINES {
            match m.period_look.expect("every modelled machine has been measured") {
                MachineLook::Measured(stored) => {
                    measured += 1;
                    for v in 1..=5 {
                        assert_eq!(
                            period_look_for(m.number, Some(v)),
                            Some(stored),
                            "{} at v{v} answers its own capture and nothing else",
                            m.name,
                        );
                    }
                    assert_eq!(
                        period_look_for(m.number, None),
                        Some(stored),
                        "{}: and a machine asked with no story is the same machine",
                        m.name,
                    );
                }
                MachineLook::Resolved { .. } => resolved.push(m.name),
            }
        }
        assert_eq!(resolved, ["IBM PC"], "one row derives its pair; the rest are measured");
        assert_eq!(measured, 8, "…and the other eight are not excused from the census");
    }

    /// A [`MachineLook::Resolved`] row must state the pair it resolves, and its
    /// palette must have both numbers — the invariant that keeps the decline inside
    /// [`period_look_for`] unreachable rather than merely unlikely.
    #[test]
    fn a_resolved_row_states_the_pair_it_resolves() {
        let mut seen = 0;
        for m in MACHINES {
            if !matches!(m.period_look, Some(MachineLook::Resolved { .. })) {
                continue;
            }
            seen += 1;
            let (bg, fg) = m.default_colours.unwrap_or_else(|| {
                panic!("{}: a resolved look has nothing to resolve without a pair", m.name)
            });
            for v in [None, Some(1), Some(5), Some(6)] {
                let p = palette_for(m.number, v);
                for n in [bg, fg] {
                    assert!(
                        crate::screen::true_colour_in(p, n).is_some(),
                        "{}: colour {n} has no true colour in {p:?}",
                        m.name,
                    );
                }
                assert!(period_look_for(m.number, v).is_some(), "{}: so it answers", m.name);
            }
        }
        assert_eq!(seen, 1, "the IBM PC is the row this invariant is about");
    }

    /// Declines are declines, and the ones here are for want of a capture rather
    /// than for want of a decision. Fails when someone fills a row by inference.
    #[test]
    fn the_machines_with_no_capture_decline() {
        let missing: Vec<_> =
            MACHINES.iter().filter(|m| m.period_look.is_none()).map(|m| m.number).collect();
        // The list is EMPTY now, and that is the point of keeping the case rather
        // than deleting it: the IBM PC left when `dos-hitchhiker.png` arrived
        // (SQ-0873), the Atari ST when `st-zork1.png` did (SQ-0933), and the next
        // machine anyone adds starts out unmeasured. This fails the moment a row
        // states a look with nothing in `machine-screenshots/` behind it.
        assert_eq!(
            missing,
            Vec::<u8>::new(),
            "every modelled machine has been measured; a new row must be too",
        );
        // The Atari ST's capture is the one that emptied the list, and its two
        // channels agree — which is the whole reason it could be stated. A v3
        // frame shows white under black; the machine's own v5 pair, sourced from
        // Infocom's `st/` interpreter long before any capture existed, is
        // DEF_BACK 9 / DEF_FORE 2 — white and black in §8.3.1.
        let st = machine(ATARI_ST_INTERPRETER_NUMBER).expect("modelled");
        let Some(MachineLook::Measured(look)) = st.period_look else {
            panic!("st-zork1.png, Zork I r88/840726 — measured, not resolved")
        };
        assert_eq!(st.default_colours, Some((9, 2)), "told white under black");
        assert_eq!(look.page, (0xFF, 0xFF, 0xFF), "…and shown the same");
        assert_eq!(look.ink, (0x00, 0x00, 0x00));
        // Not the capture's #EBEBEB: that dimming is the emulator's scanline
        // filter, and a §8.3.1 white has no shade to be wrong about. The same
        // judgement the IBM PC row makes about #0F009E.
        assert_ne!(look.page, (0xEB, 0xEB, 0xEB), "the emulator's dimming is not the machine's white");
        assert_eq!(look.status, StatusBand::FullReverse, "measured edge to edge on all eight rows");
        assert_eq!(look.cursor_shape, CursorShape::Block);
        assert_eq!(look.cursor_colour, look.ink, "a filled cell in the ink, like the C128's");

        // The Apple family shares one measurement, exactly as it shares one pair.
        let apples: Vec<_> = [
            APPLE_IIE_INTERPRETER_NUMBER,
            APPLE_IIC_INTERPRETER_NUMBER,
            APPLE_IIGS_INTERPRETER_NUMBER,
        ]
        .into_iter()
        .map(|n| machine(n).expect("modelled").period_look)
        .collect();
        assert!(apples.windows(2).all(|w| w[0] == w[1]), "one interpreter, one text screen");
    }

    /// Two machines name a system body face, and they name the right one
    /// (SQ-1037).
    ///
    /// Both had a Version 6 interpreter and both drew prose with a face that
    /// lives on the operating system: `mac/xzip.lst` says `stdFont := geneva`,
    /// and the Amiga's topaz is in ROM and in a Workbench `FONTS:` drawer. Every
    /// other row names nothing, which is a shortage of Version 6 interpreters
    /// rather than a shortage of measurement.
    ///
    /// The Amiga row is the one worth pinning by NAME: a Workbench floppy carries
    /// seven other faces, all of them proportional, and `ruby 8` at that machine's
    /// text scale would pass every test but this one.
    #[test]
    fn only_the_two_version_six_machines_name_a_system_face() {
        assert_eq!(
            machine(MACINTOSH_INTERPRETER_NUMBER).expect("modelled").v6_system_face,
            Some(V6SystemFace::MacFamily(MAC_GENEVA_FONT_FAMILY)),
            "the Macintosh paints Geneva — family 3, so FONT 396 at 12pt",
        );
        assert_eq!(
            MAC_GENEVA_FONT_FAMILY * MAC_FONT_FAMILY_STRIDE + 12,
            396,
            "…which is the id arithmetic, stated once here and read in `blorb::mac_font`",
        );
        assert_eq!(
            machine(AMIGA_INTERPRETER_NUMBER).expect("modelled").v6_system_face,
            Some(V6SystemFace::AmigaDrawer("topaz")),
            "the Amiga takes topaz, and not the seven display faces beside it",
        );
        for n in 1u8..=11 {
            let Some(m) = machine(n) else { continue };
            if n == MACINTOSH_INTERPRETER_NUMBER || n == AMIGA_INTERPRETER_NUMBER {
                continue;
            }
            assert_eq!(
                m.v6_system_face, None,
                "{} names no system face — nothing to name, not something unmeasured",
                m.name,
            );
        }
    }

    /// ZMSD §8.8.3.1.2.2's table, read back a row at a time (SQ-1071).
    ///
    /// The `---` cells are the interesting half: they say the interpreter IGNORES
    /// the state, so the Macintosh and Amiga rows must answer the SAME flow for
    /// every combination of attributes 0 and 3 and change only with `buffer_mode`.
    #[test]
    fn the_wrap_regime_reads_back_the_standards_own_table() {
        // A0 off / A3 off, A0 off / A3 on, A0 on / A3 off, A0 on / A3 on.
        const A: [u16; 4] = [0b0000, 0b1000, 0b0001, 0b1001];

        let std = V6WrapRegime::Attributes;
        assert_eq!(
            A.map(|a| std.flow(a, true)),
            [
                V6TextFlow::CharClip,
                V6TextFlow::CharClip,
                V6TextFlow::CharWrap,
                V6TextFlow::WordWrap
            ],
            "the Standard column",
        );
        assert_eq!(
            A.map(|a| std.flow(a, false)),
            A.map(|a| std.flow(a, true)),
            "…and it ignores buffer_mode, which is the `---` in that column",
        );

        for (n, unbuffered) in [
            (MACINTOSH_INTERPRETER_NUMBER, V6TextFlow::CharWrap),
            (AMIGA_INTERPRETER_NUMBER, V6TextFlow::CharClip),
        ] {
            let m = machine(n).expect("modelled");
            let r = m.v6_wrap_regime;
            assert_eq!(
                A.map(|a| r.flow(a, true)),
                [V6TextFlow::WordWrap; 4],
                "{}: buffer_mode on is word wrap whatever the attributes say",
                m.name,
            );
            assert_eq!(
                A.map(|a| r.flow(a, false)),
                [unbuffered; 4],
                "{}: buffer_mode off is one answer whatever the attributes say",
                m.name,
            );
        }

        // Every other row keeps the standard's own rule — including the IBM PC,
        // whose MSDOS column the standard names two BUGS in.
        for n in 1u8..=11 {
            let Some(m) = machine(n) else { continue };
            if n == MACINTOSH_INTERPRETER_NUMBER || n == AMIGA_INTERPRETER_NUMBER {
                continue;
            }
            assert_eq!(
                m.v6_wrap_regime,
                V6WrapRegime::Attributes,
                "{} reads the window attributes, per §8.8.3.1.1",
                m.name,
            );
        }
    }
}
