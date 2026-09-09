//! The one mount path for a release disk image: what it is, what it holds, and
//! the machine it implies (SQ-0839, SQ-0840).
//!
//! The user's rule, twice over: *"in general when reading a specific disk format
//! we should default the interpreter number to match (even on the cli), but
//! still allow an override"*, and then *"please keep the functionality
//! consistent for all disk image formats — we should have a standard api in our
//! blorb crate for disk images."*
//!
//! So this module is the standard API, and it is the **only** place in the
//! workspace that names a disk format. Everything a mounted disk is asked for
//! — is it an image ([`DiskImage::detect`]), open it ([`MountedDisk::mount`]),
//! what stories are on it ([`MountedDisk::stories`]), which one to play
//! ([`MountedDisk::story`]), what artwork ([`MountedDisk::pictures`]), what it
//! calls itself ([`MountedDisk::label`]), what machine it implies
//! ([`MountedDisk::interpreter_number`]) — is answered here, in one vocabulary,
//! for every format alike. No front-end matches on a format, and none can.
//!
//! ## Why that matters, in this codebase's own history
//!
//! Before SQ-0840 the policy was an `if looks_like_adf … else if looks_like_hfs`
//! chain, written out three times: once in the TUI's picture resolution, once in
//! its story loading, and once — with the HFS arm simply missing — in `zvm-cli`.
//! The CLI therefore mounted an Amiga floppy and refused a Macintosh one, months
//! after `blorb` had learned to read that disk. Neither lane was wrong inside its
//! own scope; the chain guaranteed that nobody owned the join. Adding a format is
//! now a row in [`FORMATS`] and an `impl Volume` beside it, both in this file,
//! and every front-end gains the format in the same commit.
//!
//! ## Two invariants worth stating outright
//!
//! **Detect and mount cannot disagree.** Both walk [`FORMATS`], so a format this
//! crate can recognise is a format it can open — the property `zvm-cli`'s old
//! `looks_like_image` had to guard against by hand, and no longer does.
//!
//! **Recognition is by CONTENT, never by extension.** A disk image under any
//! name is recognised, and a mis-named ordinary story file is not — the same
//! rule the readers apply file-by-file inside a volume, where it is not
//! negotiable: every Atari ST story is called `STORY.DAT`, five of nine Amiga
//! floppies call theirs `Story.data`, `.ima` and `.img` are one format, and the
//! ProDOS `2IMG` length field reads zero on every image in the corpus.
//!
//! ## A story that is on no single disk (SQ-0864)
//!
//! One release in the corpus does not fit "a volume holds a story": the Apple II
//! 5.25-inch presses of *Shogun* and *Zork Zero* page one story across **five**
//! and **four** floppies, and no one of them carries a whole game. That is not a
//! filesystem question — every volume mounts perfectly well and lists its
//! segment — so it is not a format's business and there is no row for it.
//!
//! It is [`MountedDisk::mount_set`]'s: one image is opened exactly as ever, and
//! the other volumes of its release are offered alongside so that the container
//! spanning them ([`crate::infocom_packed`]) can be asked. Every format gets
//! this and none of them pays for it — the companions are consulted only when
//! the named volume has no story of its own, which is true of a Shogun floppy
//! and false of every compilation disk here. [`MountedDisk::mount`] is that
//! call with no companions, so nothing that does not want a set sees one.
//!
//! A row does carry [`Format::extensions`], and that is not a crack in the rule:
//! it is the census a front-end scanning a DIRECTORY needs to decide which files
//! are worth OPENING, and what a file turns out to be is still
//! [`DiskImage::detect`]'s answer over its bytes. See [`DiskImage::extensions`]
//! for why it lives here and nowhere else — it lived in the TUI once, and went
//! stale the moment a format arrived (SQ-0849).

use crate::adf::{Adf, looks_like_story};
use crate::atr::Atr;
use crate::d64::D64;
use crate::dos33::Dos33;
use crate::g64::G64;
use crate::fat12::Fat12;
use crate::hfs::Hfs;
use crate::infocom_boot::InfocomBoot;
use crate::infocom_pics::InfocomPics;
use crate::iso9660::Iso9660;
use crate::prodos::ProDos;
use crate::xex::Xex;

/// Amiga, from the ZMSD §11.1.3 interpreter-number table (1 DECSystem-20,
/// 2 Apple IIe, 3 Macintosh, **4 Amiga**, 5 Atari ST, 6 IBM PC, 7 Commodore 128,
/// 8 Commodore 64, 9 Apple IIc, 10 Apple IIgs, 11 Tandy Color).
pub const AMIGA_INTERPRETER_NUMBER: u8 = 4;

/// Macintosh, from the same ZMSD §11.1.3 table, read from the standard rather
/// than recalled: *"Infocom used the interpreter numbers: 1 DECSystem-20,
/// 2 Apple IIe, **3 Macintosh**, 4 Amiga, 5 Atari ST, 6 IBM PC, …"* — and the
/// same section is explicit that this matters here in particular: *"In Version
/// 6, the decision is more serious, as existing Infocom story files depend on
/// interpreter number in many ways"*.
pub const MACINTOSH_INTERPRETER_NUMBER: u8 = 3;

/// Atari ST, from the same §11.1.3 table — *"… 4 Amiga, **5 Atari ST**,
/// 6 IBM PC …"* — and, unusually, corroborated by the machine's own
/// interpreters rather than by the standard alone (SQ-0835).
///
/// Infocom's Atari ST sources write this byte themselves, and say so. Both the
/// XZIP (Version 5) and the EZIP (Version 4) builds carry the identical pair of
/// lines, at `PINTWD EQU 30` — decimal 30 is `$1E`:
///
/// ```text
///   st/stx1.s:384   PINTWD  EQU     30      * INTERPRETER ID/VERSION
///   st/stx1.s:422   INTWRD  DC.B    5       * MACHINE ID FOR ATARI ST
///   st/stx1.s:731           MOVE.W  INTWRD,PINTWD(A2) * SET INTERPRETER ID/VERSION WORD
/// ```
///
/// **It is a flat constant, not a version-dependent rule** — which is the
/// question worth asking here, because the IBM PC's honest number *is*
/// version-dependent and that is exactly why [`DiskImage::Fat12Dos`] answers
/// `None`. The ST has no such rule. The one conditional in `st/stzip.s` is the
/// Version 3 assembly, and it declines the byte rather than claiming a different
/// machine:
///
/// ```text
///   st/stzip.s:339      IFEQ EZIP
///   st/stzip.s:340  INTWRD  DC.B    5       * MACHINE ID FOR ATARI ST
///   st/stzip.s:344      IFEQ CZIP
///   st/stzip.s:345  INTWRD  DC.B    0       * (UNUSED)
/// ```
///
/// Byte `$1E` carries no meaning before Version 4, so the Version 3 build leaves
/// it zero and comments it "(UNUSED)". Advertising 5 to a Version 3 story is
/// therefore harmless rather than merely tolerable, and it is *measured* so:
/// all thirty-two v3 stories in the nine-compilation ST corpus produce a
/// byte-identical trace under 1 and under 5.
pub const ATARI_ST_INTERPRETER_NUMBER: u8 = 5;

/// Apple IIgs, from the same §11.1.3 table — *"… 9 Apple IIc, **10 Apple
/// IIgs**, 11 Tandy Color"* — and, like the Atari ST's, corroborated by
/// Infocom's own interpreter rather than by the standard alone (SQ-0857).
///
/// The corroboration here is unusually direct, because the interpreter is **on
/// two of the disks in `stories/`**. `apple/yzip/rel.15/apple.equ` in
/// `github.com/erkyrath/infocom-zcode-terps` — the Apple II YZIP, Infocom's
/// Version 6 interpreter for the machine — names all three of the family's
/// numbers as constants:
///
/// ```text
///   apple/yzip/rel.15/apple.equ:136   IIeID   EQU  2   ; Apple ][e Yzip
///   apple/yzip/rel.15/apple.equ:137   IIcID   EQU  9   ; ][c Yzip
///   apple/yzip/rel.15/apple.equ:138   IIgsID  EQU  10  ; ][gs Yzip
/// ```
///
/// and `zboot.asm` writes whichever one it settled on into header `$1E`
/// (`ZINTWD EQU 30`, decimal 30 = `$1E`, in `zip.equ`):
///
/// ```text
///   apple/yzip/rel.15/zboot.asm:7   lda  ARG2+LO         ; get machine id!
///   apple/yzip/rel.15/zboot.asm:8   sta  ZBEGIN+ZINTWD   ; save before it gets zeroed
/// ```
///
/// **And it is NOT a flat constant — it is detected at boot**, which is the
/// question worth asking here because a version-dependent rule is exactly why
/// [`DiskImage::Fat12Dos`] answers `None`. `bsubs.asm`'s `MACHINE:` reads
/// ProDOS's own machine-ID bytes and picks one of the three:
///
/// ```text
///   ; Make sure we are on a good machine, like a ][c or ][e+/][gs
///   MACHINE:
///       lda MACHID1 / cmp #6 / bne BADMACH   ; nothing below an enhanced ][e
///       lda MACHID2 / bne MACH1
///       lda #IIcID                            ; Apple ][c thank you
///   MACH1:
///       sec / jsr MACHCHK / bcs OLDMACH       ; check for 'new' machine
///       lda #IIgsID                           ; this is a ][gs
///   OLDMACH:
///       lda #IIeID                            ; this is IIe
///   MACH2:
///       sta ARG2+LO                           ; save machine id
/// ```
///
/// `apple/yzip/rel.13/boot.lst` assembles that routine to
/// `AD B3 FB C9 06 D0 19 AD C0 FB D0 05 A9 09 4C CB 26 38 20 1F FE B0 04 A9 0A
/// D0 02 A9 02 85 65 60`, and the same bytes — with the one `jmp MACH2` operand
/// relocated by one, `4C CC 26` — occur **verbatim and byte-identical** in
/// `INFOCOM.SYSTEM` on both `Journey.2mg` and `Arthur Quest 4 Excalibur.2mg`, at
/// offset 1711 of each. Those are the ProDOS 8 launchers of the corpus's two
/// Version 6 releases. (The 22,528-byte `INFOCOM` interpreter beside them
/// differs between the two disks in exactly two bytes, so one interpreter serves
/// both presses.)
///
/// So the number is a property of **the machine the interpreter is running on**
/// and not of the disk, and §11.1.3 says as much: *"An interpreter should choose
/// the interpreter number most suitable for the machine it will run on."* The
/// row below is where lanthorn answers that question; see it for why the answer
/// is the top of the family.
pub const APPLE_IIGS_INTERPRETER_NUMBER: u8 = 10;

/// Commodore 128, from the same §11.1.3 table — *"… 6 IBM PC, **7 Commodore
/// 128**, 8 Commodore 64 …"* — read from the standard rather than recalled
/// (SQ-0869).
///
/// **Like ProDOS, the medium names a FAMILY**: a `.d64` is a 1541 image and a
/// 1541 hangs off a VIC-20, a C64, a C128 and a Plus/4 alike, so the geometry
/// cannot say which machine a disk was pressed for. Unlike ProDOS, the two
/// candidates here are distinguishable **on the disk**, and the corpus holds one
/// of each:
///
/// ```text
///   Hitchhiker's 1984   track 17 sector 0 is a BASIC stub, `SYS(2063)`      → C64
///   TRINITY1.D64 1986   track  1 sector 0 opens `CBM`, the C128 autoboot    → C128
/// ```
///
/// *Trinity*'s is not a near thing. Its boot sector reads `43 42 4D` — the
/// Commodore 128's autoboot signature, which a C64 does not look for — and then
/// `A5 D7 C9 80 F0 03 20 5F FF`, testing the C128's 40/80-column flag at `$D7`
/// and calling `$FF5F`, the C128 Kernal's SWAPPER. The interpreter behind it
/// references **`$FF00`, the C128's MMU configuration register, forty times**,
/// along with the `$D500` MMU block. None of those addresses is a register on a
/// Commodore 64. And the disk could not boot on one even if it wanted to: its
/// directory sector holds story data, so there is no file for a C64's
/// `LOAD"*",8,1` to find.
///
/// **Which is why the row answers 7 and not 8.** Byte `$1E` carries no meaning
/// before Version 4 — see [`ATARI_ST_INTERPRETER_NUMBER`], where Infocom's own
/// Version 3 build leaves it zero and comments it "(UNUSED)" — so the Version 3
/// *Hitchhiker's* cannot notice what it is told, and the C64 press in the corpus
/// is exactly the disk with no opinion. The one Commodore story here that reads
/// the byte is on a Commodore 128 disk. Answering 8 would be fitting the row to
/// the fixture that provably ignores it.
///
/// **And declining is not available**, on SQ-0857's finding: `None` falls through
/// to 1, the DECSystem-20, so it does not leave the machine unnamed, it names it
/// something else. Measured on *Trinity* itself — release 12 serial 860926,
/// assembled off both floppies — the number changes exactly one thing, the
/// VERSION block, which prints `Interpreter 7 Version A` where the fall-through
/// prints `Interpreter 1 Version A`. The rest of the transcript is byte-identical
/// under 1, 7 and 8.
///
/// So no [`crate::medium`] caller ships a Commodore *profile*: nothing here
/// establishes a screen, a palette or a colour pair, nothing in the corpus
/// behaves differently, and Infocom pressed no Version 6 game for the machine at
/// all. A profile would be a bundle with no measurement behind it, which is the
/// rule the ST and Apple rows were held to in the other direction. `--interpreter
/// 8` reaches the Commodore 64 by naming it, as every front-end pins.
pub const COMMODORE_128_INTERPRETER_NUMBER: u8 = 7;

/// Which release medium a story was mounted out of, when it was one at all.
///
/// The variant is the mount's own answer — every one of them is decided by the
/// image's own filesystem rather than by its filename. Callers use it to NAME
/// the container (the picker's TYPE column) and, via
/// [`DiskImage::interpreter_number`], to imply the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiskImage {
    /// An Amiga AmigaDOS release floppy — conventionally `.adf` (SQ-0719).
    Adf,
    /// A Macintosh HFS volume, bare or inside a DiskCopy 4.2 wrapper —
    /// conventionally `.image` (SQ-0837).
    Hfs,
    /// An IBM PC / MS-DOS FAT12 release floppy — conventionally `.ima` or
    /// `.img`, and the two are the same thing (SQ-0833).
    Fat12Dos,
    /// An Atari ST GEMDOS floppy — conventionally `.st` (SQ-0835). The **same**
    /// FAT12 filesystem as [`DiskImage::Fat12Dos`], down to the byte offsets of
    /// the BPB, and a different machine; see [`crate::fat12`] for the
    /// discriminator.
    Fat12AtariSt,
    /// An Apple II ProDOS volume, bare or inside a `2IMG` wrapper —
    /// conventionally `.2mg` (SQ-0836).
    ///
    /// **The one format here that names a FAMILY rather than a machine.**
    /// ProDOS is the Apple II's filesystem from the IIe on, and ZMSD §11.1.3
    /// gives that family three interpreter numbers — 2 Apple IIe, 9 Apple IIc,
    /// 10 Apple IIgs. Infocom's own Apple interpreter settles which by *detecting
    /// the machine at boot* rather than by pressing three disks, so the ambiguity
    /// is a fact about the medium and not a gap in the evidence. See this
    /// variant's row in [`FORMATS`] for why
    /// [`DiskImage::interpreter_number`] nevertheless answers, and with what.
    ProDos,
    /// A **raw self-booting Apple II 5.25-inch disk** — no filesystem at all,
    /// story sectors in DOS 3.3 logical order, conventionally `.dsk` like the
    /// ProDOS press it sits beside (SQ-0868).
    ///
    /// The one format here whose bytes are **not a volume**. Every other row
    /// names a filesystem and delegates to a reader for it; this one has nothing
    /// to enumerate, and the story is found by de-interleaving the image and
    /// verifying a run of sectors against the story's own header checksum. See
    /// [`crate::infocom_boot`] for the measurement, and for what keeps this sniff
    /// disjoint from [`DiskImage::ProDos`]'s when the two share a size, a sector
    /// order and a spelling.
    InfocomBootDisk,
    /// A **Commodore 1541 disk** — 35 tracks, 174,848 bytes, conventionally
    /// `.d64` (SQ-0869).
    ///
    /// The second format here whose bytes are not a volume, and the first whose
    /// story is **not on one disk**. Commodore DOS is present on all three images
    /// in the corpus and used by none of them: *Trinity* writes its story over
    /// its own directory sector, and *Hitchhiker's* keeps a decorative one whose
    /// only file is a BASIC loader. The story is raw sectors, laid out
    /// differently by the 1984 and 1986 presses, and *Trinity* is a Version 4
    /// game of 262,064 bytes that no single 174,848-byte floppy could hold. See
    /// [`crate::d64`].
    CommodoreD64,
    /// The **same 1541 disk, dumped as a raw GCR bitstream** — conventionally
    /// `.g64` (SQ-1095).
    ///
    /// A row of its own rather than a second spelling on the one above, and the
    /// reason is that this is not a wrapper. [`DiskImage::Hfs`] and
    /// [`DiskImage::ProDos`] each accept a bare volume *or* the same volume
    /// behind a DiskCopy 4.2 or `2IMG` header, because the sectors either way
    /// are the file's own bytes. A G64 has no sectors in it at all: it is sync
    /// marks, GCR-encoded blocks and gaps, and every sector this reader hands
    /// on was computed. Folding that into the `.d64` sniff would put a
    /// forty-track bitstream decode inside a test whose whole virtue is that it
    /// leaves at the first line on a file of the wrong length.
    ///
    /// Everything after the decode is the row above's: the same 1541 geometry,
    /// the same checksum scan, the same interpreter number, and the same answer
    /// about paging a story across two disks. See [`crate::g64`].
    CommodoreG64,
    /// An **ISO 9660 CD-ROM** — *The Lost Treasures of Infocom* I and II
    /// (SQ-0871).
    ///
    /// The second CD here and a different construction from the first: no Apple
    /// partition map and no HFS volume anywhere, just ISO 9660 with Apple's
    /// extensions layered in, which is the ordinary way a hybrid CD is made.
    /// [`DiskImage::Hfs`] reads the Masterpieces disc through its Macintosh
    /// PARTITION and cannot help here; these two opened as nothing at all.
    ///
    /// Both machines' builds share the one filesystem, so this row states **no**
    /// interpreter number and the machine is a per-file question answered from
    /// the Apple extension's Finder metadata — see
    /// [`machine_from_finder`] and [`crate::iso9660`].
    Iso9660,
    /// An **Atari 8-bit floppy** — an `.atr` image with an Atari DOS 2
    /// filesystem in it, or what is left of one (SQ-1458).
    ///
    /// The container is what identifies this medium, not the filesystem, and
    /// that is measured rather than convenient: nine of the fourteen S.A.G.A.
    /// sides in `stories/scott-dialects/atari/` have game data where their
    /// volume table of contents should be, because the release masters its
    /// database straight over it. A side with no directory left mounts and lists
    /// nothing, and its payload is reached through
    /// [`crate::atr::IMAGE_ENTRY`]. See [`crate::atr`].
    AtariDos2,
    /// An **Apple II DOS 3.3 volume** — a flat 5.25-inch sector dump,
    /// conventionally `.dsk` or `.do` (SQ-1458).
    ///
    /// The **third** format wearing the Apple II's 143,360-byte `.dsk`, beside
    /// [`DiskImage::ProDos`] and [`DiskImage::InfocomBootDisk`], and the one the
    /// spelling is named after. The three are kept apart by content and not by
    /// order: no ProDOS volume and no raw self-booting disk in the corpus has
    /// 35/16/256/122 in the four VTOC fields this one requires. See
    /// [`crate::dos33`].
    AppleDos33,
    /// An **Atari 8-bit binary-load executable** — `.xex` (SQ-1458).
    ///
    /// The one row here whose bytes are not a disk in any sense: no filesystem,
    /// no sectors, no geometry, just a list of load records and the memory they
    /// put together. It is a row rather than a special case because the question
    /// a front-end asks — *what is on this, and can I read it?* — has the same
    /// answer shape for a loadable binary as for a floppy, and the whole point
    /// of this table is that no caller may know the difference. See
    /// [`crate::xex`].
    AtariXex,
}

impl DiskImage {
    /// Which release medium `raw` is, or `None` when it is not one.
    ///
    /// The sniffs are disjoint by construction — AmigaDOS is identified by its
    /// `DOS` boot block and HFS by a volume signature at a fixed offset (bare, or
    /// past a DiskCopy 4.2 header) — so the order of [`FORMATS`] is a formality
    /// rather than a precedence.
    ///
    /// **That promise survived a second format arriving on the same medium**
    /// (SQ-0868). [`DiskImage::ProDos`] and [`DiskImage::InfocomBootDisk`] are
    /// both 143,360-byte Apple II 5.25-inch dumps in the drive's sector order,
    /// both spelled `.dsk`, and the difference between them is entirely in what
    /// is inside — so "a fixed offset" was not available to keep them apart.
    /// They are kept apart by the boot-disk sniff declining a ProDOS volume
    /// outright, which is a construction and not an ordering: swap the two rows
    /// and every image in the corpus still gets the same answer.
    ///
    /// Whatever this recognises, [`MountedDisk::mount`] opens: both walk the same
    /// table, so the two cannot drift apart.
    pub fn detect(raw: &[u8]) -> Option<DiskImage> {
        format_of(raw).map(|f| f.image)
    }

    /// Every format this crate reads, in table order. The census the API walks
    /// — a format is here exactly when it has a row in [`FORMATS`].
    pub fn all() -> impl Iterator<Item = DiskImage> {
        FORMATS.iter().map(|f| f.image)
    }

    /// This variant's row. Every variant has one; a variant added to the enum
    /// without a row in [`FORMATS`] is a format nothing can detect, mount or
    /// name, and the census test in this module catches it.
    fn row(self) -> &'static Format {
        FORMATS.iter().find(|f| f.image == self).expect("every DiskImage variant has a FORMATS row")
    }

    /// The acronym a story list shows beside the format: `Z6 (ADF)`, `Z6 (HFS)`.
    pub fn label(self) -> &'static str {
        self.row().label
    }

    /// The Z-machine interpreter number this medium DEFAULTS to — header byte
    /// `$1E`, ZMSD §11.1.3 — or `None` to leave whatever rule was already in
    /// force (each front-end's own default: Frotz's 6-for-v6, 1-otherwise).
    ///
    /// A default, never an override: every caller must let an explicitly
    /// requested number win over this one. That ordering is the other half of
    /// the user's rule and is pinned on both front-ends.
    ///
    /// [`DiskImage::Hfs`] answered `None` until SQ-0838, on the grounds that the
    /// number is not inert — a game reads `$1E` and can take machine-specific
    /// paths — and that the Macintosh's screen, colours and palette were not
    /// established by anything in hand, so `3` would have been a byte without a
    /// machine behind it. They are established now, out of Infocom's own
    /// Macintosh interpreter (`mac/xzip.lst`, `mac/gfx.p`), and the whole bundle
    /// ships together in `app::interpreter::InterpreterProfile::Macintosh`.
    pub fn interpreter_number(self) -> Option<u8> {
        self.row().interpreter_number
    }

    /// Does this medium name the IBM PC, whose number is a version rule?
    /// See [`Row::implies_ibm_pc`] — this is the other half of
    /// [`Self::interpreter_number`]'s `None`.
    pub fn implies_ibm_pc(self) -> bool {
        self.row().implies_ibm_pc
    }

    /// The filename extensions this format is CONVENTIONALLY given — lowercase,
    /// no leading dot.
    ///
    /// **A pre-filter, never evidence.** Nothing may conclude a format from a
    /// name: a front-end walking a directory uses this to decide which files are
    /// worth reading, and then asks [`DiskImage::detect`] what it actually got.
    /// So a disk image under an unexpected name still mounts by every path that
    /// sniffs content, and a `.img` full of holiday photos is opened, refused and
    /// never listed.
    ///
    /// **Why it is a property of the row.** The TUI's story picker kept its own
    /// extension list, and that list was the "nothing else anywhere" in
    /// [`FORMATS`]' doc that turned out to exist. SQ-0833 and SQ-0835 added the
    /// DOS and Atari ST rows; the picker never learned their names, so a shelf
    /// full of `.ima` and `.st` floppies that mount perfectly well was simply
    /// absent from the story list, silently, for two quests (SQ-0849). A census
    /// the table owns cannot drift from the table.
    pub fn extensions(self) -> &'static [&'static str] {
        self.row().extensions
    }
}

/// Every extension any format in [`FORMATS`] is conventionally given, in table
/// order — the whole census, for a caller that has a filename and no bytes yet.
///
/// This is what a directory scan pre-filters on; see [`DiskImage::extensions`]
/// for what it is and is not allowed to mean. The union is what a caller wants,
/// because which row claimed a spelling is [`DiskImage::detect`]'s business.
///
/// **The rows' sets are not disjoint, and since SQ-0868 the corpus proves it.**
/// `.dsk` is claimed by [`DiskImage::ProDos`] and by
/// [`DiskImage::InfocomBootDisk`], because the Apple II 5.25-inch press wears
/// one spelling and is two different formats underneath — *Shogun*'s floppies
/// are ProDOS volumes and *Planetfall*'s retail disk has no filesystem on it at
/// all. This function therefore repeats a spelling, which costs a pre-filter
/// nothing (it is a membership test) and would cost a name-to-format lookup
/// everything — which is why there is no such lookup.
pub fn image_extensions() -> impl Iterator<Item = &'static str> {
    FORMATS.iter().flat_map(|f| f.extensions.iter().copied())
}

// ── The one table ─────────────────────────────────────────────────────────────

/// One disk format: how to recognise it, how to open it, and what it implies.
///
/// The whole point of the struct is that a format is a **row**, not a branch
/// scattered through the callers. Nothing outside this module ever sees one.
struct Format {
    /// The [`DiskImage`] variant this row is.
    image: DiskImage,
    /// See [`DiskImage::label`].
    label: &'static str,
    /// See [`DiskImage::interpreter_number`].
    interpreter_number: Option<u8>,
    /// Does this medium name the **IBM PC**, whose §11.1.3 number is a version
    /// RULE rather than a constant? (SQ-0930)
    ///
    /// `interpreter_number: None` above says two different things and this is the
    /// half that got lost. A DOS floppy's `None` means "the rule already in force
    /// IS the IBM PC's" — a machine, stated by deferral. The ISO's means "this
    /// disc is both machines and a number here would be wrong for half of it".
    /// One is a machine and the other is not, and reading them alike made a DOS
    /// floppy resolve as *no medium at all*: it took the fallback profile, so the
    /// story was told DECSystem-20 and the machine's own colours never applied.
    implies_ibm_pc: bool,
    /// See [`DiskImage::extensions`]. Lowercase, no dot, at least one — a row
    /// with none is a format a directory scan can never offer, and the census
    /// test in this module says so.
    extensions: &'static [&'static str],
    /// The content sniff — [`Volume::looks_like`] for the reader below.
    looks_like: fn(&[u8]) -> bool,
    /// The mount — [`Volume::mount`], boxed so the table can be one type.
    mount: fn(Vec<u8>) -> Option<Box<dyn Volume>>,
    /// Whether this format's multi-disk assembler needs the volumes as whole
    /// IMAGES rather than as files — see [`MountedDisk::sides`].
    ///
    /// True only for a format that pages a story across raw sectors, which is
    /// the Commodore's case and nobody else's: the Apple's packed volume pages
    /// across `.D1`…`.D5`, which are files and arrive in `across`. It is a row
    /// here rather than a `match` in `mount_set` for the reason the whole table
    /// exists — no front-end and no reader should know a format's name.
    ///
    /// It is load-bearing for COST, not just correctness. `mount_set` must copy
    /// the image before the mount consumes it, and it cannot know whether the
    /// volume has a story of its own until after. Cloning unconditionally made
    /// every ordinary mount pay for a copy it dropped — 354 MB on a hybrid CD
    /// (SQ-0875).
    pages_across_images: bool,
}

/// **Every disk format lanthorn reads.** Adding one is a row here plus the
/// `impl Volume` it names, and nothing else anywhere: `detect`, `mount`,
/// `label`, `interpreter_number`, **the extensions a directory scan
/// pre-filters on**, the TUI's story loading and picture resolution, and
/// `zvm-cli`'s disk menu all read this table and none of them knows a format's
/// name.
///
/// The extensions column is the newest of those, and it is here because the one
/// front-end that needed it kept its own copy instead: `picker::STORY_EXTS`
/// listed `adf` and `image` and never heard about `ima`, `img` or `st`, so two
/// shipped formats were unreachable from the story list. That is exactly the
/// half-wiring the table exists to make impossible, one column late (SQ-0849).
///
/// **The Apple II 5.25-inch press arrived without one** (SQ-0864), and that is
/// the table working rather than the table being bypassed. SQ-0852 queued
/// `shogun_s1..s5.dsk` and `zork_zero_1..4.dsk` here as a sixth format, on the
/// reading that a bare 143,360-byte DOS-order sector dump carries the packed
/// volume directly, through a per-disk block map, with no filesystem under it.
///
/// Re-derived, that reading was one layer too low. The block map is a **ProDOS
/// index block**, because the image is a **ProDOS volume** — the whole of the
/// difference from a `.2mg` is that its sectors are in the order the 5.25-inch
/// drive numbers them. De-interleave it ([`crate::dos_order`]) and block 2 is an
/// ordinary volume directory naming `SHOGUN.1`…`SHOGUN.5` and `ZORK0.1`…
/// `ZORK0.4`, each holding its segment as an ordinary ProDOS file
/// (`SHOGUN.D3` is a tree file; the hand-rolled map could not have read it).
/// So the medium is ProDOS, it wears the ProDOS row, it answers the Apple IIgs
/// like every other ProDOS volume, and `.dsk` is a spelling that row claims.
///
/// What was genuinely missing was not a row but a **set**: the segments are on
/// five different volumes and [`Volume::mount`] takes one image. That is
/// [`MountedDisk::mount_set`], above the table and format-neutral, so the two
/// sets ship without this list growing an entry.
const FORMATS: &[Format] = &[
    Format {
        image: DiskImage::Adf,
        label: "ADF",
        interpreter_number: Some(AMIGA_INTERPRETER_NUMBER),
        implies_ibm_pc: false,
        // Every Amiga floppy in the corpus is `.adf`; the format has no second
        // customary spelling.
        extensions: &["adf"],
        looks_like: <Adf as Volume>::looks_like,
        mount: mount_boxed::<Adf>,
        pages_across_images: false,
    },
    Format {
        image: DiskImage::Hfs,
        label: "HFS",
        interpreter_number: Some(MACINTOSH_INTERPRETER_NUMBER),
        implies_ibm_pc: false,
        // `.image` is DiskCopy 4.2's own name and what the corpus uses (`Zork
        // Zero Disk.image`). Macintosh volumes also circulate as `.img` and
        // `.dsk`; the first is admitted by the DOS row below and the second by
        // the ProDOS row at the bottom, and the union is what a scan
        // pre-filters on — so an HFS volume under either name is opened, and
        // `looks_like_hfs` is what then claims it. Which row spells a name is
        // never which format the bytes are.
        //
        // `bin` is the second spelling, and the corpus earned it (SQ-0870):
        // `Classic Text Adventure Masterpieces of Infocom (USA).bin` is a raw
        // MODE1/2352 dump of a hybrid CD whose third Apple partition is the
        // Macintosh collection. A raw dump is what `.bin` means, so a scan that
        // would not open one could never offer the disc — SQ-0849's defect, on a
        // medium 350 MB in size instead of 800 KB. What the bytes ARE is still
        // `looks_like_hfs`'s answer, and it declines a `.bin` that is anything
        // else.
        //
        // `iso` is the third, and it was declined once (SQ-0870) on the ground
        // that "nothing in the corpus is one". That ground does not hold up
        // (SQ-0879). A cooked 2048-byte image is the ORDINARY way a hybrid disc
        // is archived — the raw `.bin` here cooks to a perfectly readable
        // 308 MB `.iso`, walked in place with no unwrapping at all — so the
        // question was never whether the reader could take one, only whether a
        // scan would offer it. It would not: the file was skipped before its
        // bytes were ever looked at, which is SQ-0849's defect verbatim, and the
        // only symptom is a disc that is silently absent from the story list
        // while opening it by name works fine.
        //
        // `dc42` is the fourth, and it is the same argument a fourth time (SQ-0910).
        // DiskCopy 4.2 is the wrapper this row already unwraps; `.dc42` is simply
        // what the tool's own images are called when the archive does not shorten
        // it to `.image`. The five *Lost Treasures of Infocom* Macintosh discs wear
        // it, and every one of them mounted here while a directory scan skipped the
        // file before its bytes were ever looked at — SQ-0849's defect verbatim,
        // and its only symptom is a disc silently missing from the story list while
        // opening it by name works.
        //
        // A spelling is still claimed when a medium wears it — and `.iso` is
        // what this medium wears everywhere it is distributed.
        //
        // `toast` is the fifth, by the same rule a fifth time (SQ-1055). Roxio
        // Toast writes a bare HFS volume under that name — `stories/Shogun.toast`
        // carries the Macintosh press of *Shogun*, release 292 / serial 890314, and
        // its Master Directory Block sits at offset 1024 with no wrapper of any
        // kind in front of it. It mounted here the day it arrived; only the
        // DIRECTORY SCAN skipped it, on its name, before its bytes were looked at —
        // which is SQ-0849's defect verbatim and, again, presents as a disc
        // silently missing from the story list while opening it by name works.
        extensions: &["image", "bin", "iso", "dc42", "toast"],
        looks_like: <Hfs as Volume>::looks_like,
        mount: mount_boxed::<Hfs>,
        pages_across_images: false,
    },
    // ── One filesystem, two machines (SQ-0833, SQ-0835) ──────────────────────
    //
    // These two rows share a reader and differ only in the sniff, because
    // GEMDOS puts its BPB at the DOS offsets and a plain DOS parser reads an
    // Atari disk with no Atari-specific code in it. The MACHINE is a separate
    // question, asked of the boot sector — see `crate::fat12`.
    Format {
        image: DiskImage::Fat12Dos,
        label: "DOS",
        // **`None`, and that is the IBM PC's answer rather than a gap.** This
        // codebase's IBM PC bundle — `app::interpreter::InterpreterProfile::IbmPc`,
        // where a DOS disk resolves — deliberately returns no number of its
        // own, because the honest one is version-dependent (Frotz's rule: 6 for
        // Version 6, 1 otherwise) and no single constant expresses it. So
        // `None` here means "the rule already in force IS the IBM PC's", which
        // is exactly true, and a DOS floppy behaves as it always has.
        //
        // Hard-coding ZMSD §11.1.3's 6 would not be inert: `BEYONDZO.DAT` sits
        // on `floppy1.ima` and *Beyond Zork* swaps Font 3 arrows for CP437
        // character graphics when it believes it is on an IBM PC. That may well
        // be the authentic Lost Treasures experience — it is also a visible
        // rendering change on real media that nothing in this lane establishes,
        // and it belongs to whoever can look at it.
        interpreter_number: None,
        // …and THIS `None` is a deferral, not an absence: see the comment above.
        // The machine is the IBM PC; only its number is a rule (SQ-0930).
        implies_ibm_pc: true,
        // Two spellings of one thing, as this module's header already says:
        // `floppy1.ima` and `disk1.img` are the same raw sector dump and the
        // same reader opens both.
        extensions: &["ima", "img"],
        looks_like: crate::fat12::looks_like_dos,
        mount: mount_boxed::<Fat12>,
        pages_across_images: false,
    },
    Format {
        image: DiskImage::Fat12AtariSt,
        label: "ST",
        // **5, the Atari ST** (SQ-0835's profile half). This row answered `None`
        // for one commit, on the argument that no ST press of a graphical v6
        // title exists so the number would be "a byte with no verified machine
        // behind it". That argument was wrong, and its own premise is why: the
        // failure it guards against — a number that changes what games do while
        // the artwork keeps another machine's scale — **cannot arise on a corpus
        // with no artwork in it.** All thirty-nine stories across the nine ST
        // compilations are v3, v4 or v5, so there is nothing for the number to
        // disagree with.
        //
        // What replaced the argument is evidence. Infocom's own ST interpreters
        // write 5 into `$1E` unconditionally and label it "MACHINE ID FOR ATARI
        // ST"; see [`ATARI_ST_INTERPRETER_NUMBER`], which quotes them and shows
        // the byte is a flat constant rather than the version-dependent rule
        // that keeps [`DiskImage::Fat12Dos`] at `None`.
        //
        // The rest of the bundle is in `app::interpreter::InterpreterProfile::AtariSt`,
        // and it declines the one member nothing establishes: the ST never had a
        // YZIP, so it has no Version 6 art geometry to state.
        interpreter_number: Some(ATARI_ST_INTERPRETER_NUMBER),
        implies_ibm_pc: false,
        // `.st` is the raw ST sector dump, which is what this reader opens and
        // what all nine compilations in the corpus are. **Not `.msa`**: Magic
        // Shadow Archiver images are RLE-compressed with their own header, not
        // FAT12 at offset zero, so `looks_like_atari_st` refuses one and a row
        // claiming the name would be advertising a format with no reader — the
        // same half-wiring the extensions column was added to end. It arrives
        // with a decompressor or not at all.
        extensions: &["st"],
        looks_like: crate::fat12::looks_like_atari_st,
        mount: mount_boxed::<Fat12>,
        pages_across_images: false,
    },
    Format {
        image: DiskImage::ProDos,
        label: "ProDOS",
        // **10, the Apple IIgs** (SQ-0857). This row answered `None` from
        // SQ-0836 until SQ-0857, and the argument then was the one immediately
        // above at [`DiskImage::Fat12Dos`]: ProDOS names the Apple II FAMILY, not
        // a machine, and ZMSD §11.1.3 numbers three of them (2 IIe, 9 IIc,
        // 10 IIgs). That premise is not merely still true, it is now proven from
        // Infocom's own code — see [`APPLE_IIGS_INTERPRETER_NUMBER`], where the
        // Apple II YZIP's `MACHINE:` routine picks between all three at boot, and
        // where those bytes are located on the two Version 6 disks in `stories/`.
        //
        // **What changed is the realisation that `None` is not neutral here.**
        // The `Fat12Dos` row can decline because zvm's own rule — Frotz's, 6 for
        // Version 6 and 1 otherwise — *is* the IBM PC's rule, so declining leaves
        // a DOS floppy describing itself correctly. On a ProDOS volume the same
        // deferral lands on 1 (DECSystem-20) or, for Version 6, on 6 — the IBM
        // PC, a machine on another continent, and the one value `zvm`'s `exec.rs`
        // gates its CP437 remap on. Declining does not leave the Apple II
        // unnamed; it names it something else.
        //
        // §11.1.3 asks the question this row actually has to answer: *"An
        // interpreter should choose the interpreter number most suitable for the
        // machine it will run on."* The number is a property of the machine in
        // front of the player — which is precisely why Infocom detected it rather
        // than pressing it — so the question is which Apple II lanthorn is. Of
        // the three the YZIP will run on at all (`cmp #6 / bne BADMACH` refuses
        // anything below an enhanced IIe), the IIgs is the top, and it is the one
        // a modern terminal with colour and a large screen actually resembles.
        // The other two remain reachable by naming them: `--interpreter 2`
        // or `9` outranks this row, as every front-end pins.
        //
        // **Measured, not assumed** — the same way SQ-0835 settled the ST's 5.
        // All thirty-one stories on the ten `.2mg` images were traced under the
        // default rule and under 10. Twenty-four are byte-identical (every
        // Version 3 story, including the high-ASCII-serial *Leather Goddesses*
        // SQ-0856 made visible, plus *A Mind Forever Voyaging* and *Bureaucracy*).
        // Five print the number in their VERSION block and are otherwise
        // unchanged (Hitchhiker's, Trinity, Sherlock, Border Zone, Nord and
        // Bert). **One behaves differently, twice, and it is the right
        // difference**: *Beyond Zork* r57 s871221 — on both the GS/OS `BZ.DAT`
        // press and the Lost Treasures `BEYOND.ZORK` one — stops asking "Is this
        // a VT220?" and goes straight to BEGIN/RESTORE/QUIT, because an Apple
        // IIgs is not a terminal that might or might not have line-drawing
        // characters. That is the identical finding SQ-0835 recorded for the ST,
        // on the identical game. It also answers VERSION with **"Apple //gs
        // Color Version A"** where it used to say "DEC-20" — the story naming
        // the machine in Infocom's own spelling, which no part of this codebase
        // supplied.
        //
        // The rest of the bundle is `app::interpreter::InterpreterProfile::AppleIIgs`,
        // and it declines the one member nothing here establishes: the Apple's
        // Version 6 screen is 140x192 on a 3x9 cell, which is not a standard
        // window in this codebase's sense at all. See that knob's docs.
        interpreter_number: Some(APPLE_IIGS_INTERPRETER_NUMBER),
        implies_ibm_pc: false,
        // Two spellings, one filesystem. `.2mg` is the wrapper every 3.5-inch
        // image in the corpus wears; `.dsk` is what a 5.25-inch dump is called,
        // and SQ-0864 established that those are ProDOS volumes too — the same
        // reader, one de-interleave earlier (see [`crate::dos_order`]). Nine of
        // them are in `stories/`, so the spelling is earned rather than assumed.
        //
        // `.dsk` is also a Macintosh spelling, which costs nothing: the census
        // is a union a directory scan pre-filters on, and `looks_like_hfs`
        // claims an HFS `.dsk` before this row is ever asked.
        //
        // `.po` is the third, and it is the corpus that earned it (SQ-0863).
        // SQ-0864 declined the spelling on the ground that nothing in `stories/`
        // was a BARE ProDOS volume — this reader has always opened one, but a
        // census entry no medium justifies is a guess. `Journey.po` is now here:
        // the 3.5-inch consolidated pressing of *Journey*, volume `JOURNEY.3.5`,
        // carrying all five `JOURNEY.1/`…`JOURNEY.5/` segments where the flat
        // `Journey.2mg` beside it carries four. It mounted the day it arrived,
        // because the reader falls back to a bare volume at offset 0 — and it
        // was invisible in the story list, which is the only place most people
        // look (SQ-0849's defect, exactly).
        //
        // **Not `.hdv`**, still: nothing in the corpus is one, and the argument
        // above is the whole argument. A bare volume under any name still mounts
        // when it is opened directly — the extension is a directory scan's
        // pre-filter, never the recogniser.
        extensions: &["2mg", "dsk", "po"],
        looks_like: <ProDos as Volume>::looks_like,
        mount: mount_boxed::<ProDos>,
        pages_across_images: false,
    },
    Format {
        image: DiskImage::InfocomBootDisk,
        // "Boot", because self-booting is exactly what distinguishes it: the row
        // above is a disk you mount, this is a disk you start. Not "DOS 3.3" —
        // the sector ORDER is DOS 3.3's and there is no DOS 3.3 filesystem
        // anywhere on it, and naming it after a filesystem it does not have is
        // wrong in the one way that matters here.
        label: "Boot",
        // **10, the Apple IIgs — the same answer as the ProDOS row above, and
        // for the reason that row gives.** SQ-0857 settled that declining is not
        // neutral on an Apple II: `None` lands on 1 (DECSystem-20), so it does
        // not leave the machine unnamed, it names it something else. That
        // argument does not care which filesystem the disk has, and neither does
        // ZMSD §11.1.3's question — *"An interpreter should choose the
        // interpreter number most suitable for the machine it will run on"* — so
        // two Apple II rows answering two different numbers would be saying the
        // number is a property of the DISK, which is precisely the reading
        // SQ-0857 rejected out of Infocom's own YZIP (it detects the machine at
        // boot; see [`APPLE_IIGS_INTERPRETER_NUMBER`]).
        //
        // **Nothing observable rides on it here**, and that is worth stating
        // plainly rather than leaning on. Byte `$1E` carries no meaning before
        // Version 4 — the same fact that has Infocom's own Version 3 Atari ST
        // build leave it zero and comment it "(UNUSED)", quoted at
        // [`ATARI_ST_INTERPRETER_NUMBER`] — and the one disk of this kind in the
        // corpus is *Planetfall*, a Version 3 game. So this row is consistency
        // with its neighbour, not a claim about *Planetfall*; a Version 4 or 5
        // raw disk arriving later inherits the Apple II answer already argued
        // rather than a fresh guess.
        interpreter_number: Some(APPLE_IIGS_INTERPRETER_NUMBER),
        implies_ibm_pc: false,
        // The same spelling as the ProDOS row, which the census handles by being
        // a UNION: a directory scan pre-filters on `.dsk` and
        // [`DiskImage::detect`] then says which of the two formats the bytes are.
        // The two Apple II 5.25-inch presses in `stories/` are both `.dsk` and
        // are different formats, so this is the case that column was built for.
        extensions: &["dsk"],
        looks_like: <InfocomBoot as Volume>::looks_like,
        mount: mount_boxed::<InfocomBoot>,
        pages_across_images: false,
    },
    Format {
        image: DiskImage::CommodoreD64,
        // The machine, not the filesystem — because there is no filesystem in
        // use. "CBM" is what Commodore calls itself on its own disks, in the DOS
        // byte at `$02` of every BAM here and in the C128 autoboot signature at
        // track 1 sector 0 of `TRINITY1.D64`.
        label: "CBM",
        // **7, the Commodore 128**, argued in full at
        // [`COMMODORE_128_INTERPRETER_NUMBER`]. The short of it: `.d64` names the
        // 1541 and therefore a family, the corpus holds one C64 press and one
        // C128 press, and the C64 one is Version 3 and cannot read the byte —
        // so the only Commodore story here that reads it is on a C128 disk,
        // which boots through the C128 autoboot sector into an interpreter that
        // touches the C128 MMU forty times. Declining would land it on 1, the
        // DECSystem-20 (SQ-0857). `--interpreter 8` names the Commodore 64.
        interpreter_number: Some(COMMODORE_128_INTERPRETER_NUMBER),
        implies_ibm_pc: false,
        // `.d64` is the universal spelling for a 1541 dump and what all three
        // images in `stories/` wear — two of them shouting, which costs nothing:
        // the census is matched case-insensitively by every scan that uses it.
        //
        // **Not `.d71`, `.d81` or the 40-track `.d64`**: those are the 1571, the
        // 1581 and an extension, all different geometries, and nothing in the
        // corpus is one. A row claiming a spelling this reader would refuse is
        // the half-wiring the extensions column exists to end.
        extensions: &["d64"],
        looks_like: <D64 as Volume>::looks_like,
        mount: mount_boxed::<D64>,
        pages_across_images: true,
    },
    Format {
        image: DiskImage::CommodoreG64,
        // **"CBM", the same label as the `.d64` row.** The picker's TYPE column
        // names the MEDIUM, not the container it was dumped into, and every
        // other family here already reads that way: "HFS" covers `.image`,
        // `.dc42` and `.toast`; "ProDOS" covers `.2mg`, `.po` and `.dsk`. A
        // Commodore pair that reported "CBM" beside "GCR" was the only place a
        // library showed one 1541 floppy as two kinds of thing, which reads as a
        // distinction the player has to account for and cannot act on (SQ-1095
        // labelled it by encoding; the user reported the confusion).
        //
        // The encoding is not lost — `DiskImage::CommodoreG64` is still its own
        // variant, so anything that needs to tell a bitstream dump from a sector
        // dump still can. It is the one-word TYPE shown in a list that has no
        // room for the difference and no use for it.
        label: "CBM",
        // **7, the Commodore 128 — the same answer as the `.d64` row, and
        // deliberately not a fresh one.** §11.1.3's question is which machine
        // the interpreter runs on; a bitstream dump and a sector dump of the
        // same floppy cannot disagree about that, and two Commodore rows
        // answering differently would be saying the number is a property of the
        // CONTAINER. The argument is at [`COMMODORE_128_INTERPRETER_NUMBER`] and
        // is not restated. `--interpreter 8` names the Commodore 64.
        interpreter_number: Some(COMMODORE_128_INTERPRETER_NUMBER),
        implies_ibm_pc: false,
        // `.g64` is the only spelling the format has ever had; it is what the
        // signature says (`GCR-1541`) and what every nibbler writes.
        extensions: &["g64"],
        looks_like: <G64 as Volume>::looks_like,
        mount: mount_boxed::<G64>,
        // True, on the same ground as the row above: this is a raw-sector
        // Commodore press, and a release that pages a story across two floppies
        // pages it across two whatever they were dumped into.
        // `crate::d64::story_across` decodes a G64 side to sectors before it
        // reads one, so the two containers reach it in one shape.
        pages_across_images: true,
    },
    Format {
        image: DiskImage::Iso9660,
        // The filesystem, like every row that has one — and NOT the machine,
        // because this disc is both. `machine_from_finder` answers that per
        // file, and `image_of` below is what a listing and a boot ask.
        label: "ISO",
        // **None, and here that is a fact about the medium rather than a
        // deferral.** A CD-ROM is not a machine: these two carry Macintosh and
        // DOS builds in one filesystem, so a number stated by the row would be
        // wrong for half the disc — which is exactly the defect SQ-0876 fixed on
        // the hybrid HFS disc, and there the row at least had a machine to be
        // wrong about. A file Apple's extension identifies gets its own answer;
        // one it does not leaves the rule already in force, which is the IBM PC's
        // and is right for the DOS files on disc 1 that carry a blank creator.
        interpreter_number: None,
        // …and SQ-0930 makes that last sentence true rather than merely stated.
        // Every one of `LostTreasures1.iso`'s nineteen `PC/DATA/*.DAT` stories
        // carries a blank creator, so `image_of` cannot name their machine and
        // they fall through to this row. While that fall-through meant "no
        // machine", the whole PC side of the disc got no machine at all — no
        // period look, no page — on a disc that says `PC/` in the path.
        //
        // The MAC side is unaffected: `image_of` answers `Hfs` for anything
        // wearing Infocom's own creator, and only what it cannot name reaches
        // here.
        implies_ibm_pc: true,
        // `iso` is the universal spelling and what both discs wear. `bin` and
        // `img` are claimed by rows above and reach this one anyway, since a
        // scan pre-filters on the union and `looks_like` decides.
        extensions: &["iso"],
        looks_like: <Iso9660 as Volume>::looks_like,
        mount: mount_boxed::<Iso9660>,
        pages_across_images: false,
    },
    Format {
        image: DiskImage::AtariDos2,
        // The filesystem, like every row that has one, and spelled the way the
        // machine's own manuals do. NOT "Atari" — the row below the FAT12 pair
        // already says "ST", and a library showing "Atari" beside "ST" would be
        // offering a distinction a player cannot act on, which is the confusion
        // SQ-1095 fixed on the Commodore pair. "Atari DOS" is a filesystem name
        // and the 8-bit machine is the only one that has it.
        label: "Atari DOS",
        // **None, and here that is a third meaning of `None`** beside the two the
        // `implies_ibm_pc` field below distinguishes. A DOS floppy declines
        // because the IBM PC's number is a version rule already in force; an ISO
        // declines because a hybrid disc is two machines. This declines because
        // **ZMSD §11.1.3 has no Atari 8-bit at all** — its 5 is the Atari ST, a
        // different machine with a different processor, and claiming it here
        // would tell a story it was running on hardware that did not exist when
        // it was pressed.
        //
        // Nothing observable rides on it on this corpus: byte `$1E` carries no
        // meaning before Version 4 (see [`ATARI_ST_INTERPRETER_NUMBER`] for
        // Infocom's own Version 3 build leaving it zero), and every image here is
        // a Scott Adams release with no Z-machine header to write it into. If an
        // Infocom Atari 8-bit press ever arrives, the honest answer is still not
        // in the table and `--interpreter` still reaches whatever a person means.
        interpreter_number: None,
        // …and this is the half that says which kind of `None` it is. The IBM PC
        // is not implied: leaving the rule in force means leaving each front-end's
        // own default, which is what "no row in §11.1.3" deserves.
        implies_ibm_pc: false,
        // `.atr` is the only spelling this container has ever had, and all
        // fifteen Atari specimens in `stories/scott-dialects/atari/` wear it.
        // **Not `.xfd`** — that is the same disk with the header cut off, which
        // §7.3 says to refuse by name rather than guess at by size, and nothing
        // in the corpus is one.
        extensions: &["atr"],
        looks_like: <Atr as Volume>::looks_like,
        mount: mount_boxed::<Atr>,
        pages_across_images: false,
    },
    Format {
        image: DiskImage::AppleDos33,
        // The filesystem's own name, and the version is what tells it from the
        // IBM PC row's "DOS" — they are different filesystems on different
        // machines that happen to share three letters, and DOS 3.3 is what
        // everyone who has ever held one of these disks calls it. It sits beside
        // "ProDOS", the Apple II's other filesystem, which is the pairing a
        // player reading a TYPE column will actually make.
        label: "DOS 3.3",
        // **10, the Apple IIgs — the same answer as the two Apple II rows above,
        // and deliberately not a fresh one.** SQ-0857's argument does not care
        // which filesystem the disk has: §11.1.3 asks which machine the
        // interpreter runs on, Infocom's own Apple YZIP detects that at boot, and
        // three Apple II rows answering three numbers would be saying the number
        // is a property of the disk. Declining instead would land an Apple story
        // on 1, the DECSystem-20. Argued in full at
        // [`APPLE_IIGS_INTERPRETER_NUMBER`] and at the ProDOS row.
        //
        // As on the `InfocomBootDisk` row, nothing observable rides on it here:
        // no Z-machine story in `stories/` is on a DOS 3.3 volume.
        interpreter_number: Some(APPLE_IIGS_INTERPRETER_NUMBER),
        implies_ibm_pc: false,
        // `.dsk`, and only `.dsk` — the spelling all fourteen Apple sides in
        // `stories/scott-dialects/apple/` wear, and the third row to claim it.
        // The census is a union and a scan pre-filters on it, so sharing a
        // spelling with the two Apple rows above costs nothing;
        // [`DiskImage::detect`] still says which of the three a file is.
        //
        // **Not `.do`**, the explicit "DOS order" spelling, and **not `.po`**:
        // this reader would open a `.do` perfectly and nothing in the corpus is
        // one, which is the same standard `hdv` is held to on the ProDOS row —
        // a spelling gets a name when a medium wears it. `.po` is ProDOS order,
        // which this reader REFUSES for the reason
        // [`crate::dos33`]'s header measures, so claiming it would be worse
        // than unearned.
        //
        // **Not `.woz`**: a bit-preserving image has no sectors in it at all — it
        // is a GCR bitstream needing the whole nibble decode `scott-dialects-spec.md`
        // §7.4 sets out, which is [`crate::g64`]'s shape of work and a quest of
        // its own. Nothing in the corpus is one, and a row claiming a spelling
        // this reader would refuse is the half-wiring the extensions column
        // exists to end.
        extensions: &["dsk"],
        looks_like: <Dos33 as Volume>::looks_like,
        mount: mount_boxed::<Dos33>,
        pages_across_images: false,
    },
    Format {
        image: DiskImage::AtariXex,
        // The spelling, uniquely — because there is no medium under it to name.
        // Every other row here calls itself after a filesystem or a machine, and
        // this one is neither: it is a file of load records, and `XEX` is what
        // the Atari world has always called that. "Boot" above is the closest
        // precedent: a name for what the bytes DO rather than what they sit on.
        label: "XEX",
        // None, on exactly the ground the `.atr` row states: §11.1.3 numbers no
        // Atari 8-bit, and the one specimen is a Scott Adams memory image with no
        // Z-machine header to write a byte into.
        interpreter_number: None,
        implies_ibm_pc: false,
        // `.xex` is the spelling the one specimen wears and the one the format is
        // universally called. **Not `.obj`, `.com` or `.bin`** — the same file
        // does wear those, and every one of them is claimed by something else
        // entirely on a modern shelf; a census entry no medium here justifies is
        // a guess, and this reader opens the file by content whatever it is
        // called.
        extensions: &["xex"],
        looks_like: <Xex as Volume>::looks_like,
        mount: mount_boxed::<Xex>,
        pages_across_images: false,
    },
];

impl Volume for Iso9660 {
    fn looks_like(raw: &[u8]) -> bool {
        Iso9660::looks_like_iso9660(raw)
    }

    fn mount(raw: Vec<u8>) -> Option<Iso9660> {
        Iso9660::mount(raw).ok()
    }

    fn volume_name(&self) -> Option<&str> {
        // The PVD names every disc, and these two use it for something a person
        // recognises: `INFOCOM` and `LOST TREASURES II`.
        Some(Iso9660::volume_name(self)).filter(|n| !n.is_empty())
    }

    fn file_count(&self) -> usize {
        self.files().len()
    }

    fn contents(&self) -> Vec<(String, Vec<u8>)> {
        // By PATH, like HFS and FAT12 — three files on disc 2 are called
        // `STORY.DATA` and the folder is the only thing between them.
        self.files().iter().filter_map(|e| self.read(e).map(|b| (e.path(), b))).collect()
    }

    fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        Iso9660::read_named(self, name)
    }

    fn story(&self) -> Option<DiskStory> {
        Iso9660::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        Iso9660::pictures(self).map(DiskArt::from)
    }

    fn pictures_beside(&self, path: &str) -> ArtPairing {
        if !Iso9660::holds(self, path) {
            return ArtPairing::WholeVolume;
        }
        ArtPairing::Beside(Iso9660::pictures_beside(self, path).map(DiskArt::from))
    }

    fn image_of(&self, path: &str) -> Option<DiskImage> {
        // The Finder metadata Apple's ISO 9660 extension carries, read through
        // the same rule the HFS catalog's is (SQ-0871).
        Iso9660::machine_of(self, path)
    }
}

/// The row whose sniff claims `raw`, if any. The single point at which bytes
/// become a format — [`DiskImage::detect`] and [`MountedDisk::mount`] both go
/// through it, which is why they cannot answer differently.
fn format_of(raw: &[u8]) -> Option<&'static Format> {
    FORMATS.iter().find(|f| (f.looks_like)(raw))
}

/// [`Volume::mount`] with the concrete type erased, so one `fn` pointer type
/// serves every row.
fn mount_boxed<V: Volume + Sized + 'static>(raw: Vec<u8>) -> Option<Box<dyn Volume>> {
    V::mount(raw).map(|v| Box::new(v) as Box<dyn Volume>)
}

// ── What a mounted disk holds ─────────────────────────────────────────────────

/// A story found on a volume, with the name it was stored under.
///
/// The name is for a listing, never for identification: it is `Story.data` on
/// five of nine Amiga floppies and `STORY.DAT` on every Atari ST compilation.
/// What makes this a story is that its bytes are one — see
/// [`crate::adf::looks_like_story`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskStory {
    /// How the volume spells this file: the stored filename, prefixed by its
    /// directory on a format that has them (`HITCHHIK/STORY.DAT`). Still not an
    /// identifier — but on an ST compilation the directory is the only thing
    /// that tells four files called `STORY.DAT` apart, so it is carried.
    pub name: String,
    /// The story image, byte-exact off the disk.
    pub bytes: Vec<u8>,
}

impl From<(String, Vec<u8>)> for DiskStory {
    fn from((name, bytes): (String, Vec<u8>)) -> DiskStory {
        DiskStory { name, bytes }
    }
}

/// The native Infocom picture archive a release disk carries, with its stored
/// name. The story and the art came off the same floppy, so the pairing is
/// guaranteed by the medium and needs no configuration.
#[derive(Debug)]
pub struct DiskArt {
    /// The stored filename, e.g. `Pic.data` on an Amiga disk, `CPic.data` on a
    /// Macintosh one.
    pub name: String,
    /// The parsed archive.
    pub pictures: InfocomPics,
}

impl From<(String, InfocomPics)> for DiskArt {
    fn from((name, pictures): (String, InfocomPics)) -> DiskArt {
        DiskArt { name, pictures }
    }
}

/// The machine a file's **Finder metadata** says it was made for, or `None`
/// when the metadata says nothing this crate recognises (SQ-0876, SQ-0871).
///
/// One copy, called by every reader whose format carries a Finder type and
/// creator — [`crate::hfs`] reads them out of the catalog record, and
/// [`crate::iso9660`] out of an Apple `AA` System Use entry. The two formats
/// spell the metadata differently and mean exactly the same thing by it, so the
/// RULE lives here with the rest of the machine vocabulary rather than twice in
/// the readers.
///
/// **Measured on the corpus, and stated as narrowly as it was measured.** Three
/// compilation discs carry both machines' builds side by side:
///
/// | disc | Macintosh side | DOS side |
/// |---|---|---|
/// | Masterpieces (HFS) | `IN**` + `APPL`/`INdf` | `mdos` |
/// | Lost Treasures 1 (ISO) | `IN**` + `APPL`/`INdf` | `mdos`, and blank |
/// | Lost Treasures 2 (ISO) | `IN**` + `APPL`/`INdf` | `PCXT`, `????` |
///
/// So the **Macintosh** test is the one that generalises: Infocom stamped its
/// own creator on its own Macintosh releases, uniformly, and no DOS file on any
/// of the three wears one. The DOS test does not generalise — `mdos` covers
/// Masterpieces and only part of Lost Treasures 1, and disc 2 uses a creator of
/// its authoring tool's choosing — so it is a list of what was actually seen and
/// will grow when a disc shows something new.
///
/// Both fail SAFE: an unrecognised pair is `None`, and the caller falls back to
/// whatever the VOLUME implies, which is what every medium did before this
/// existed.
///
/// It is a rule about the metadata, never about the bytes — what a file IS
/// remains [`crate::adf::looks_like_story`]'s and
/// [`crate::infocom_pics::InfocomPics::parse`]'s answer, asked separately.
pub fn machine_from_finder(file_type: &[u8; 4], creator: &[u8; 4]) -> Option<DiskImage> {
    /// Creators seen on DOS files sitting on a Macintosh-readable volume:
    /// Apple's PC Exchange stamps `mdos` and `dosa` on an import, and Lost
    /// Treasures II's authoring tool stamped `PCXT`.
    const DOS_CREATORS: [&[u8; 4]; 3] = [b"mdos", b"dosa", b"PCXT"];
    /// A Macintosh Infocom release is one of exactly two Finder types: the
    /// game as a double-clickable application, or its data file.
    const INFOCOM_TYPES: [&[u8; 4]; 2] = [b"APPL", b"INdf"];

    if DOS_CREATORS.contains(&creator) {
        return Some(DiskImage::Fat12Dos);
    }
    if creator.starts_with(b"IN") && INFOCOM_TYPES.contains(&file_type) {
        return Some(DiskImage::Hfs);
    }
    None
}

/// How good a rendition is, as a sort key — lower is better (SQ-0880).
///
/// One copy, used by every reader that has to pick one archive out of several,
/// so the Macintosh disc and the two CD-ROMs cannot rank them differently.
///
/// **Colour beats monochrome**, which is the rule this codebase already had:
/// two colours is a 1989 hardware constraint rather than an authorial choice,
/// and handing a terminal with sixteen million of them a two-colour Zork Zero
/// would need a reason nothing on the disk gives.
///
/// **Then MCGA beats EGA**, which is new and is the first half of a question
/// that was deliberately left open. [`DiskImage::Fat12Dos`]'s row said no video
/// card is preferred, on the ground that no release put two colour renditions
/// where one choice had to be made — "a rule with no example". *The Lost
/// Treasures of Infocom II* is the example: `ARTHUR.MG1` and `ARTHUR.EG1` sit
/// in ONE folder, for three games, so there is no disk order left to defer to.
///
/// The count cannot settle it and must not be asked to. Once an EGA set is
/// merged with its continuation the two hold the same pictures — Arthur 171
/// against 171, Journey 135 against 134, Shogun 50 against 48 — so ranking on
/// count is ranking on noise, and it split those three games two ways.
///
/// MCGA wins on the same argument colour wins on: 320x200 in 256 colours
/// against 640x200 in 16. The horizontal resolution EGA trades for it is the
/// lesser thing, and the pictures are the same pictures either way.
///
/// It is a DEFAULT and only a default. Every rendition a release carries is
/// listed in the launch dialog and reachable by name, which is where someone
/// who wants the EGA plates says so.
pub fn art_preference(pics: &InfocomPics) -> (bool, bool) {
    let wide_pc =
        pics.flavour() == crate::infocom_pics::Flavour::Pc && pics.picture_space_width() != 320;
    (pics.is_monochrome(), wide_pc)
}

/// What a volume can say about which artwork pairs with ONE story on it
/// (SQ-0876).
///
/// The tri-state exists because "no artwork beside this story" and "this volume
/// cannot be more specific than the whole disk" are different answers that a
/// bare `Option` would spell the same way — and collapsing them is exactly the
/// bug: a Macintosh *Zork I*, which never had artwork, would be handed the
/// archive of whichever graphical game happened to sit elsewhere on the disc.
#[derive(Debug)]
pub enum ArtPairing {
    /// This volume has nothing finer to say — ask [`MountedDisk::pictures`].
    /// Every format but HFS answers this, and so does HFS for a story it does
    /// not hold.
    WholeVolume,
    /// The archive stored beside this story, or `None` when it genuinely has
    /// none. **`Beside(None)` is a decision**, and the caller must not fall
    /// back past it.
    Beside(Option<DiskArt>),
}

/// Why a disk would not open. **Format-neutral on purpose**: a caller reports
/// that a disk did not mount, never that an HFS catalog B*-tree was short — the
/// front-ends do not know what a catalog is, and must not have to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountError {
    /// Not a disk image in any format this crate reads.
    NotADiskImage,
    /// Recognised as `.0`, but its filesystem would not read.
    Unreadable(DiskImage),
}

impl std::fmt::Display for MountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MountError::NotADiskImage => f.write_str("not a disk image in any format we read"),
            MountError::Unreadable(image) => {
                write!(f, "the {} filesystem on it would not read", image.label())
            }
        }
    }
}

// ── The standard API a format implements ──────────────────────────────────────

/// **The standard disk-image API.** One reader per format implements it; every
/// caller in the workspace consumes it through [`MountedDisk`], never directly.
///
/// The required surface is deliberately the raw thing a filesystem reader gives
/// — recognise, open, name yourself, list your files, and the format's own
/// answer for which story and which archive. [`Volume::stories`] is provided
/// once here, on top of `contents`, because "a story is a file whose bytes are a
/// story" is a policy no format gets to restate.
///
/// A new format's impl is pure delegation to its reader; see the two below.
pub trait Volume: std::fmt::Debug {
    /// Cheap content sniff: could `raw` be this format? Must never claim bytes
    /// [`Volume::mount`] would then refuse, since [`DiskImage::detect`] is
    /// exactly this question asked of the whole table.
    fn looks_like(raw: &[u8]) -> bool
    where
        Self: Sized;

    /// Open the image and enumerate it. `None` when its filesystem will not
    /// read; the format-neutral [`MountError`] is added by the caller.
    fn mount(raw: Vec<u8>) -> Option<Self>
    where
        Self: Sized;

    /// The volume's own name, where the format keeps one (HFS does; AmigaDOS's
    /// is not exposed by its reader).
    fn volume_name(&self) -> Option<&str>;

    /// How many files the mount found — what an error message means by "is this
    /// the boot disk?".
    fn file_count(&self) -> usize;

    /// Every file that reads, in disk order, as `(name, bytes)`.
    fn contents(&self) -> Vec<(String, Vec<u8>)>;

    /// One file by the name the user typed, or `None` when the volume has no
    /// such file.
    ///
    /// **The matching rule is the format's own and is not restated here.** It is
    /// what the user's `--pictures Pic.data` has to hit, so every reader
    /// delegates straight to its own `read_named` rather than being normalised
    /// into a shared one — AmigaDOS, HFS and FAT12 do not spell names alike, and
    /// a seam that "helpfully" agreed on a rule would break the door SQ-0838
    /// added (it is the only way to reach the Macintosh's two-colour archive).
    fn read_named(&self, name: &str) -> Option<Vec<u8>>;

    /// The story to open, by the format's own tiebreak when a disk offers more
    /// than one.
    fn story(&self) -> Option<DiskStory>;

    /// The native picture archive, if the disk carries a readable one.
    fn pictures(&self) -> Option<DiskArt>;

    /// The artwork paired with ONE story on this volume, when the volume can
    /// pair more precisely than "the archive on this disk" (SQ-0876).
    ///
    /// Provided, and the default is [`ArtPairing::WholeVolume`] — "I cannot be
    /// more specific" — so a format that adopts nothing here behaves exactly as
    /// it did. Only a volume that keeps its games in FOLDERS has anything finer
    /// to say, and on this corpus that is HFS: the Masterpieces CD holds six
    /// graphical games in six folders and one flat `pictures()` for all of them.
    ///
    /// `path` is the story's own [`DiskStory::name`], as this volume spells it.
    fn pictures_beside(&self, _path: &str) -> ArtPairing {
        ArtPairing::WholeVolume
    }

    /// The machine ONE file on this volume was pressed for, when the volume
    /// records that per file — a hybrid disc carries both (SQ-0876).
    ///
    /// `None` means "the volume's own machine", which is every format, every
    /// file, but one: a DOS build sitting on the Macintosh half of a hybrid CD,
    /// which the Finder metadata marks as an import.
    fn image_of(&self, _path: &str) -> Option<DiskImage> {
        None
    }

    /// **Every** story on the volume, in disk order — what a picker lists when a
    /// compilation disk holds four games and an InvisiClues file.
    ///
    /// Provided, not required: identifying a story by its bytes is this crate's
    /// policy and there is one copy of it.
    ///
    /// **A story need not be a file.** *Arthur* and *Journey* page theirs out of
    /// a packed Apple volume spread over the disk's `.D1`…`.D5` segments, none
    /// of which is a story on its own, so no per-file test can find it. That
    /// container is asked for here rather than in one format's impl because it
    /// is not a property of any filesystem — the same index addresses the raw
    /// 5.25-inch pressings, which have no filesystem at all (SQ-0852).
    fn stories(&self) -> Vec<DiskStory> {
        let contents = self.contents();
        let mut found: Vec<DiskStory> = contents
            .iter()
            .filter(|(_, bytes)| looks_like_story(bytes))
            .cloned()
            .map(DiskStory::from)
            .collect();
        found.extend(crate::infocom_packed::story(&contents).map(DiskStory::from));
        found
    }
}

impl Volume for Adf {
    fn looks_like(raw: &[u8]) -> bool {
        Adf::looks_like_adf(raw)
    }

    fn mount(raw: Vec<u8>) -> Option<Adf> {
        Adf::mount(raw).ok()
    }

    fn volume_name(&self) -> Option<&str> {
        // AmigaDOS names its root block, but `Adf` does not expose it, and the
        // seam does not invent what a reader does not report.
        None
    }

    fn file_count(&self) -> usize {
        self.files().len()
    }

    fn contents(&self) -> Vec<(String, Vec<u8>)> {
        // The PATH, not the bare name (SQ-0908): an Amiga release disk stores every
        // game under Infocom's one conventional filename, so `Story.Data` appears once
        // per game directory and a caller handed the basename got whichever the block
        // scan reached first. `Hfs` and `Iso9660` already report full paths; this is
        // the format that did not.
        self.files().iter().filter_map(|e| self.read(e).map(|b| (e.path.clone(), b))).collect()
    }

    fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        // Case-insensitive on the stored AmigaDOS name.
        Adf::read_named(self, name)
    }

    fn story(&self) -> Option<DiskStory> {
        Adf::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        Adf::pictures(self).map(DiskArt::from)
    }
}

impl Volume for Hfs {
    fn looks_like(raw: &[u8]) -> bool {
        Hfs::looks_like_hfs(raw)
    }

    fn mount(raw: Vec<u8>) -> Option<Hfs> {
        Hfs::mount(raw).ok()
    }

    fn volume_name(&self) -> Option<&str> {
        // An unnamed volume is `None`, not `Some("")` — a caller that splices
        // the name into a sentence must not be handed a hole to splice.
        Some(Hfs::volume_name(self)).filter(|n| !n.is_empty())
    }

    fn file_count(&self) -> usize {
        self.files().len()
    }

    fn contents(&self) -> Vec<(String, Vec<u8>)> {
        // The PATH, not the bare name: three files on the Masterpieces CD are
        // called `STORY.DATA` and the folder is the only thing between them
        // (SQ-0877). Same reason `Fat12` reports `HITCHHIK/STORY.DAT`.
        self.files().iter().filter_map(|e| self.read(e).map(|b| (e.path(), b))).collect()
    }

    fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        // Case-insensitive on the stored path, then on the bare catalog name.
        Hfs::read_named(self, name)
    }

    fn story(&self) -> Option<DiskStory> {
        Hfs::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        Hfs::pictures(self).map(DiskArt::from)
    }

    /// A story this volume holds gets the archive beside it, and a story it does
    /// not hold gets no opinion at all — which is what keeps a caller passing a
    /// name from somewhere else falling through to the volume-wide answer rather
    /// than being told "no artwork".
    fn pictures_beside(&self, path: &str) -> ArtPairing {
        if Hfs::is_from_dos(self, path).is_none() {
            return ArtPairing::WholeVolume;
        }
        ArtPairing::Beside(Hfs::pictures_beside(self, path).map(DiskArt::from))
    }

    fn image_of(&self, path: &str) -> Option<DiskImage> {
        // A DOS build on the Macintosh half of a hybrid disc is a DOS build:
        // it wears the DOS row, so it answers the DOS row's interpreter number
        // (`None` — the IBM PC's rule is version-dependent) and calls itself
        // "DOS" in a listing, instead of claiming the Macintosh the FILESYSTEM
        // implies. See `hfs::HfsEntry::is_from_dos`.
        Hfs::is_from_dos(self, path)?.then_some(DiskImage::Fat12Dos)
    }
}

/// One impl for both FAT12 rows. The rows differ in their sniff — which machine
/// pressed the disk — and in nothing else, because the filesystem is the same
/// filesystem.
impl Volume for Fat12 {
    /// The FILESYSTEM sniff, deliberately machine-neutral. The table does not
    /// use this one: [`FORMATS`] holds `fat12::looks_like_dos` and
    /// `fat12::looks_like_atari_st`, which are this question and then the
    /// machine question, so the two rows stay disjoint.
    fn looks_like(raw: &[u8]) -> bool {
        Fat12::looks_like_fat12(raw)
    }

    fn mount(raw: Vec<u8>) -> Option<Fat12> {
        Fat12::mount(raw).ok()
    }

    fn volume_name(&self) -> Option<&str> {
        // The DOS release disks label their volumes (`Tresure 1`, `DISK 1`,
        // `ZORK0 1`); no ST compilation does, and that reads as `None` rather
        // than as an empty name spliced into somebody's sentence.
        Fat12::volume_label(self)
    }

    fn file_count(&self) -> usize {
        self.files().len()
    }

    fn contents(&self) -> Vec<(String, Vec<u8>)> {
        // Named by PATH, not by filename: four of the games on `Infocom
        // Compilation 9` are called `STORY.DAT` and only the directory tells
        // them apart.
        self.files().iter().filter_map(|e| self.read(e).map(|b| (e.path(), b))).collect()
    }

    fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        // Case-insensitive on either the 8.3 filename or the full path — this
        // is the one format with directories, and `contents` names its files by
        // path, so the name a caller was SHOWN has to be a name it can ask for.
        Fat12::read_named(self, name)
    }

    fn story(&self) -> Option<DiskStory> {
        Fat12::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        Fat12::pictures(self).map(DiskArt::from)
    }
}

impl Volume for ProDos {
    fn looks_like(raw: &[u8]) -> bool {
        ProDos::looks_like_prodos(raw)
    }

    fn mount(raw: Vec<u8>) -> Option<ProDos> {
        ProDos::mount(raw).ok()
    }

    fn volume_name(&self) -> Option<&str> {
        // Every ProDOS volume is named — the format has no unnamed one — but an
        // empty name is `None` rather than a hole spliced into somebody's
        // sentence, exactly as the HFS impl above.
        Some(ProDos::volume_name(self)).filter(|n| !n.is_empty())
    }

    fn file_count(&self) -> usize {
        self.files().len()
    }

    fn contents(&self) -> Vec<(String, Vec<u8>)> {
        // Named by PATH, like FAT12: ProDOS nests, and the GS/OS disks put most
        // of what they carry under `SYSTEM/`.
        self.files().iter().filter_map(|e| self.read(e).map(|b| (e.path(), b))).collect()
    }

    fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        // Case-insensitive on either the stored name or the full path.
        ProDos::read_named(self, name)
    }

    fn story(&self) -> Option<DiskStory> {
        ProDos::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        ProDos::pictures(self).map(DiskArt::from)
    }
}

/// **The one impl here that is not a filesystem reader** (SQ-0868). A raw
/// self-booting disk has no volume, so most of this trait's questions have a
/// short answer — but they all have one, which is the point of the trait: the
/// front-ends ask a disk what it holds and never ask what kind of disk it is.
impl Volume for InfocomBoot {
    fn looks_like(raw: &[u8]) -> bool {
        InfocomBoot::looks_like_boot_disk(raw)
    }

    fn mount(raw: Vec<u8>) -> Option<InfocomBoot> {
        InfocomBoot::mount(raw).ok()
    }

    fn volume_name(&self) -> Option<&str> {
        // There is no volume, so there is no volume name. `None` and not an
        // invented one — the same rule the AmigaDOS impl above follows for a name
        // its reader does not report.
        None
    }

    fn file_count(&self) -> usize {
        InfocomBoot::file_count(self)
    }

    fn contents(&self) -> Vec<(String, Vec<u8>)> {
        InfocomBoot::contents(self)
    }

    fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        // Case-insensitive on the only name this medium has: where the story is.
        InfocomBoot::read_named(self, name)
    }

    fn story(&self) -> Option<DiskStory> {
        InfocomBoot::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        InfocomBoot::pictures(self).map(DiskArt::from)
    }
}

/// **The second impl here that is not a filesystem reader, and the first whose
/// disk may hold only part of a game** (SQ-0869). See [`crate::d64`].
impl Volume for D64 {
    fn looks_like(raw: &[u8]) -> bool {
        D64::looks_like_d64(raw)
    }

    fn mount(raw: Vec<u8>) -> Option<D64> {
        D64::mount(raw).ok()
    }

    fn volume_name(&self) -> Option<&str> {
        // Commodore DOS names every disk, and these three use it for something
        // a person would recognise: `TRINITY`, `SIDE 2`, `HITCHHIKER GUIDE`.
        D64::volume_name(self)
    }

    fn file_count(&self) -> usize {
        D64::file_count(self)
    }

    fn contents(&self) -> Vec<(String, Vec<u8>)> {
        D64::contents(self)
    }

    fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        // Case-insensitive on either name this medium has: the disk's own, and
        // where the story is.
        D64::read_named(self, name)
    }

    fn story(&self) -> Option<DiskStory> {
        D64::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        D64::pictures(self).map(DiskArt::from)
    }

    /// **Overridden, uniquely here**, because the provided implementation would
    /// be asking the wrong question of the wrong bytes.
    ///
    /// [`Volume::stories`] identifies a story by testing each of `contents()`
    /// with [`looks_like_story`], and the RAW-SECTOR entry in this format's
    /// `contents()` is one this reader has already reassembled and checked
    /// against the story's **own header checksum** — far stronger than the
    /// structural test, and not to be re-decided by it.
    ///
    /// **Since SQ-1458 `contents()` also carries real CBM files**, so the
    /// override now *adds* the verified story to the provided answer instead of
    /// replacing it. That keeps this engine-neutral, which is the property that
    /// matters: a Commodore disk keeping a Z-machine story, a Glulx image or a
    /// Blorb in its filesystem is found by [`looks_like_story`] like any other
    /// volume's file. Nothing in the corpus does — *Hitchhiker's* one CBM file
    /// is a 552-byte BASIC loader — and a **Scott Adams `PRG` is deliberately
    /// not found here either**: what makes those bytes a game is not a fact this
    /// crate knows, so they are offered through `contents()` and classified by
    /// the caller.
    fn stories(&self) -> Vec<DiskStory> {
        commodore_stories(&self.contents(), Volume::story(self))
    }
}

/// The Commodore override both impls above share: the ordinary rule over the
/// volume's files, plus the checksum-verified raw-sector story if it has one.
///
/// One function rather than two bodies, because a `.g64` IS a `.d64` once
/// decoded and two copies of this would be exactly the hand-maintained
/// cross-file invariant the refactoring policy names.
fn commodore_stories(
    contents: &[(String, Vec<u8>)],
    verified: Option<DiskStory>,
) -> Vec<DiskStory> {
    let mut found: Vec<DiskStory> = contents
        .iter()
        .filter(|(_, bytes)| looks_like_story(bytes))
        .cloned()
        .map(DiskStory::from)
        .collect();
    if let Some(story) = verified {
        if !found.iter().any(|f| f.name == story.name) {
            found.push(story);
        }
    }
    found
}

/// **The same disk as the impl above, arriving as a bitstream** (SQ-1095).
///
/// Every answer is [`D64`]'s, because a mounted [`G64`] *is* a mounted `D64`:
/// the decode happens once, in [`G64::mount`], and what comes out of it has no
/// memory of having been GCR. So there is no second story-finding path to drift
/// away from the first, and the `stories` override below is the same override
/// for the same reason. See [`crate::g64`].
impl Volume for G64 {
    fn looks_like(raw: &[u8]) -> bool {
        G64::looks_like_g64(raw)
    }

    fn mount(raw: Vec<u8>) -> Option<G64> {
        G64::mount(&raw).ok()
    }

    fn volume_name(&self) -> Option<&str> {
        G64::volume_name(self)
    }

    fn file_count(&self) -> usize {
        G64::file_count(self)
    }

    fn contents(&self) -> Vec<(String, Vec<u8>)> {
        G64::contents(self)
    }

    fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        G64::read_named(self, name)
    }

    fn story(&self) -> Option<DiskStory> {
        G64::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        G64::pictures(self).map(DiskArt::from)
    }

    /// Overridden for the reason the impl above states, and by the same route:
    /// the raw-sector entry is a story found by checksum rather than a listing
    /// to run a heuristic over, and any CBM file beside it is an ordinary file
    /// and goes through the ordinary rule.
    fn stories(&self) -> Vec<DiskStory> {
        commodore_stories(&self.contents(), Volume::story(self))
    }
}

/// **An Atari 8-bit floppy, whose filesystem may or may not still be there**
/// (SQ-1458). See [`crate::atr`].
impl Volume for Atr {
    fn looks_like(raw: &[u8]) -> bool {
        Atr::looks_like_atr(raw)
    }

    fn mount(raw: Vec<u8>) -> Option<Atr> {
        Atr::mount(raw).ok()
    }

    fn volume_name(&self) -> Option<&str> {
        // Atari DOS 2 keeps no volume name — only a sector count and a bitmap —
        // so there is none to report, and none is invented. The same rule the
        // AmigaDOS impl above follows.
        None
    }

    fn file_count(&self) -> usize {
        self.files().len()
    }

    fn contents(&self) -> Vec<(String, Vec<u8>)> {
        Atr::contents(self)
    }

    fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        // Case-insensitive on the stored 8.3 name — **and on the one reserved
        // name that is not a file**, `crate::atr::IMAGE_ENTRY`, which is how the
        // S.A.G.A. database is reached at all. It is deliberately not listed by
        // `contents`; see that constant.
        Atr::read_named(self, name)
    }

    fn story(&self) -> Option<DiskStory> {
        Atr::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        Atr::pictures(self).map(DiskArt::from)
    }
}

/// **An Apple II DOS 3.3 volume** (SQ-1458). See [`crate::dos33`].
impl Volume for Dos33 {
    fn looks_like(raw: &[u8]) -> bool {
        Dos33::looks_like_dos33(raw)
    }

    fn mount(raw: Vec<u8>) -> Option<Dos33> {
        Dos33::mount(raw).ok()
    }

    fn volume_name(&self) -> Option<&str> {
        // DOS 3.3 has a volume NUMBER and no name. A number is not something to
        // splice into "the disk called …", so this reports nothing rather than
        // inventing a transliteration — `Dos33::volume_number` is where it lives.
        None
    }

    fn file_count(&self) -> usize {
        self.files().len()
    }

    fn contents(&self) -> Vec<(String, Vec<u8>)> {
        Dos33::contents(self)
    }

    fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        // Case-insensitive on the normalised catalogue name. A binary file comes
        // back with its four-byte load prologue in front, exactly as DOS stores
        // it; `Dos33::read_binary` is the door to the stripped form, and the
        // module docs say which a caller wants.
        Dos33::read_named(self, name)
    }

    fn story(&self) -> Option<DiskStory> {
        Dos33::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        Dos33::pictures(self).map(DiskArt::from)
    }
}

/// **The one impl here whose bytes are not a disk of any kind** (SQ-1458): an
/// Atari binary-load file, whose single entry is the memory it loads. See
/// [`crate::xex`].
impl Volume for Xex {
    fn looks_like(raw: &[u8]) -> bool {
        Xex::looks_like_xex(raw)
    }

    fn mount(raw: Vec<u8>) -> Option<Xex> {
        Xex::parse(&raw).ok()
    }

    fn volume_name(&self) -> Option<&str> {
        // There is no volume, so there is no volume name — the same short answer
        // `InfocomBoot` gives, and for the same reason.
        None
    }

    fn file_count(&self) -> usize {
        1
    }

    fn contents(&self) -> Vec<(String, Vec<u8>)> {
        Xex::contents(self)
    }

    fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        // Case-insensitive on the only name this medium has: the range it loads.
        Xex::read_named(self, name)
    }

    fn story(&self) -> Option<DiskStory> {
        Xex::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        Xex::pictures(self).map(DiskArt::from)
    }
}

// ── A mounted disk ────────────────────────────────────────────────────────────

/// An open release disk, whatever format it turned out to be.
///
/// This is what every front-end holds. It answers in one vocabulary, so no
/// caller has an `if adf … else if hfs` in it and none can acquire one: the
/// concrete reader is behind a [`Volume`] and its name never leaves this file.
#[derive(Debug)]
pub struct MountedDisk {
    image: DiskImage,
    volume: Box<dyn Volume>,
    /// The files on the OTHER volumes of this image's multi-disk release, when
    /// there are any and this one had no story of its own. Empty for every
    /// ordinary mount, which is nearly all of them; see
    /// [`MountedDisk::mount_set`].
    across: Vec<(String, Vec<u8>)>,
    /// The set's volumes as whole IMAGES, this one first — for the container
    /// whose segments are not files (SQ-0869).
    ///
    /// The Apple II's packed volume pages a story across `.D1`…`.D5`, which are
    /// ordinary files and arrive in `across` above. A Commodore release pages one
    /// across raw sectors, so what its assembler needs is the sides themselves;
    /// a `D64` side of a two-disk game lists no files at all, and a listing is
    /// exactly what it cannot be asked for. Empty for every ordinary mount, on
    /// the same condition as `across` — and now also empty for a format whose
    /// assembler reads files instead, which is why `volumes` below exists.
    sides: Vec<Vec<u8>>,
    /// How many volumes this release turned out to have, this one included.
    ///
    /// `sides.len()` used to answer this, which silently required EVERY format
    /// to keep whole images just so the packed-Apple path could count them
    /// (SQ-0875). One is an ordinary mount.
    volumes: usize,
}

impl MountedDisk {
    /// Open `raw` as whichever format claims it.
    ///
    /// [`MountError::NotADiskImage`] is the ordinary "this is a plain story
    /// file" answer and callers fall through on it; [`MountError::Unreadable`]
    /// means a disk we recognised is damaged, which is worth reporting.
    pub fn mount(raw: Vec<u8>) -> Result<MountedDisk, MountError> {
        MountedDisk::mount_set(raw, Vec::new)
    }

    /// Open `raw`, with the other volumes of its multi-disk release available to
    /// it — the mount a story that is on **no single disk** needs (SQ-0864).
    ///
    /// `companions` yields those other images, in disk order and without `raw`
    /// itself. It is a closure and not a list because reading seven 800 KB
    /// floppies to open one of them would be an absurd price for a library scan
    /// to pay per row: **it is called only when the named volume turns out to
    /// have no story of its own**, which is exactly the case that cannot be
    /// answered without them. Every compilation disk in the corpus answers for
    /// itself and never asks.
    ///
    /// What the companions can add is one thing and one thing only: the story a
    /// release pages ACROSS its volumes, which is [`crate::infocom_packed`]'s
    /// container. They do not become part of this volume — [`MountedDisk::contents`],
    /// [`MountedDisk::read_named`], [`MountedDisk::file_count`] and
    /// [`MountedDisk::volume_name`] all still describe the disk that was named,
    /// because that is the disk the caller has. Ordinary files on a sibling
    /// floppy are that floppy's business and it can be mounted.
    ///
    /// Format-neutral by construction: the companions are opened through the
    /// **same row** that claimed `raw`, and one that the row declines is
    /// dropped. No format implements anything for this and none can opt out.
    pub fn mount_set(
        raw: Vec<u8>,
        companions: impl FnOnce() -> Vec<Vec<u8>>,
    ) -> Result<MountedDisk, MountError> {
        let format = format_of(&raw).ok_or(MountError::NotADiskImage)?;
        // The mount consumes `raw`, and whether this volume has a story of its
        // own is only knowable after it. So a format whose assembler needs whole
        // images must be copied FIRST — but only that format: cloning
        // unconditionally made every ordinary mount pay for a copy it dropped,
        // which is 354 MB on a hybrid CD and 12 MB on a Macintosh volume
        // (SQ-0875). The row says which formats care.
        let mine = format.pages_across_images.then(|| raw.clone());
        let volume = (format.mount)(raw).ok_or(MountError::Unreadable(format.image))?;
        let (across, sides, volumes) = if volume.stories().is_empty() {
            let others: Vec<Vec<u8>> =
                companions().into_iter().filter(|raw| (format.looks_like)(raw)).collect();
            let across = others
                .iter()
                .filter_map(|raw| (format.mount)(raw.clone()))
                .flat_map(|v| v.contents())
                .collect();
            let volumes = 1 + others.len();
            // `mine` is `Some` exactly when the row said so, so this keeps the
            // Commodore's images and drops everyone else's.
            let sides = match mine {
                Some(mine) => {
                    let mut sides = vec![mine];
                    sides.extend(others);
                    sides
                }
                None => Vec::new(),
            };
            (across, sides, volumes)
        } else {
            (Vec::new(), Vec::new(), 1)
        };
        Ok(MountedDisk { image: format.image, volume, across, sides, volumes })
    }

    /// The story this release keeps across its volumes, reassembled out of all
    /// of them — or `None` when no companions were offered, or they hold no such
    /// container, or the one they hold is incomplete.
    ///
    /// The named volume's files come FIRST, so the container's own
    /// "which segment carries the index" search (and therefore the name it
    /// reports) does not depend on which floppy of the set a person happened to
    /// open. Reassembly is verified against the story's own header checksum by
    /// [`crate::infocom_packed`], so a set that does not belong together is
    /// refused rather than handed over as plausible-looking Z-code.
    fn story_across_the_set(&self) -> Option<DiskStory> {
        // `volumes` rather than `across.len()`, because a set whose members list
        // no files at all is exactly the Commodore case and has an empty
        // `across` — and rather than `sides.len()`, which is now empty for the
        // formats whose assembler reads files (SQ-0875).
        if self.volumes < 2 {
            return None;
        }
        let mut all = self.volume.contents();
        all.extend(self.across.iter().cloned());
        // Two containers, asked in turn, and both are verified against the
        // story's own header checksum so neither can answer with plausible
        // rubbish. The Apple II's packed volume pages a story across `.D1`…`.D5`
        // segments that ARE files; the Commodore's pages one across raw sector
        // images that are not (SQ-0869). A third would be a third line here, and
        // that is the whole of what "a story that is on no single disk" costs a
        // format — nothing, unless it has one.
        crate::infocom_packed::story(&all)
            .or_else(|| crate::d64::story_across(&self.sides))
            .map(DiskStory::from)
    }

    /// Which format this turned out to be.
    pub fn format(&self) -> DiskImage {
        self.image
    }

    /// See [`DiskImage::label`].
    pub fn label(&self) -> &'static str {
        self.image.label()
    }

    /// See [`DiskImage::interpreter_number`].
    pub fn interpreter_number(&self) -> Option<u8> {
        self.image.interpreter_number()
    }

    /// The volume's own name, where the format keeps one.
    pub fn volume_name(&self) -> Option<&str> {
        self.volume.volume_name()
    }

    /// How many files the mount found.
    pub fn file_count(&self) -> usize {
        self.volume.file_count()
    }

    /// Every file that reads, in disk order, as `(name, bytes)`.
    ///
    /// The unfiltered listing, for a caller that identifies files by its own
    /// test rather than by this crate's. `app`'s asset discovery needs it
    /// because [`MountedDisk::pictures`] answers with THE archive by the
    /// format's own tiebreak, and a Macintosh disk carries two — a colour
    /// `CPic.data` and a monochrome `Pic.data` — both of which a person must be
    /// able to choose between (SQ-0843).
    pub fn contents(&self) -> Vec<(String, Vec<u8>)> {
        self.volume.contents()
    }

    /// Every story on the disk, in disk order — plus the one its release pages
    /// across its volumes, when [`MountedDisk::mount_set`] was given them.
    ///
    /// Naming any floppy of a set therefore lists the same one game, which is
    /// what makes a set behave like a shelf rather than like five refusals. The
    /// duplicate rows that produces across a set are the browser's to fold, and
    /// it already does (`app::picker::dedupe_within_sets`, SQ-0844).
    pub fn stories(&self) -> Vec<DiskStory> {
        let mut found = self.volume.stories();
        if let Some(story) = self.story_across_the_set() {
            if !found.iter().any(|f| f.name == story.name) {
                found.push(story);
            }
        }
        found
    }

    /// One file off the volume by the name the user typed — the `--pictures`
    /// door (SQ-0838), and the other half of [`MountedDisk::contents`]: what a
    /// caller was shown, it can ask for.
    ///
    /// See [`Volume::read_named`] for why the matching rule stays each format's
    /// own.
    pub fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        self.volume.read_named(name)
    }

    /// The story to open, by the format's tiebreak — falling back to the one the
    /// release pages across its volumes.
    ///
    /// The order matters and is the conservative one: a volume that carries a
    /// game answers with its own, always. Only a volume with nothing on it
    /// reaches for the set, which is precisely the Shogun floppy's case.
    pub fn story(&self) -> Option<DiskStory> {
        self.volume.story().or_else(|| self.story_across_the_set())
    }

    /// The artwork this release keeps across its volumes, merged out of all of
    /// them — or `None` when no companions were offered, or they hold no such
    /// container, or the artwork it holds is not all here.
    ///
    /// [`Self::story_across_the_set`]'s sibling, in the same shape and for the
    /// same reason (SQ-0863/SQ-0867): the named volume's files come FIRST so the
    /// container's "which segment carries the index" search does not depend on
    /// which floppy a person opened, and [`crate::infocom_packed::pictures`]
    /// refuses a partial set rather than handing back a picture space with whole
    /// rooms missing.
    ///
    /// This is why *Shogun*'s and *Journey*'s five-volume 5.25-inch presses draw
    /// and `Journey.2mg` does not, and the three of them are one rule seen from
    /// both sides: Shogun's `SGTPICOF` fields name an archive on all five
    /// segments and Journey's on four, and on the `.dsk` sets every segment is
    /// on the shelf. `Journey.2mg` is a genuinely short pressing — it declares
    /// five segments and holds four, and the missing `JOURNEY.D5` carries a
    /// quarter of the artwork — so the merge refuses it whole rather than
    /// serving a picture space with rooms missing, exactly as [`Self::story`]
    /// refuses the story the same segment would have completed.
    fn pictures_across_the_set(&self) -> Option<DiskArt> {
        if self.across.is_empty() {
            return None;
        }
        let mut all = self.volume.contents();
        all.extend(self.across.iter().cloned());
        crate::infocom_packed::pictures(&all).map(DiskArt::from)
    }

    /// The disk's own artwork, if it carries a readable archive — falling back
    /// to the artwork the release pages across its volumes.
    ///
    /// The order is [`Self::story`]'s and is conservative for the same reason: a
    /// volume that carries an archive of its own answers with it, always, and
    /// only a volume with none reaches for the set.
    pub fn pictures(&self) -> Option<DiskArt> {
        self.volume.pictures().or_else(|| self.pictures_across_the_set())
    }

    /// The artwork paired with ONE story on this release, named as
    /// [`DiskStory::name`] spells it (SQ-0876).
    ///
    /// A volume that keeps its games in folders answers for that story alone,
    /// **including when the answer is "none"** — see [`ArtPairing`]. Every other
    /// volume, and any name this one does not hold, falls through to
    /// [`Self::pictures`], so nothing that worked before moves: a single-game
    /// floppy keeps its story and its archive at the volume root, where "beside
    /// this story" and "on this disk" are the same set of files.
    ///
    /// This is what stops a compilation handing every graphical game the first
    /// archive on the platter. All six on the Masterpieces CD resolved to
    /// `MAC/ZORK ZERO/CPIC.DATA` — so opening Journey drew Zork Zero's plates,
    /// silently, and looked like artwork the whole time.
    pub fn pictures_for(&self, entry: &str) -> Option<DiskArt> {
        match self.volume.pictures_beside(entry) {
            ArtPairing::Beside(art) => art,
            ArtPairing::WholeVolume => self.pictures(),
        }
    }

    /// The medium ONE story on this release was pressed for — this disk's own
    /// format, unless the volume records that this particular file came off
    /// another machine (SQ-0876).
    ///
    /// A hybrid disc is the case: the Masterpieces CD's Macintosh partition
    /// carries Infocom's DOS builds too, and answering "HFS" for all 83 stories
    /// told every PC one to advertise itself as a Macintosh — header `$1E` = 3,
    /// which ZMSD §11.1.3 warns is exactly the byte a Version 6 story leans on.
    pub fn image_for(&self, entry: &str) -> DiskImage {
        self.volume.image_of(entry).unwrap_or(self.image)
    }

    /// [`DiskImage::interpreter_number`] for the medium ONE story came off —
    /// see [`Self::image_for`].
    pub fn interpreter_number_for(&self, entry: &str) -> Option<u8> {
        self.image_for(entry).interpreter_number()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The AmigaDOS boot block is all `looks_like_adf` needs, and it is what
    /// every `.adf` in the corpus opens with.
    fn adf_bytes() -> Vec<u8> {
        let mut v = vec![0u8; 4 * 512];
        v[0..3].copy_from_slice(b"DOS");
        v[3] = 0; // OFS
        v
    }

    #[test]
    fn an_amiga_floppy_defaults_to_interpreter_four() {
        // ZMSD §11.1.3: 4 = Amiga. The whole of SQ-0839's rule in two lines.
        assert_eq!(DiskImage::detect(&adf_bytes()), Some(DiskImage::Adf));
        assert_eq!(DiskImage::Adf.interpreter_number(), Some(4));
    }

    /// SQ-0838 lifted this block: the Macintosh's number was always known, and
    /// now its machine is too, so the medium hands the number out like any
    /// other. ZMSD §11.1.3, quoted at [`MACINTOSH_INTERPRETER_NUMBER`].
    #[test]
    fn a_macintosh_disk_defaults_to_interpreter_three() {
        assert_eq!(DiskImage::Hfs.interpreter_number(), Some(3), "ZMSD §11.1.3: 3 = Macintosh");
        assert_eq!(DiskImage::Hfs.label(), "HFS");
    }

    /// SQ-0857 lifted this block too, and for the opposite reason to the
    /// Macintosh's: the Apple II's number was never a constant — Infocom's own
    /// YZIP picks between 2, 9 and 10 at boot — but declining left a ProDOS
    /// story being told it was a DECSystem-20 or an IBM PC. Argued in full at
    /// the row and at [`APPLE_IIGS_INTERPRETER_NUMBER`].
    #[test]
    fn a_prodos_volume_defaults_to_the_apple_iigs() {
        assert_eq!(
            DiskImage::ProDos.interpreter_number(),
            Some(10),
            "ZMSD §11.1.3: 10 = Apple IIgs, and `IIgsID EQU 10 ; ][gs Yzip`",
        );
        assert_eq!(DiskImage::ProDos.label(), "ProDOS");
        // …and it is the ONE number, not one of the family's three: the row has
        // to name a machine, and the other two stay reachable by asking for them.
        assert_ne!(DiskImage::ProDos.interpreter_number(), Some(2), "the IIe is not the default");
        assert_ne!(DiskImage::ProDos.interpreter_number(), Some(9), "nor is the IIc");
    }

    /// Content, not extension — and an ordinary story file is not a medium, so
    /// it never moves the number.
    #[test]
    fn an_ordinary_story_file_is_not_a_medium() {
        let mut story = vec![0u8; 0x400];
        story[0] = 3;
        story[0x12..0x18].copy_from_slice(b"840726");
        assert_eq!(DiskImage::detect(&story), None);
        assert_eq!(DiskImage::detect(&[]), None);
        assert_eq!(DiskImage::detect(b"not a disk image at all"), None);
    }

    #[test]
    fn the_label_names_the_filesystem() {
        assert_eq!(DiskImage::Adf.label(), "ADF");
    }

    // ── The seam: one mount path, every format (SQ-0840) ──────────────────────

    /// A structurally valid v6 story, so `looks_like_story` finds it and every
    /// format's sample carries the same payload.
    ///
    /// It carries a **correct header checksum**, and since SQ-0869 that is not
    /// decoration. A filesystem's reader finds a story by finding a file; a
    /// raw-sector reader has no file to find and identifies a run of sectors by
    /// checksumming it, so a sample declaring `0000` — which
    /// [`crate::infocom_packed::verified`] reads as "not recorded" and skips —
    /// would be a sample [`DiskImage::CommodoreD64`] could not honestly claim.
    fn fake_story() -> Vec<u8> {
        let mut b = vec![0u8; 4096];
        b[0] = 6;
        let mut word = |o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_be_bytes());
        word(0x04, 0x0400); // high memory
        word(0x06, 0x0500); // initial program counter
        word(0x08, 0x0300); // dictionary
        word(0x0a, 0x0100); // objects
        word(0x0c, 0x0200); // globals
        word(0x0e, 0x0280); // static memory base
        word(0x1a, (4096 / 8) as u16); // file length, v6 unit
        b[0x12..0x18].copy_from_slice(b"890323");
        // A body, so the checksum below is a number rather than zero. An
        // all-zero story sums to `0000`, which `verified` reads as "not
        // recorded" and skips — the one value that would let a raw-sector
        // reader accept any run of sectors at all.
        for (i, byte) in b.iter_mut().enumerate().skip(64) {
            *byte = (i % 251) as u8;
        }
        // Last, over `$40..` of everything above it.
        let sum = b[64..].iter().fold(0u16, |a, &x| a.wrapping_add(u16::from(x)));
        b[0x1c..0x1e].copy_from_slice(&sum.to_be_bytes());
        b
    }

    /// A real, mountable volume of `image`'s format, carrying one story and one
    /// file that is not one.
    ///
    /// **The match is exhaustive on purpose.** A new [`DiskImage`] variant does
    /// not COMPILE until it is given a sample here, and then does not PASS the
    /// property test below until [`FORMATS`] can detect and open what it
    /// returns. That is the whole guard against a format being half-wired again.
    fn sample_of(image: DiskImage) -> Vec<u8> {
        let story = fake_story();
        // `STORY.DAT` rather than the Amiga's `Story.data`, because the sample
        // has to be a name EVERY filesystem here can actually store and FAT12's
        // 8.3 directory entry would shorten the longer one. The name is not what
        // is under test; that a story is found by its bytes is.
        let files: [(&str, &[u8]); 2] = [("Readme", b"just a text file"), ("STORY.DAT", &story)];
        match image {
            DiskImage::Adf => crate::adf::tests::sample_disk(&files),
            DiskImage::Hfs => crate::hfs::tests::sample_disk(&files),
            DiskImage::Fat12Dos => {
                crate::fat12::tests::sample_disk(&files, crate::fat12::Machine::Dos)
            }
            DiskImage::Fat12AtariSt => {
                crate::fat12::tests::sample_disk(&files, crate::fat12::Machine::AtariSt)
            }
            DiskImage::ProDos => crate::prodos::tests::sample_disk(&files),
            DiskImage::Iso9660 => crate::iso9660::tests::sample_disc(&files),
            // No `files`, because there is nowhere to put them: a raw
            // self-booting disk has no directory at all. See
            // [`sample_entries`] for what the tests below do about that.
            DiskImage::InfocomBootDisk => crate::infocom_boot::tests::sample_disk(&story),
            // No `files` either, and for the same reason: a Commodore press
            // keeps its story in raw sectors outside the filesystem.
            DiskImage::CommodoreD64 => crate::d64::tests::sample_disk(&story),
            // The same disk as the row above, GCR-encoded into a bitstream —
            // which is what makes the property test below a round trip through
            // the decoder rather than a second look at a sector dump.
            DiskImage::CommodoreG64 => crate::g64::tests::sample_disk(&story),
            // Real filesystems, so they take the files like the rows above —
            // the Atari's directory is 8.3 and the Apple's is thirty high-ASCII
            // characters, and `STORY.DAT` fits both without shortening.
            DiskImage::AtariDos2 => crate::atr::tests::sample_disk(&files),
            // **The story alone**, and the reason is the filesystem's: DOS 3.3
            // records a file's length in SECTORS and nowhere records its bytes,
            // so a sixteen-byte `Readme` comes back as a 256-byte sector and
            // cannot round-trip byte-exact through any read this reader could
            // honestly offer. The story is 4,096 bytes — sixteen whole sectors —
            // and does. (A type-`B` file's four-byte prologue DOES record a byte
            // length, and `Dos33::read_binary` is that door; it is not this one,
            // because the dialect spec's mastering constants index the file WITH
            // its prologue. See `crate::dos33`.)
            DiskImage::AppleDos33 => crate::dos33::tests::sample_disk(&[("STORY.DAT", &story)]),
            // No `files`, on the ground the two rows above it state and one of
            // its own: a loadable binary has no directory, and it has no
            // sectors either — it is one segment and the memory it becomes.
            DiskImage::AtariXex => crate::xex::tests::sample_binary(&story),
        }
    }

    /// What [`sample_of`]'s disk holds: the name its format gives the story, and
    /// the other files beside it.
    ///
    /// **Exhaustive on purpose, like [`sample_of`]** — and it exists because
    /// SQ-0868 added the first format that is not a filesystem. Every row before
    /// it names files, so the tests below could simply assume a `Readme` and a
    /// `STORY.DAT`; [`DiskImage::InfocomBootDisk`] has no directory to hold
    /// either, and the honest thing is for the census to say so once rather than
    /// for a `if image != …` to appear in four tests.
    ///
    /// What is NOT weakened by this: every format is still detected, mounted,
    /// asked for its label, its interpreter number, its stories, its artwork and
    /// its listing, and every name it lists is still read back. Only the
    /// *contents of the sample* differ, because only they can.
    fn sample_entries(image: DiskImage) -> (&'static str, &'static [&'static str]) {
        match image {
            DiskImage::Adf
            | DiskImage::Hfs
            | DiskImage::Fat12Dos
            | DiskImage::Fat12AtariSt
            | DiskImage::ProDos
            | DiskImage::Iso9660 => ("STORY.DAT", &["Readme"]),
            // Where the story is, which is the only thing this medium knows
            // about it. `crate::infocom_boot::InfocomBoot::entry_name`.
            DiskImage::InfocomBootDisk => ("T3/S0", &[]),
            // The same, and the same reason — and the sample keeps its story at
            // track 3 sector 0 because that is where *Trinity* keeps its own.
            DiskImage::CommodoreD64 | DiskImage::CommodoreG64 => ("T3/S0", &[]),
            // A filesystem with a byte count per sector, so the same two names
            // as the top arm.
            DiskImage::AtariDos2 => ("STORY.DAT", &["Readme"]),
            // …and one without: see `sample_of` for why this disk carries only
            // the story.
            DiskImage::AppleDos33 => ("STORY.DAT", &[]),
            // The range the binary loads, which is the only thing this medium
            // knows about its one entry — `crate::xex::Xex::entry_name`. The
            // sample story is 4,096 bytes at `$4000`.
            DiskImage::AtariXex => ("$4000-$4FFF", &[]),
        }
    }

    /// The census: the enum and [`FORMATS`] must name the same formats, and the
    /// table must be where the label and the interpreter number come from.
    ///
    /// The match is exhaustive, so adding a variant fails to compile here and
    /// walks whoever added it straight to the row it is missing. A variant with
    /// no row is a format nothing can detect, mount or name — the half-wiring
    /// SQ-0840 was filed for, one enum away from happening again.
    #[test]
    fn every_variant_the_enum_declares_has_a_row_in_the_one_table() {
        let census = [
            DiskImage::Adf,
            DiskImage::Hfs,
            DiskImage::Fat12Dos,
            DiskImage::Fat12AtariSt,
            DiskImage::ProDos,
            DiskImage::InfocomBootDisk,
            DiskImage::CommodoreD64,
            DiskImage::CommodoreG64,
            DiskImage::Iso9660,
            DiskImage::AtariDos2,
            DiskImage::AppleDos33,
            DiskImage::AtariXex,
        ];
        for image in census {
            let (label, interpreter) = match image {
                DiskImage::Adf => ("ADF", Some(AMIGA_INTERPRETER_NUMBER)),
                DiskImage::Hfs => ("HFS", Some(MACINTOSH_INTERPRETER_NUMBER)),
                // A CD-ROM is not a machine: this one carries both, so the row
                // states none and the file decides (SQ-0871).
                DiskImage::Iso9660 => ("ISO", None),
                // One FAT12 filesystem, two machines, two different answers —
                // and the difference is the point. The IBM PC's honest number
                // is version-dependent (6 for Version 6, else 1), so no single
                // constant expresses it and its own rule is already in force.
                // The Atari ST's is a flat 5, written as such by Infocom's own
                // ST interpreters; both are argued at their rows in `FORMATS`.
                DiskImage::Fat12Dos => ("DOS", None),
                DiskImage::Fat12AtariSt => ("ST", Some(ATARI_ST_INTERPRETER_NUMBER)),
                // …and the Apple II answers like the ST rather than like DOS,
                // which is the reversal SQ-0857 argued at the row. ProDOS still
                // names the FAMILY and §11.1.3 still numbers three machines in
                // it — but declining lands a ProDOS story on 1 or 6, the
                // DECSystem-20 or the IBM PC, so `None` names the wrong machine
                // rather than no machine. 10 is the top of the family the Apple
                // YZIP will run on, and `--interpreter` still reaches the
                // other two.
                DiskImage::ProDos => ("ProDOS", Some(APPLE_IIGS_INTERPRETER_NUMBER)),
                // …and the raw self-booting press answers **the same number as
                // the ProDOS row**, because §11.1.3's question is which machine
                // the interpreter runs on and not which filesystem the disk has.
                // Two Apple II rows disagreeing would say the number is a
                // property of the disk, which is exactly what SQ-0857 disproved
                // out of Infocom's own YZIP. Argued in full at the row.
                DiskImage::InfocomBootDisk => ("Boot", Some(APPLE_IIGS_INTERPRETER_NUMBER)),
                // …and the Commodore press names a family too, like ProDOS —
                // but unlike ProDOS the two candidates are told apart ON the
                // disk, and the corpus holds one of each. The C64 press is
                // Version 3 and cannot read `$1E` at all, so the only Commodore
                // story here that reads it is on a Commodore 128 disk. Argued in
                // full at [`COMMODORE_128_INTERPRETER_NUMBER`].
                DiskImage::CommodoreD64 => ("CBM", Some(COMMODORE_128_INTERPRETER_NUMBER)),
                // …and the bitstream dump of the same floppy answers the same
                // number, because §11.1.3 asks which MACHINE the interpreter
                // runs on and a container cannot change that. It carries the
                // same LABEL for the same reason: the type a library shows is
                // the medium, and both of these are a 1541 floppy.
                DiskImage::CommodoreG64 => ("CBM", Some(COMMODORE_128_INTERPRETER_NUMBER)),
                // …and the Atari 8-bit declines for a THIRD reason, which is
                // neither of the two above: §11.1.3 numbers no such machine at
                // all. Its 5 is the Atari ST. Argued at the row.
                DiskImage::AtariDos2 => ("Atari DOS", None),
                DiskImage::AtariXex => ("XEX", None),
                // …and the Apple II's third filesystem answers the same number
                // as its other two, because §11.1.3 asks which machine the
                // interpreter runs on and three rows disagreeing would make the
                // number a property of the disk (SQ-0857).
                DiskImage::AppleDos33 => ("DOS 3.3", Some(APPLE_IIGS_INTERPRETER_NUMBER)),
            };
            assert!(DiskImage::all().any(|d| d == image), "{image:?} has no row in FORMATS");
            assert_eq!(image.label(), label, "{image:?}");
            assert_eq!(image.interpreter_number(), interpreter, "{image:?}");
        }
        assert_eq!(
            DiskImage::all().count(),
            census.len(),
            "FORMATS and DiskImage have drifted apart — every format is one row and one variant"
        );
    }

    /// The extensions census: every format offers at least one spelling, and
    /// [`image_extensions`] is exactly the union of the rows'.
    ///
    /// The "at least one" is the guard that matters. A row with no extension is
    /// a format a directory scan can never *offer* — it mounts perfectly when
    /// you name the file, and is invisible in the story list — which is the
    /// precise shape of the defect SQ-0849 was filed for, arrived at from the
    /// other side. Spelling rules (lowercase, no dot, no duplicates) are pinned
    /// too, because `has_story_ext` lowercases the candidate's extension and
    /// compares it as-is, so a row shouting `"ADF"` would silently match
    /// nothing.
    #[test]
    fn every_format_offers_at_least_one_extension_and_the_census_is_their_union() {
        let mut union: Vec<&str> = Vec::new();
        for image in DiskImage::all() {
            let exts = image.extensions();
            assert!(
                !exts.is_empty(),
                "{image:?} names no extension — a directory scan can never offer it"
            );
            for e in exts {
                assert!(
                    !e.is_empty() && !e.starts_with('.') && *e == e.to_ascii_lowercase(),
                    "{image:?}: {e:?} must be lowercase and dotless"
                );
                union.push(e);
            }
        }
        assert_eq!(
            image_extensions().collect::<Vec<_>>(),
            union,
            "the census is the rows' extensions and nothing else"
        );
        // **A spelling MAY be claimed by more than one row**, and two are. This
        // assertion used to forbid the overlap outright; what it forbids now is
        // an *unnoticed* one, because a shared spelling is a claim about the
        // corpus that wants writing down.
        //
        // `.dsk` (SQ-0868) is the Apple II 5.25-inch press, which is a ProDOS
        // volume on nine disks in `stories/` and a raw self-booting disk on the
        // tenth.
        //
        // `.iso` (SQ-0871) is a CD-ROM, and a CD-ROM is two constructions: a
        // cooked hybrid whose Apple partition map leads to an HFS volume, and a
        // plain ISO 9660 disc with Apple's extensions and no HFS anywhere. The
        // corpus holds one of each. Which one a file IS stays `looks_like`'s
        // answer over its bytes, so the shared spelling costs a scan nothing.
        let mut shared: Vec<&str> =
            union.iter().filter(|e| union.iter().filter(|o| o == e).count() > 1).copied().collect();
        shared.sort_unstable();
        shared.dedup();
        assert_eq!(shared, ["dsk", "iso"], "an extension is shared by two rows and undocumented");
        // The spellings the corpus in `stories/` actually uses, named outright
        // so a row losing one fails here rather than in the picker.
        // `dsk` joined them in SQ-0864, on the ProDOS row: the fourteen
        // 5.25-inch images in `stories/` are ProDOS volumes whose sectors are in
        // the drive's order rather than the filesystem's, and they mount through
        // the same reader one de-interleave earlier.
        // `po` joined them in SQ-0863, on the same row and by the same rule: the
        // corpus acquired four ProDOS volumes wearing it — `Arthur.po`,
        // `Journey.po` and `ZorkZero.po` bare, and a `Shogun.po` that is really
        // a DiskCopy wrapper round one. Three of them mounted then and all four
        // do since SQ-0889 taught the ProDOS reader that placement. A spelling
        // is claimed when a medium wears it.
        // `toast` joined them in SQ-1055, on the Macintosh row and by the same
        // rule: `Shogun.toast` is a bare HFS volume and the corpus now wears it.
        // `atr` and `xex` joined them in SQ-1458, by the same rule: the fifteen
        // Atari specimens in `stories/scott-dialects/atari/` wear one or the
        // other, and the fourteen Apple sides beside them wear `dsk`, which the
        // DOS 3.3 row now claims as a third.
        for want in ["adf", "image", "ima", "img", "st", "2mg", "dsk", "po", "toast", "atr", "xex"] {
            assert!(image_extensions().any(|e| e == want), "no row claims {want:?}");
        }
        // …and a spelling still does NOT get a name before a medium wears it:
        // `hdv` is the other bare-ProDOS convention, this crate can open one,
        // and nothing in the corpus is called that.
        const QUEUED: &str = "hdv";
        assert!(
            !image_extensions().any(|e| e == QUEUED),
            "{QUEUED:?} is claimed by a row, and no medium in the corpus justifies it"
        );
    }

    /// A name is a pre-filter, never evidence — stated as a test so the
    /// extensions column cannot quietly become a recogniser.
    ///
    /// Both halves of this module's header rule, on the new column: bytes that
    /// ARE a disk are one whatever they are called, and bytes that are not stay
    /// not one however suggestively they are named.
    #[test]
    fn the_extension_census_decides_nothing_about_what_bytes_are() {
        for image in DiskImage::all() {
            // A real image is recognised, and no extension was consulted to do
            // it — `detect` never sees a filename at all.
            assert_eq!(DiskImage::detect(&sample_of(image)), Some(image), "{image:?}");
        }
        // An ordinary story file is not a disk image, and would not become one
        // if it were called `zork.ima`.
        assert_eq!(DiskImage::detect(&fake_story()), None);
        // Nor is a file of nonsense that happens to carry a claimed extension.
        assert_eq!(DiskImage::detect(&vec![0u8; 720 * 1024]), None);
    }

    /// **The property this quest exists for**: whatever `detect` claims, `mount`
    /// opens, and the mounted disk answers every question a front-end asks —
    /// for every format alike, with no caller naming one.
    ///
    /// This is what `zvm-cli`'s old `looks_like_image` had to guard by hand: it
    /// narrowed itself to `Adf` because an HFS disk would detect and then fail
    /// to open. Under one table that cannot happen, and this test says so out
    /// loud rather than leaving it to a comment.
    #[test]
    fn whatever_detect_claims_mounts_and_answers_every_question() {
        for image in DiskImage::all() {
            let raw = sample_of(image);
            assert_eq!(DiskImage::detect(&raw), Some(image), "{image:?} is not detected");

            let disk = MountedDisk::mount(raw)
                .unwrap_or_else(|e| panic!("{image:?} detects but will not mount: {e}"));
            assert_eq!(disk.format(), image);
            assert_eq!(disk.label(), image.label());
            assert_eq!(disk.interpreter_number(), image.interpreter_number());
            let (story_name, others) = sample_entries(image);
            assert_eq!(disk.file_count(), 1 + others.len(), "{image:?} lists what it mounted");

            // Identified by CONTENT: `Readme` is not a story and `STORY.DAT`
            // is, and no format is allowed to decide that by name.
            let stories = disk.stories();
            assert_eq!(stories.len(), 1, "{image:?} found {stories:?}");
            assert_eq!(stories[0].name, story_name, "{image:?}");
            assert_eq!(stories[0].bytes, fake_story(), "{image:?} reads it byte-exact");
            assert_eq!(disk.story().map(|s| s.bytes), Some(fake_story()), "{image:?}");

            // No archive on a synthetic disk — the point is that the question is
            // answerable at all, on every format, without a panic or a chain.
            assert!(disk.pictures().is_none(), "{image:?}");
            let _ = disk.volume_name();
        }
    }

    /// The unfiltered listing every format hands back, and the reason it is on
    /// [`MountedDisk`] at all: `pictures()` answers with ONE archive by the
    /// format's own tiebreak, so a caller offering a person a choice between a
    /// disk's two archives has to see all its files (SQ-0843).
    #[test]
    fn a_mounted_disk_lists_every_file_it_holds() {
        for image in DiskImage::all() {
            let disk = MountedDisk::mount(sample_of(image)).expect("mounts");
            let contents = disk.contents();
            let names: Vec<&str> = contents.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(names.len(), disk.file_count(), "{image:?} lists what it counted");
            let (story_name, others) = sample_entries(image);
            assert!(names.contains(&story_name), "{image:?}: {names:?}");
            for other in others {
                assert!(names.contains(other), "{image:?}: {names:?}");
                // Bytes, not just names — this is what a caller identifies by.
                let file = contents.iter().find(|(n, _)| n == other).expect("present");
                assert_eq!(file.1, b"just a text file", "{image:?}");
            }
            // …and the story is in the listing too, unfiltered: deciding what a
            // file IS belongs to the caller, not to the mount.
            assert!(contents.iter().any(|(_, b)| *b == fake_story()), "{image:?}");
        }
    }

    /// **What a caller is shown, it can ask for** — on every format.
    ///
    /// `contents` is how the launch dialog enumerates a disk's artwork and
    /// `read_named` is how the `--pictures` door then loads it, so the two
    /// disagreeing is a file that is offered and cannot be opened. That is
    /// exactly what happened while `graphics::read_off_the_medium` still carried
    /// its own two-reader chain (SQ-0833): a FAT12 disk enumerated through the
    /// table and loaded through a chain that had never heard of it.
    ///
    /// The matching rule stays each format's own; what is pinned here is that
    /// the name in hand is one the same volume answers to.
    #[test]
    fn every_name_a_disk_lists_is_a_name_it_will_read_back() {
        for image in DiskImage::all() {
            let disk = MountedDisk::mount(sample_of(image)).expect("mounts");
            for (name, bytes) in disk.contents() {
                assert_eq!(
                    disk.read_named(&name).as_ref(),
                    Some(&bytes),
                    "{image:?}: listed {name:?} and would not read it back"
                );
                // …and case is not what decides it, on any format.
                assert_eq!(
                    disk.read_named(&name.to_ascii_lowercase()).as_ref(),
                    Some(&bytes),
                    "{image:?}: {name:?} is case-sensitive"
                );
            }
            assert_eq!(disk.read_named("NoSuchFile.data"), None, "{image:?}");
        }
    }

    /// **The table order is a formality rather than a precedence** — stated as a
    /// test over the whole corpus rather than left to [`DiskImage::detect`]'s
    /// doc comment.
    ///
    /// `detect` returns the FIRST row whose sniff fires, so two rows claiming
    /// one file would make the table's order load-bearing and a format's
    /// identity depend on where it was pasted in. This walks every file in
    /// `stories/` — Amiga, Macintosh, DOS, Atari ST and ProDOS floppies, bare
    /// story files, Blorbs, saved games and loose artwork — and insists that at
    /// most one row ever claims one, and that whatever is claimed opens.
    ///
    /// SQ-0836 is why it exists: a fifth filesystem is the point at which "they
    /// happen not to collide" stops being obvious.
    #[test]
    fn at_most_one_format_ever_claims_a_file() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("SKIP: no stories directory");
            return;
        };
        let mut seen = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(raw) = std::fs::read(&path) else { continue };
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let claims: Vec<DiskImage> =
                FORMATS.iter().filter(|f| (f.looks_like)(&raw)).map(|f| f.image).collect();
            assert!(claims.len() <= 1, "{name}: claimed by {claims:?}");
            let Some(image) = claims.first().copied() else { continue };
            seen += 1;
            assert_eq!(DiskImage::detect(&raw), Some(image), "{name}");
            let disk = MountedDisk::mount(raw).unwrap_or_else(|e| panic!("{name}: {e}"));
            // A mounted disk normally lists something. The one exception in the
            // corpus is a **side** of a multi-disk Commodore release (SQ-0869):
            // `TRINITY1.D64` and `TRINITY2.D64` each hold part of one game and
            // no whole file, so mounting either alone — which is what this walk
            // does — correctly finds nothing. Said as a narrowing of the claim
            // rather than as a `continue`, so it stays a property of the corpus
            // and not a hole in the test.
            if disk.file_count() == 0 {
                assert_eq!(image, DiskImage::CommodoreD64, "{name}: mounted but empty");
                assert!(disk.story().is_none(), "{name}: no files, and no game either");
            }
        }
        if seen == 0 {
            eprintln!("SKIP: no release media present");
        }
    }

    /// The same property on the real disk that motivated it: an ST compilation
    /// names its files by folder, and the folder has to survive the round trip
    /// or a picker offers `HITCHHIK/STORY.DAT` and opens nothing.
    #[test]
    fn a_real_directoried_disk_reads_back_the_paths_it_lists() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/Infocom Compilation 9 (19xx)(-).st");
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return;
        };
        let disk = MountedDisk::mount(bytes).expect("the ST disk mounts");
        assert_eq!(
            disk.read_named("HITCHHIK/STORY.DAT").map(|b| b.len()),
            Some(113444),
            "the path names the game"
        );
        assert_eq!(
            disk.read_named("cuthroat/story.dat").map(|b| b.len()),
            Some(112558),
            "…case-insensitively, like every other format here"
        );
        // The bare filename still resolves — to the first of the four, which is
        // why a picker shows the path and not this.
        assert!(disk.read_named("STORY.DAT").is_some());
    }

    /// A boot disk carries files and no game, on any format that has files. The
    /// mount succeeds — that is what lets a caller say "is this the boot disk?"
    /// instead of "corrupt story file".
    ///
    /// **[`DiskImage::InfocomBootDisk`] is excluded, and it is the case the test
    /// is named after** (SQ-0868). A raw self-booting disk has no filesystem, so
    /// its identity IS the story on it: "a disk of this format with files and no
    /// game" is not a thing that can be constructed, because without the game
    /// there is nothing left to recognise. It is refused as not-a-disk-image
    /// rather than mounted empty, which is the right answer and a different one.
    #[test]
    fn a_disk_with_no_game_mounts_and_offers_nothing() {
        for image in DiskImage::all() {
            let files: [(&str, &[u8]); 2] =
                [("Startup-Sequence", b"LoadWB\n"), ("Desktop", b"\x00\x01\x02\x03")];
            let raw = match image {
                DiskImage::Adf => crate::adf::tests::sample_disk(&files),
                DiskImage::Hfs => crate::hfs::tests::sample_disk(&files),
                DiskImage::Fat12Dos => {
                    crate::fat12::tests::sample_disk(&files, crate::fat12::Machine::Dos)
                }
                DiskImage::Fat12AtariSt => {
                    crate::fat12::tests::sample_disk(&files, crate::fat12::Machine::AtariSt)
                }
                DiskImage::ProDos => crate::prodos::tests::sample_disk(&files),
            DiskImage::Iso9660 => crate::iso9660::tests::sample_disc(&files),
                DiskImage::InfocomBootDisk => {
                    // Said as an assertion rather than as a `continue`, so the
                    // exclusion is a claim this test checks and not a hole in it.
                    let blank = vec![0u8; crate::dos_order::DOS_ORDER_LEN];
                    assert_eq!(DiskImage::detect(&blank), None, "no story, no boot disk");
                    continue;
                }
                // Excluded for the same reason, and asserted the same way: a
                // Commodore press keeps no files, so "files and no game" cannot
                // be built. A 1541 image with a readable, empty directory and
                // nothing outside it is refused rather than mounted empty.
                //
                // The mounted-but-gameless case DOES exist on this format — it
                // is either side of *Trinity* — but that is a disk with no files
                // AND no game, which `crate::d64` covers directly.
                DiskImage::CommodoreD64 => {
                    let blank = vec![0u8; crate::d64::D64_LEN];
                    assert_eq!(DiskImage::detect(&blank), None, "no story, no Commodore disk");
                    continue;
                }
                // Excluded on the same ground, and the assertion is worth more
                // here than on the row above: the bitstream really does decode —
                // 683 sectors of it — and is still refused, because what makes
                // these bytes a release is the story and not the sectors.
                DiskImage::CommodoreG64 => {
                    let blank = crate::g64::tests::g64_of(&vec![0u8; crate::d64::D64_LEN]);
                    assert_eq!(DiskImage::detect(&blank), None, "no story, no Commodore disk");
                    continue;
                }
                // Filesystems, so "files and no game" is an ordinary disk.
                DiskImage::AtariDos2 => crate::atr::tests::sample_disk(&files),
                DiskImage::AppleDos33 => crate::dos33::tests::sample_disk(&files),
                // Excluded, and asserted rather than skipped: a loadable binary
                // is ONE entry by construction, so a two-file version of it
                // cannot be built. What it can be is a binary that loads
                // something which is not a game, and that mounts, lists its one
                // entry, and offers no story — which is the claim being made.
                DiskImage::AtariXex => {
                    let raw = crate::xex::tests::sample_binary(b"LoadWB\n");
                    let disk = MountedDisk::mount(raw).expect("a loadable binary still mounts");
                    assert_eq!(disk.file_count(), 1);
                    assert!(disk.stories().is_empty());
                    assert_eq!(disk.story(), None);
                    continue;
                }
            };
            let disk = MountedDisk::mount(raw).expect("a boot disk still mounts");
            assert_eq!(disk.file_count(), 2, "{image:?}");
            assert!(disk.stories().is_empty(), "{image:?}");
            assert_eq!(disk.story(), None, "{image:?}");
        }
    }

    /// An ordinary story file is not a disk, and says so in the one way every
    /// caller falls through on.
    #[test]
    fn mounting_a_plain_story_file_is_not_a_disk_image() {
        let mut story = vec![0u8; 0x400];
        story[0] = 3;
        assert_eq!(MountedDisk::mount(story).map(|d| d.format()), Err(MountError::NotADiskImage));
    }

    /// The error a front-end prints must be about a disk, never about a
    /// filesystem's internals — `app` and `zvm-cli` do not know what an extents
    /// overflow file is and must never have to.
    #[test]
    fn a_mount_error_reads_as_a_disk_problem_not_a_filesystem_one() {
        assert_eq!(MountError::NotADiskImage.to_string(), "not a disk image in any format we read");
        assert_eq!(
            MountError::Unreadable(DiskImage::Hfs).to_string(),
            "the HFS filesystem on it would not read"
        );
    }

    /// Real media, every format: the disks the corpus actually holds mount
    /// through the shared path and hand back their own story — and their own
    /// art wherever the medium carries any. They live outside the repo, so each
    /// arm skips vacuously.
    ///
    /// The match is exhaustive, so a new format has to name the disk that
    /// proves it rather than quietly riding on somebody else's fixture. What
    /// each arm expects is the medium's own truth and not a shared shape: three
    /// of the four are *Zork Zero* in one press or another, and the fourth is
    /// an Atari ST compilation, because **no ST v6 release exists in this
    /// corpus** — all thirty-eight ST stories are v3, v4 or v5, and none of the
    /// nine disks carries artwork at all.
    /// SQ-0875: `mount_set` must not copy an image it will never read.
    ///
    /// The clone has to happen BEFORE the mount consumes `raw`, and whether this
    /// volume has a story of its own is only knowable after — so the decision
    /// cannot be made from the volume. It is made from the row instead, and only
    /// a format that pages a story across raw SECTORS ever needs the images: the
    /// Apple's packed volume pages across `.D1`…`.D5`, which are files.
    ///
    /// Cloning unconditionally cost a copy of the whole image on every ordinary
    /// mount, dropped again a few lines later — 354 MB on the hybrid CD, 12 MB on
    /// a Macintosh volume. `sides` staying EMPTY is what proves it was not paid.
    ///
    /// FALSIFICATION: make `mine` unconditional (`Some(raw.clone())`) and the
    /// `sides.is_empty()` assertion below fails on every fixture; keep the clone
    /// conditional but count volumes with `sides.len()` again and the
    /// multi-volume tests fail instead, which is the pair this split exists for.
    #[test]
    fn a_format_that_reads_files_keeps_no_copy_of_the_image() {
        // Exactly one row wants whole images, and it is the Commodore's.
        let paging: Vec<DiskImage> =
            FORMATS.iter().filter(|f| f.pages_across_images).map(|f| f.image).collect();
        assert_eq!(
            paging,
            vec![DiskImage::CommodoreD64, DiskImage::CommodoreG64],
            "only a raw-sector set needs the images, and both Commodore containers are one"
        );

        let mut ran = 0;
        for (fixture, image) in [
            ("Zork Zero Disk.image", DiskImage::Hfs),
            ("Zork Zero - The Revenge of Megaboz.adf", DiskImage::Adf),
            ("floppy5.ima", DiskImage::Fat12Dos),
            // A member of a real multi-volume release, so the set path is the one
            // being measured — not merely a disk that answered for itself.
            ("shogun_s1.dsk", DiskImage::ProDos),
        ] {
            let Ok(raw) = std::fs::read(stories_dir().join(fixture)) else { continue };
            ran += 1;
            // Shogun's story is on none of its five floppies, so its set path
            // really runs; the others answer for themselves and never ask.
            let companions =
                if fixture == "shogun_s1.dsk" { read_set(&SHOGUN_SET).unwrap_or_default() } else { Vec::new() };
            let disk = MountedDisk::mount_set(raw, || companions)
                .unwrap_or_else(|e| panic!("{fixture}: should mount: {e:?}"));
            assert_eq!(disk.format(), image, "{fixture}");
            assert!(
                disk.sides.is_empty(),
                "{fixture}: a format whose assembler reads files must keep no image copy"
            );
        }
        assert!(ran > 0 || !stories_dir().is_dir(), "media are present but none were read");
    }

    #[test]
    fn real_release_disks_of_every_format_mount_through_one_path() {
        for image in DiskImage::all() {
            // (fixture, the entry's stored name, its version, does the disk
            //  also carry a picture archive?)
            //
            // **A version of `0` means the medium carries no Z-machine story at
            // all** (SQ-1458), and then `story_name` is an entry it must LIST
            // and read back instead. The three formats that answer that way are
            // the Scott Adams media: nothing Infocom ever pressed is an Atari
            // 8-bit disk, an Atari loadable binary or an Apple II DOS 3.3
            // volume, so there is no Z-code on any specimen of them and saying
            // otherwise would be inventing a fixture. What the property under
            // test needs is unchanged — detect, mount, name itself, list, read
            // back — and each of those is asserted for every row alike.
            let (fixture, story_name, version, has_art) = match image {
                DiskImage::Adf => ("Zork Zero - The Revenge of Megaboz.adf", "Story.data", 6, true),
                DiskImage::Hfs => ("Zork Zero Disk.image", "Story.data", 6, true),
                // The one fixture NOT in `stories/`: the Lost Treasures discs
                // live in `treasures/` beside the other CD-ROMs, and the loader
                // below looks there when `stories/` has no such name.
                //
                // Shogun's DOS pressing is the largest story on disc 2 at
                // 345,088 bytes — just over its Macintosh pressing's 341,416 —
                // and largest is this format's tiebreak, the two halves having
                // no naming convention in common (SQ-0871). A compilation wants
                // `stories()`; this pins that the single-story door answers
                // deterministically at all.
                DiskImage::Iso9660 => ("LostTreasures2.iso", "DOS/SHOGUN/SHOGUN.ZIP", 6, true),
                // Lost Treasures I floppy5: Zork Zero's story AND its EGA art.
                // (Its CGA art is on floppy4, which is the whole of why a set
                // model is a real thing this lane does not have.)
                DiskImage::Fat12Dos => ("floppy5.ima", "ZORK0.ZIP", 6, true),
                // Four games in four folders, ALL called `STORY.DAT` — so the
                // conventional-name tiebreak cannot separate them and the
                // largest wins, which is *Bureaucracy* (v4, 243200 bytes).
                // Deterministic, and a compilation wants `stories()` anyway.
                DiskImage::Fat12AtariSt => {
                    ("Infocom Compilation 9 (19xx)(-).st", "BUREAUCR.ACY/STORY.DAT", 4, false)
                }
                // The Apple IIgs, and its own truth again: *Lost Treasures*
                // volume 2 holds five games with no conventional name between
                // them, so the largest opens — *Beyond Zork*, v5, 261 388 bytes
                // — and no ProDOS release in the corpus carries an Infocom
                // picture archive at all. (The two that DO ship artwork,
                // *Arthur* and *Journey*, keep it inside the same segmented
                // container as their story and offer no whole story file; see
                // `crate::prodos`.)
                DiskImage::ProDos => (
                    "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 2 of 7).2mg",
                    "BEYOND.ZORK",
                    5,
                    false,
                ),
                // **The first non-v6 Apple disk in the corpus** (SQ-0868), and
                // the only one here whose story has no stored name at all — the
                // medium has no directory, so the mount reports where the story
                // is instead. *Planetfall* release 29, serial 840118.
                DiskImage::InfocomBootDisk => {
                    ("Planetfall r29 (clean copy from retail disk).dsk", "T3/S0", 3, false)
                }
                // **The only single-disk Commodore press in the corpus**
                // (SQ-0869) — *Hitchhiker's* release 47, serial 840914, off the
                // 1984 C64 floppy, whose story is 16 sectors of every track from
                // track 5. Named here rather than *Trinity*, because *Trinity*
                // is on TWO disks and this test mounts one: `MountedDisk::mount`
                // is `mount_set` with no companions, so it correctly finds no
                // game on either side. That pair is exercised through
                // `mount_set` in `crate::d64` and end to end in the app's
                // `real_media_releases`.
                DiskImage::CommodoreD64 => {
                    ("Hitchhikers_Guide_to_the_Galaxy_The_1984_Infocom.d64", "T5/S0", 3, false)
                }
                // **The only bitstream dump in the corpus** (SQ-1095) —
                // *Plundered Hearts* release 26, serial 870730, off a 1987
                // 40-track C64 floppy nibbled to GCR. Its story is seventeen
                // sectors of every track from track 5, which is a third layout
                // in three presses, and what comes off it is byte-identical to
                // `plunderedhearts-r26-s870730.z3` — pinned in `crate::g64`,
                // where the oracle lives.
                DiskImage::CommodoreG64 => {
                    ("plundered_hearts[infocom_1987](r26)(!).g64", "T5/S0", 3, false)
                }
                // **The S.A.G.A. Atari 8-bit database side** — and the entry it
                // must list is the one the disk still has a directory for, not
                // the game: this release masters its database over sectors
                // 360-368, so the filesystem it names is the DOS that boots it.
                // The database is reached by file offset through
                // `crate::atr::IMAGE_ENTRY`, which is `crate::atr`'s business
                // and not this table's.
                DiskImage::AtariDos2 => (
                    "scott-dialects/atari/SAGA #1 - Adventureland [side A].atr",
                    "AUTORUN.SYS",
                    0,
                    false,
                ),
                // **The Apple II S.A.G.A. boot side**, whose database IS a file
                // and is named `A1.DAT` — one of §7.4's four recognised
                // spellings.
                DiskImage::AppleDos33 => (
                    "scott-dialects/apple/Scott Adams Graphic Adventure 1 - Adventureland v2.1-416 (4am crack) side B - boot.dsk",
                    "A1.DAT",
                    0,
                    false,
                ),
                // **The one loadable binary in the corpus** — Questprobe #1, The
                // Hulk, loading `$4000..$9530`.
                DiskImage::AtariXex => {
                    ("scott-dialects/atari/The Hulk.xex", "$4000-$9530", 0, false)
                }
            };
            // `stories/` for a floppy, `treasures/` for a CD-ROM — both
            // gitignored, and a fixture in neither is a skip.
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
            let path = [root.join("../../stories"), root.join("../../treasures")]
                .into_iter()
                .map(|d| d.join(fixture))
                .find(|p| p.is_file());
            let Some(path) = path else {
                eprintln!("SKIP: {image:?} media absent ({fixture})");
                continue;
            };
            let Ok(bytes) = std::fs::read(&path) else {
                eprintln!("SKIP: {image:?} media unreadable at {}", path.display());
                continue;
            };
            assert_eq!(DiskImage::detect(&bytes), Some(image), "{fixture}");
            let disk = MountedDisk::mount(bytes).expect("the release disk mounts");
            assert_eq!(disk.format(), image, "{fixture}");
            if version == 0 {
                // A medium with no Z-code on it says so, and still lists and
                // reads back the entry this table names.
                assert_eq!(disk.story(), None, "{fixture}: nothing here is Z-code");
                assert!(disk.stories().is_empty(), "{fixture}");
                let listed = disk.contents();
                let found = listed
                    .iter()
                    .find(|(name, _)| name == story_name)
                    .unwrap_or_else(|| panic!("{fixture}: no {story_name} in {listed:?}"));
                assert_eq!(disk.read_named(story_name).as_ref(), Some(&found.1), "{fixture}");
                assert_eq!(disk.pictures().map(|a| a.name), None, "{fixture}");
                continue;
            }
            let story = disk.story().expect("the release disk carries its game");
            assert_eq!(story.name, story_name, "{fixture}");
            assert_eq!(story.bytes[0], version, "{fixture}");
            match disk.pictures() {
                Some(art) => {
                    assert!(has_art, "{fixture}: unexpected artwork {}", art.name);
                    assert!(art.pictures.entries().len() > 100, "{fixture}: {}", art.name);
                }
                None => assert!(!has_art, "{fixture}: its own artwork is missing"),
            }
        }
    }

    /// **A compilation disk is a list, not a game** — and the list is what a
    /// picker shows. `Infocom Compilation 9` is the sharpest case in the
    /// corpus: four games, four folders, four files called `STORY.DAT`, and a
    /// saved game sitting beside three of them.
    #[test]
    fn a_real_compilation_disk_lists_every_game_and_no_saved_game() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/Infocom Compilation 9 (19xx)(-).st");
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return;
        };
        let disk = MountedDisk::mount(bytes).expect("the ST disk mounts");
        assert_eq!(disk.format(), DiskImage::Fat12AtariSt);
        assert_eq!(disk.label(), "ST");
        assert_eq!(
            disk.interpreter_number(),
            Some(ATARI_ST_INTERPRETER_NUMBER),
            "an ST floppy announces the Atari ST — ZMSD §11.1.3, and the machine's own \
             `INTWRD DC.B 5 * MACHINE ID FOR ATARI ST`",
        );
        let names: Vec<String> = disk.stories().into_iter().map(|s| s.name).collect();
        assert_eq!(names, [
            "HITCHHIK/STORY.DAT",
            "BUREAUCR.ACY/STORY.DAT",
            "CUTHROAT/STORY.DAT",
            "LEATHER.GOD/STORY.DAT",
        ]);
        assert_eq!(disk.file_count(), 14, "…out of fourteen files, saves and interpreters included");
    }

    // ── A disc, not a floppy (SQ-0870) ────────────────────────────────────────

    /// **A hybrid CD is the Macintosh row wearing another container**, and the
    /// table needed no new entry to read one: the sniff sees past the raw
    /// sectors and the partition map to the same `BD` signature a floppy has.
    ///
    /// Both framings, because both circulate — a raw `.bin` dump keeps the whole
    /// 2352-byte frame and a cooked `.iso` keeps only the 2048 bytes of user
    /// data. The cooked one is read in place, with nothing copied at all; see
    /// [`crate::cd`].
    ///
    /// Synthetic, so CI runs it. The real disc is in
    /// [`crate::hfs`]'s tests.
    #[test]
    fn a_partitioned_disc_mounts_as_the_macintosh_volume_it_carries() {
        let story = fake_story();
        let files: [(&str, &[u8]); 2] = [("Readme", b"just a text file"), ("STORY.DAT", &story)];
        let volume = crate::hfs::tests::sample_volume(&files);
        // A partition map claiming twenty times the space the disc holds, which
        // is what a hybrid disc's own map does.
        let cooked = crate::cd::tests::partitioned(&volume, 20 * volume.len() / 512);
        let raw = crate::cd::tests::raw_sectors(&cooked);

        for (what, image) in [("a cooked .iso", cooked), ("a raw .bin dump", raw)] {
            assert_eq!(DiskImage::detect(&image), Some(DiskImage::Hfs), "{what}");
            // …and by exactly one row: a disc is not claimed by a floppy format
            // that happens to be asked first.
            let claims: Vec<DiskImage> =
                FORMATS.iter().filter(|f| (f.looks_like)(&image)).map(|f| f.image).collect();
            assert_eq!(claims, [DiskImage::Hfs], "{what}");

            let disk = MountedDisk::mount(image).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(disk.volume_name(), Some("Test Disk"), "{what}");
            assert_eq!(disk.file_count(), 2, "{what}");
            assert_eq!(disk.story().map(|s| s.bytes), Some(story.clone()), "{what}");
            // The medium is still a Macintosh, so the number is still the
            // Macintosh's — a container does not change the machine.
            assert_eq!(disk.interpreter_number(), Some(MACINTOSH_INTERPRETER_NUMBER), "{what}");
        }
    }

    /// **Real media**: the hybrid *Masterpieces* CD is claimed by one row and
    /// opens through the shared path, with no front-end taught anything
    /// (SQ-0870).
    ///
    /// The disjointness assertion is the one that earns its keep at 354 MB: a
    /// file that large sails past most size-based sniffs, so "at most one row
    /// claims it" is worth checking on the medium itself rather than inferring
    /// from the corpus in `stories/`, which this walk does not cover.
    ///
    /// Skips vacuously — CI has no `masterpieces/`.
    #[test]
    fn the_real_hybrid_cd_is_claimed_by_exactly_one_row_and_mounts() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../masterpieces/Classic Text Adventure Masterpieces of Infocom (USA).bin");
        let Ok(raw) = std::fs::read(&path) else {
            eprintln!("SKIP: the Masterpieces CD is absent at {}", path.display());
            return;
        };
        assert_eq!(raw.len(), 354_011_280);
        let claims: Vec<DiskImage> =
            FORMATS.iter().filter(|f| (f.looks_like)(&raw)).map(|f| f.image).collect();
        assert_eq!(claims, [DiskImage::Hfs], "a 354 MB disc image is claimed once, by HFS");

        let disk = MountedDisk::mount(raw).expect("the disc mounts");
        assert_eq!(disk.format(), DiskImage::Hfs);
        assert_eq!(disk.volume_name(), Some("Masterpieces"));
        assert_eq!(disk.label(), "HFS");
        assert_eq!(disk.interpreter_number(), Some(MACINTOSH_INTERPRETER_NUMBER));
        assert_eq!(disk.stories().len(), 83, "the whole shelf, Macintosh and PC builds alike");
        // …and the row that claims it names the spelling a scan pre-filters on,
        // so the disc is offered rather than merely openable (SQ-0849's rule).
        assert!(image_extensions().any(|e| e == "bin"));
        // …and so does the cooked spelling the same disc is archived under
        // everywhere else (SQ-0879). Declining it meant the file was skipped
        // before its bytes were ever looked at, so a disc that opens perfectly
        // well by name was silently absent from the story list.
        assert!(image_extensions().any(|e| e == "iso"));
    }

    /// **A cooked 2048-byte image of the hybrid disc reads exactly as the raw
    /// dump does** (SQ-0879).
    ///
    /// Cooked here rather than kept on disk, because the property is that the
    /// two are the same volume and a second 308 MB fixture would prove nothing
    /// the arithmetic does not. MODE1/2352 carries 2048 bytes of user data at
    /// offset 16 of each sector; stripping the frames IS the cook.
    ///
    /// FALSIFICATION: drop `iso` from the row and this still passes — the
    /// reader never needed it — which is exactly why the extension census has
    /// its own assertion above. The two halves of SQ-0849's rule are separate:
    /// can we read it, and will a scan offer it.
    #[test]
    fn the_hybrid_disc_reads_the_same_cooked_as_raw() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../masterpieces");
        let file = "Classic Text Adventure Masterpieces of Infocom (USA).bin";
        let Ok(raw) = std::fs::read(dir.join(file)) else {
            eprintln!("SKIP: the raw disc is absent at {}", dir.join(file).display());
            return;
        };
        let cooked: Vec<u8> =
            raw.as_chunks::<2352>().0.iter().flat_map(|s| s[16..16 + 2048].iter().copied()).collect();
        assert!(
            DiskImage::detect(&cooked) == Some(DiskImage::Hfs),
            "the cooked image is the same Macintosh volume"
        );
        let disk = MountedDisk::mount(cooked).expect("the cooked image mounts");
        assert_eq!(disk.volume_name(), Some("Masterpieces"));
        assert_eq!(disk.stories().len(), 83, "every story the raw dump offers");
        assert_eq!(disk.image_for("PC/AMFV/AMFV.DAT"), DiskImage::Fat12Dos, "and both halves");
    }

    /// **Every disc the user drops in `treasures/` mounts AND would be offered
    /// by a directory scan** (SQ-0879).
    ///
    /// That directory exists for cooked `.iso` pressings, and it is gitignored
    /// like `stories/` and `masterpieces/`, so this skips vacuously when it is
    /// empty or absent — which is CI, always.
    ///
    /// Both halves are asserted because they fail independently, and the second
    /// is the one that bites silently: a reader that opens a medium a SCAN never
    /// offers is a disc missing from the story list with nothing on screen to say
    /// why. That is SQ-0849's defect, and it has now recurred twice.
    #[test]
    fn every_disc_in_treasures_mounts_and_a_scan_would_offer_it() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../treasures");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("SKIP: no treasures/ at {}", dir.display());
            return;
        };
        let mut ran = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if name.starts_with('.') {
                continue; // .DS_Store and friends
            }
            let raw = std::fs::read(&path).expect("readable");
            // CONTENT decides what is a disc, so the box scans and the checksum
            // file sitting beside the discs are simply not this test's business.
            let Some(image) = DiskImage::detect(&raw) else { continue };
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            assert!(
                image_extensions().any(|e| e == ext),
                "{name}: we can read it as {}, but no row claims {ext:?} — so a directory \
                 scan skips the file before its bytes are ever looked at",
                image.label()
            );
            let disk = MountedDisk::mount(raw).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(
                !disk.stories().is_empty(),
                "{name}: mounted as {} and offered no story",
                image.label()
            );
            ran += 1;
        }
        if ran == 0 {
            eprintln!("SKIP: treasures/ holds no disc images yet");
        }
    }

    // ── A story on no single disk (SQ-0864) ───────────────────────────────────

    /// `stories/`, gitignored — every case below skips vacuously without it, and
    /// CI has none of it at all.
    fn stories_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories")
    }

    /// The 5.25-inch sets, as the filenames in `stories/` spell them.
    const SHOGUN_SET: [&str; 5] = [
        "shogun_s1.dsk",
        "shogun_s2.dsk",
        "shogun_s3.dsk",
        "shogun_s4.dsk",
        "shogun_s5.dsk",
    ];
    const ZORK_ZERO_SET: [&str; 4] =
        ["zork_zero_1.dsk", "zork_zero_2.dsk", "zork_zero_3.dsk", "zork_zero_4.dsk"];

    /// Every volume of `set`, or `None` when any of them is absent.
    fn read_set(set: &[&str]) -> Option<Vec<Vec<u8>>> {
        set.iter().map(|n| std::fs::read(stories_dir().join(n)).ok()).collect()
    }

    /// **The headline.** *Shogun* is on five floppies and *Zork Zero* on four,
    /// and not one of the nine carries a game — so this is the only test in the
    /// file whose subject is a release rather than a disk.
    ///
    /// Every member is opened in turn with the other volumes as companions,
    /// because that is what a person does: they name whichever floppy is on top.
    /// All five must give the same story, under the same name, and it must be
    /// the real one — pinned by release, serial and the story's own header
    /// checksum, which is the oracle that says the pages were reassembled in the
    /// right order rather than merely plausibly (ZMSD §11.1.6).
    /// One 5.25-inch press: its volumes, and the build the whole set carries.
    struct Press {
        /// Every floppy of the release, in disk order.
        set: &'static [&'static str],
        /// The segment carrying the index, which names the reassembled story.
        story_name: &'static str,
        /// The story's declared length, header `$1A` in v6 units.
        length: usize,
        /// Header `$02`.
        release: u16,
        /// Header `$12..$18`.
        serial: &'static str,
        /// Header `$1C` — the oracle that says the pages went back in order.
        checksum: u16,
    }

    #[test]
    fn a_release_pressed_across_five_floppies_opens_from_any_one_of_them() {
        let releases = &[
            Press {
                set: &SHOGUN_SET,
                story_name: "SHOGUN.D1",
                length: 344_224,
                release: 311,
                serial: "890510",
                checksum: 0xE200,
            },
            Press {
                set: &ZORK_ZERO_SET,
                story_name: "ZORK0.D1",
                length: 299_392,
                release: 383,
                serial: "890602",
                checksum: 0x6F7F,
            },
        ];
        let mut ran = 0;
        for Press { set, story_name, length, release, serial, checksum } in releases {
            let Some(images) = read_set(set) else {
                eprintln!("SKIP: {} is not complete in stories/", set[0]);
                continue;
            };
            ran += 1;
            for (n, named) in images.iter().enumerate() {
                let rest: Vec<Vec<u8>> = images
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != n)
                    .map(|(_, b)| b.clone())
                    .collect();
                let who = set[n];
                let disk = MountedDisk::mount_set(named.clone(), || rest)
                    .unwrap_or_else(|e| panic!("{who}: {e}"));
                // It is a ProDOS volume like any other, and answers so.
                assert_eq!(disk.format(), DiskImage::ProDos, "{who}");
                assert_eq!(disk.label(), "ProDOS", "{who}");
                assert_eq!(
                    disk.interpreter_number(),
                    Some(APPLE_IIGS_INTERPRETER_NUMBER),
                    "{who}: a 5.25-inch press is Apple II media like the 3.5-inch one",
                );

                let stories = disk.stories();
                assert_eq!(stories.len(), 1, "{who}: {:?}", stories.len());
                let story = &stories[0];
                assert_eq!(story.name, *story_name, "{who}: named for the index segment");
                assert_eq!(disk.story().map(|s| s.name), Some(story.name.clone()), "{who}");
                assert_eq!(disk.story().map(|s| s.bytes), Some(story.bytes.clone()), "{who}");

                let bytes = &story.bytes;
                assert_eq!(bytes.len(), *length, "{who}: the story's declared length");
                assert_eq!(bytes[0], 6, "{who}: Version 6");
                assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), *release, "{who}");
                assert_eq!(&bytes[0x12..0x18], serial.as_bytes(), "{who}");
                let sum = bytes[64..].iter().fold(0u16, |a, &b| a.wrapping_add(u16::from(b)));
                assert_eq!(sum, *checksum, "{who}: ZMSD §11.1.6 checksum");
                assert_eq!(u16::from_be_bytes([bytes[0x1c], bytes[0x1d]]), sum, "{who}");
            }
        }
        // CI carries no `stories/`, so the premise is guarded and not asserted.
        assert!(ran > 0 || !stories_dir().join(SHOGUN_SET[0]).exists());
        if ran == 0 {
            eprintln!("SKIP: no 5.25-inch media present");
        }
    }

    /// **What a lone volume of a set does**, which is the question a person asks
    /// by double-clicking `shogun_s3.dsk`.
    ///
    /// It mounts. It is a real ProDOS volume with a real name and a real file on
    /// it, and it is honest that the file is not a game — so the front-ends'
    /// existing "no story file on the disk image (N files on SHOGUN.3; is this
    /// the boot disk?)" is what a person is told, rather than a refusal that
    /// would suggest the floppy was unreadable or a truncated story that would
    /// crash later. **Nothing is ever handed over half-assembled**: the header
    /// checksum in [`crate::infocom_packed`] refuses four fifths of a game.
    #[test]
    fn a_lone_volume_of_a_set_mounts_and_says_it_has_no_game() {
        // (image, volume name, files on it)
        let lone: &[(&str, &str, usize)] = &[
            ("shogun_s1.dsk", "SHOGUN.1", 4), // the one carrying the index…
            ("shogun_s3.dsk", "SHOGUN.3", 1), // …and one that carries only pages
            ("zork_zero_1.dsk", "ZORK0.1", 4),
            ("zork_zero_4.dsk", "ZORK0.4", 1),
        ];
        let mut ran = 0;
        for (file, volume, files) in lone {
            let Ok(raw) = std::fs::read(stories_dir().join(file)) else { continue };
            ran += 1;
            assert_eq!(DiskImage::detect(&raw), Some(DiskImage::ProDos), "{file}");
            let disk = MountedDisk::mount(raw).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert_eq!(disk.volume_name(), Some(*volume), "{file}");
            assert_eq!(disk.file_count(), *files, "{file}");
            assert!(disk.stories().is_empty(), "{file}: a fifth of a game is not a game");
            assert_eq!(disk.story(), None, "{file}");
        }
        assert!(ran > 0 || !stories_dir().join("shogun_s1.dsk").exists());
        if ran == 0 {
            eprintln!("SKIP: no 5.25-inch media present");
        }
    }

    /// **Volumes that are not one release are refused, not spliced.**
    ///
    /// Shogun's index disk with Zork Zero's three page disks is a set that
    /// pairs by NAME perfectly well — every `SHOGUN.D2`…`D5` is simply absent —
    /// and giving it Shogun's own disk 2 beside three of Zork Zero's is the
    /// sharper case: the names now resolve and the pages do not. Either way
    /// nothing is handed out, which is the property that makes name-based
    /// pairing safe at all (`infocom_packed`'s header-checksum oracle).
    #[test]
    fn volumes_from_two_different_releases_assemble_nothing() {
        let (Some(shogun), Some(zork)) = (read_set(&SHOGUN_SET), read_set(&ZORK_ZERO_SET)) else {
            eprintln!("SKIP: both 5.25-inch sets are needed");
            assert!(!stories_dir().join(SHOGUN_SET[0]).exists());
            return;
        };
        // Shogun's index disk, with Zork Zero's volumes for company.
        let strangers: Vec<Vec<u8>> = zork[1..].to_vec();
        let disk = MountedDisk::mount_set(shogun[0].clone(), || strangers).expect("mounts");
        assert!(disk.stories().is_empty(), "no story spans these five");
        assert_eq!(disk.story(), None);

        // And the other direction, with one real sibling in the mix.
        let mixed: Vec<Vec<u8>> = vec![shogun[1].clone(), zork[1].clone(), zork[2].clone()];
        let disk = MountedDisk::mount_set(shogun[0].clone(), || mixed).expect("mounts");
        assert!(disk.stories().is_empty(), "three of Shogun's five are still not Shogun");
    }

    /// **Nobody else pays for the set.** The companions closure is called only
    /// when the named volume has no story of its own, so a library scan does not
    /// read seven 800 KB floppies to list one of them.
    ///
    /// Stated over every format's synthetic disk, because it is a property of
    /// the seam and not of ProDOS: each sample carries `STORY.DAT`, so each must
    /// answer for itself and never ask.
    #[test]
    fn a_volume_with_its_own_story_never_asks_for_its_siblings() {
        for image in DiskImage::all() {
            let asked = std::cell::Cell::new(false);
            let disk = MountedDisk::mount_set(sample_of(image), || {
                asked.set(true);
                Vec::new()
            })
            .expect("mounts");
            assert!(!asked.get(), "{image:?} read its siblings to open itself");
            assert_eq!(disk.stories().len(), 1, "{image:?}");
        }
        // …and a volume with nothing on it does ask, which is the other half.
        let files: [(&str, &[u8]); 1] = [("Readme", b"just a text file")];
        let asked = std::cell::Cell::new(false);
        let disk = MountedDisk::mount_set(crate::adf::tests::sample_disk(&files), || {
            asked.set(true);
            Vec::new()
        })
        .expect("mounts");
        assert!(asked.get(), "a volume with no game must consult its release");
        assert!(disk.stories().is_empty());
    }

    /// A 5.25-inch dump that is not a ProDOS volume is not a disk image here —
    /// refused outright rather than de-interleaved into something that gets
    /// misread. `.dsk` is overwhelmingly Apple II media and most of it is DOS
    /// 3.3, which this crate does not read.
    #[test]
    fn a_five_and_a_quarter_inch_image_that_is_not_prodos_is_refused() {
        let blank = vec![0u8; crate::dos_order::DOS_ORDER_LEN];
        assert_eq!(DiskImage::detect(&blank), None);
        assert_eq!(MountedDisk::mount(blank).err(), Some(MountError::NotADiskImage));
        // Noise of the right size, likewise — the volume directory decides.
        let noise: Vec<u8> =
            (0..crate::dos_order::DOS_ORDER_LEN).map(|i| (i * 31 + 7) as u8).collect();
        assert_eq!(DiskImage::detect(&noise), None);
    }

    /// The same property on the Apple IIgs press (SQ-0836). *Lost Treasures*
    /// volume 2 is the ProDOS analogue of the ST compilation above: five games
    /// on one disk, four of somebody's saved games beside them, and a format
    /// with no conventional story name at all — so the LIST is the answer and
    /// `story()`'s largest-wins tiebreak is only the default.
    ///
    /// It also pins the medium's own number, which is the interesting one: a
    /// ProDOS volume answers **10, the Apple IIgs** (SQ-0857, reversing SQ-0836).
    /// ZMSD §11.1.3 does give the Apple II family three numbers, and nothing on a
    /// volume says which machine pressed it — Infocom's own Apple II YZIP settles
    /// that by detecting the machine at boot. What makes 10 the answer anyway is
    /// that declining names a machine too, and the one it names is the
    /// DECSystem-20 or the IBM PC. See the row in [`FORMATS`].
    #[test]
    fn a_real_apple_iigs_compilation_lists_every_game_and_no_saved_game() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../stories/Lost Treasures of Infocom, The (1993)\
             (Big Red Computer Club)(Disk 2 of 7).2mg",
        );
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return;
        };
        let disk = MountedDisk::mount(bytes).expect("the ProDOS disk mounts");
        assert_eq!(disk.format(), DiskImage::ProDos);
        assert_eq!(disk.label(), "ProDOS");
        assert_eq!(disk.volume_name(), Some("INFOCOM2"));
        assert_eq!(
            disk.interpreter_number(),
            Some(APPLE_IIGS_INTERPRETER_NUMBER),
            "ProDOS names the Apple II family; 10 names the member lanthorn presents as",
        );
        let names: Vec<String> = disk.stories().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["ZORK.III", "ZORK.II", "ZORK.I", "HITCHHIKER", "BEYOND.ZORK"]);
        assert_eq!(disk.story().map(|s| s.name), Some("BEYOND.ZORK".to_string()), "the largest");
        assert_eq!(disk.file_count(), 10, "…out of ten files, four saved games included");
        // What a caller was shown it can ask for, on this format too.
        for (name, bytes) in disk.contents() {
            assert_eq!(disk.read_named(&name).as_ref(), Some(&bytes), "listed {name:?}");
        }
    }
}
