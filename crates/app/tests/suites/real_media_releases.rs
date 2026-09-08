//! SQ-0760: the release each **medium** actually carries, pinned.
//!
//! An Infocom Amiga release floppy is not the same story as the bare story file
//! sitting beside it in `stories/` — it is a *different build of the game*.
//! `Journey - The Quest Begins.adf` is release 30, serial 890322; `journey.z6`
//! is release 83, serial 890706, and the two do not behave alike: r83 narrates
//! through window 0, r30 through window 2. That difference was the whole of
//! SQ-0755's reopened defect, and it cost SQ-0747 five investigation passes,
//! three of them sweeping tens of thousands of configurations against a release
//! the user does not play.
//!
//! `InterpreterProfile::resolve` reads the MEDIUM, so "under the Amiga profile"
//! names a machine *and* — when the fixture is a disk image — a different build.
//! Until this file there was **no committed test that drove a floppy at all**:
//! the SQ-0755 corpus measured both media for Zork Zero, Shogun and Arthur and
//! found them to agree, but an agreement measured once and never pinned is
//! exactly how this slipped through the first time.
//!
//! So: one table, [`MEDIA`], naming every medium in `stories/` together with the
//! version, release and serial it must load — and since SQ-1158 a guard,
//! [`every_disk_image_in_stories_is_pinned_or_explained`], that fails on any
//! medium the table has never heard of, because an absence from a
//! hand-maintained table is indistinguishable from a fact about the world and
//! was read as one.
//!
//! Every case walks the table, guards on that identity FIRST, and prefixes every
//! assertion message with [`ctx`] — the file, the release and the serial it
//! loaded — so a failure here can never again be attributed to the wrong build.
//!
//! `stories/` is gitignored (commercial media), so every case skips vacuously
//! per missing file, exactly like the other real-game smokes. The `ran > 0`
//! guards catch a LOCAL run whose filenames drifted; they are gated on
//! [`any_real_media_present`] so they cannot fire where the fixtures legitimately
//! cannot exist.

use std::path::{Path, PathBuf};

use app::engine::Engine;
use app::graphics::PictSource;
use app::hints::DiskImage;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

// ── The table ────────────────────────────────────────────────────────────────

/// One story image as it exists on one medium.
#[derive(Clone, Copy)]
struct Medium {
    /// The game, so a pair reads as a pair.
    title: &'static str,
    /// Filename under `stories/`.
    file: &'static str,
    /// The release disk image this medium is, if it is one rather than a bare
    /// story file — and which filesystem, because that is what says which
    /// machine's media it is.
    image: Option<DiskImage>,
    /// Z-machine version, header $00.
    version: u8,
    /// Release number, header $02.
    release: u16,
    /// Serial, header $12..$18.
    serial: &'static str,
}

/// Every medium in `stories/` worth pinning a build against, with the build each
/// one carries. **Measured**, 2026-08-10, extended 2026-08-13 for the DOS and
/// Atari ST presses and 2026-08-30 for the twenty-one media the guard below
/// found unnamed, by mounting each file through `app::hints::load_mounted_story`
/// and reading its header.
///
/// Read the pairs: four of the five titles that ship both ways ship *different
/// builds*, and the four v3/v5 pairs ship the same one. Then read the
/// Hitchhiker's trio at the bottom, which is the same lesson at three media and
/// two Z-machine versions.
const MEDIA: &[Medium] = &[
    // Journey — the pair that started this. Different builds, and they differ
    // in which window they narrate through (see `V6_FRAMES`).
    Medium { title: "Journey", file: "Journey - The Quest Begins.adf", image: Some(DiskImage::Adf), version: 6, release: 30, serial: "890322" },
    Medium { title: "Journey", file: "journey-r83-s890706.z6", image: None, version: 6, release: 83, serial: "890706" },
    // Zork Zero ships on THREE media here, and all three are different builds.
    // The Macintosh disk is the outlier by a mile: r296/881019 is October 1988,
    // where both others are 1989 (SQ-0837).
    Medium { title: "Zork Zero (Macintosh)", file: "Zork Zero Disk.image", image: Some(DiskImage::Hfs), version: 6, release: 296, serial: "881019" },
    // …and the *Masterpieces of Infocom* CD-ROM, which is the same Macintosh
    // build again on a 12 MB hybrid disc rather than an 800 KB floppy
    // (SQ-1158). It mounts as the HFS volume it is — `blorb::medium` reads the
    // Macintosh partition — and the story it opens with is Zork Zero
    // r296/881019, byte-identical in release and serial to the floppy above.
    // That agreement is the finding: the outlier build is the MACINTOSH's, not
    // that one disk's.
    Medium { title: "Zork Zero (Masterpieces CD)", file: "InfocomMasterpieces.img", image: Some(DiskImage::Hfs), version: 6, release: 296, serial: "881019" },
    // Zork Zero — different builds, and the floppy lays its story window out at
    // a different place and size.
    Medium { title: "Zork Zero", file: "Zork Zero - The Revenge of Megaboz.adf", image: Some(DiskImage::Adf), version: 6, release: 366, serial: "890323" },
    Medium { title: "Zork Zero", file: "zork0-r393-s890714.z6", image: None, version: 6, release: 393, serial: "890714" },
    // Shogun — different builds that (so far) lay out identically.
    Medium { title: "Shogun", file: "James Clavell's Shogun.adf", image: Some(DiskImage::Adf), version: 6, release: 295, serial: "890321" },
    Medium { title: "Shogun", file: "shogun-r322-s890706.z6", image: None, version: 6, release: 322, serial: "890706" },
    // **And the Macintosh press, which this table did not name for months while
    // the file played perfectly well** (SQ-1158). `Shogun.toast` is a bare HFS
    // volume — `BD` at offset `0x400`, volume name `Shogun` — that
    // `DiskImage::detect` reads by CONTENT, so a Toast CD's extension over an
    // ordinary Macintosh floppy was never evidence about anything. What it cost
    // is the reason the guard below exists: a lane consulted this table, found
    // no Macintosh Shogun, and reported "Shogun has no Macintosh press in the
    // corpus" as a fact about the world.
    //
    // A FOURTH Shogun build on a fifth medium, and the earliest of them:
    // r292/890314 against the Amiga's r295/890321, the Apple's r311/890510 and
    // the bare file's r322/890706.
    Medium { title: "Shogun (Macintosh)", file: "Shogun.toast", image: Some(DiskImage::Hfs), version: 6, release: 292, serial: "890314" },
    // Arthur — different builds that (so far) lay out identically.
    Medium { title: "Arthur", file: "Arthur - The Quest for Excalibur.adf", image: Some(DiskImage::Adf), version: 6, release: 54, serial: "890606" },
    Medium { title: "Arthur", file: "arthur-r74-s890714.z6", image: None, version: 6, release: 74, serial: "890714" },
    // Beyond Zork — the SAME build on both media.
    Medium { title: "Beyond Zork", file: "Beyond Zork - The Coconut of Quendor.adf", image: Some(DiskImage::Adf), version: 5, release: 57, serial: "871221" },
    Medium { title: "Beyond Zork", file: "beyondzork-r57-s871221.z5", image: None, version: 5, release: 57, serial: "871221" },
    // Sherlock on three media, and the two that pair carry the SAME build
    // (SQ-1158). The Macintosh floppy is a third print of it — and the DOS row
    // further down is a *different* build, r21/871214, so this title makes both
    // halves of the rule on one shelf.
    Medium { title: "Sherlock", file: "Sherlock - The Riddle of the Crown Jewels.adf", image: Some(DiskImage::Adf), version: 5, release: 26, serial: "880127" },
    Medium { title: "Sherlock", file: "sherlock-r26-s880127.z5", image: None, version: 5, release: 26, serial: "880127" },
    Medium { title: "Sherlock (Macintosh)", file: "Sherlock.img", image: Some(DiskImage::Hfs), version: 5, release: 26, serial: "880127" },
    // The Zork trilogy — the same build on both media.
    Medium { title: "Zork I", file: "Zork I - The Great Underground Empire.adf", image: Some(DiskImage::Adf), version: 3, release: 88, serial: "840726" },
    Medium { title: "Zork I", file: "zork1-r88-s840726.z3", image: None, version: 3, release: 88, serial: "840726" },
    Medium { title: "Zork II", file: "Zork II - The Wizard of Frobozz.adf", image: Some(DiskImage::Adf), version: 3, release: 48, serial: "840904" },
    Medium { title: "Zork II", file: "zork2-r48-s840904.z3", image: None, version: 3, release: 48, serial: "840904" },
    Medium { title: "Zork III", file: "Zork III - The Dungeon Master.adf", image: Some(DiskImage::Adf), version: 3, release: 17, serial: "840727" },
    Medium { title: "Zork III", file: "zork3-r17-s840727.z3", image: None, version: 3, release: 17, serial: "840727" },
    // Zork: The Undiscovered Underground ships on a floppy only.
    Medium { title: "ZTUU", file: "Zork - The Undiscovered Underground.adf", image: Some(DiskImage::Adf), version: 5, release: 16, serial: "970828" },
    // ── The PC and the Atari ST (SQ-0833, SQ-0835) ───────────────────────────
    //
    // Zork Zero's DOS press, on the disk its STORY is actually on: *The Lost
    // Treasures of Infocom* I, floppy5 — with its EGA art beside it, while its
    // CGA art sits on floppy4 and is unreachable from here. r393/890714, the
    // same build as the bare `zork0-r393-s890714.z6`, byte for byte.
    Medium { title: "Zork Zero (DOS)", file: "floppy5.ima", image: Some(DiskImage::Fat12Dos), version: 6, release: 393, serial: "890714" },
    // …and the SAME build off two standalone DOS pressings of it (SQ-1158): a
    // three-disk 360 KB set and a two-disk 720 KB one. Both reassemble to
    // r393/890714, the same build the collection disk above and the bare
    // `zork0-r393-s890714.z6` carry — three DOS images of one release, which is
    // what makes them a control on the FAT12 reader rather than a fourth build.
    // Only the disk that OPENS is pinned; the continuation volumes fold into it
    // through their own set, and the guard below checks that fold is a build
    // match and not merely a shared filename stem.
    Medium { title: "Zork Zero (DOS 360K set)", file: "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (360K) (Disk 1) [!].ima", image: Some(DiskImage::Fat12Dos), version: 6, release: 393, serial: "890714" },
    Medium { title: "Zork Zero (DOS 720K set)", file: "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (720K) (Disk 1) [!].ima", image: Some(DiskImage::Fat12Dos), version: 6, release: 393, serial: "890714" },
    // **Three media, three Hitchhiker's, two Z-machine versions.** This is the
    // project's "a disk image is a different release" rule at its most extreme,
    // and now it is pinned rather than asserted: the standalone DOS disk is v3
    // r58, the Lost Treasures collection ships the later Solid Gold v5 r31, and
    // the Atari ST press is v3 r56. A finding about "Hitchhiker's" that does not
    // name its medium describes none of them.
    Medium { title: "Hitchhiker's (DOS 360K)", file: "Hitchhiker's Guide to the Galaxy, The (1987) (r58, Serial 851002) (Infocom, Inc.) (360K) [!].ima", image: Some(DiskImage::Fat12Dos), version: 3, release: 58, serial: "851002" },
    Medium { title: "Hitchhiker's (Lost Treasures)", file: "floppy2.ima", image: Some(DiskImage::Fat12Dos), version: 5, release: 31, serial: "871119" },
    // …and the other three volumes of that collection, so all five `floppy*.ima`
    // are named rather than three of them (SQ-1158). Each opens with a different
    // game, which is what makes them five rows and not one: `floppy1.ima` Beyond
    // Zork v5 r57/871221 — the SAME build as the Amiga floppy, the Apple IIgs
    // volume and the bare `.z5`, four media agreeing — `floppy3.ima` The Lurking
    // Horror v3 r203/870506 and `floppy4.ima` Stationfall v3 r107/870430, both
    // the builds their Apple IIgs volumes carry.
    Medium { title: "Beyond Zork (Lost Treasures DOS)", file: "floppy1.ima", image: Some(DiskImage::Fat12Dos), version: 5, release: 57, serial: "871221" },
    Medium { title: "The Lurking Horror (Lost Treasures DOS)", file: "floppy3.ima", image: Some(DiskImage::Fat12Dos), version: 3, release: 203, serial: "870506" },
    Medium { title: "Stationfall (Lost Treasures DOS)", file: "floppy4.ima", image: Some(DiskImage::Fat12Dos), version: 3, release: 107, serial: "870430" },
    // **Four more DOS floppies, four more games** (SQ-1158). `disk1.img`…
    // `disk4.img` are 720 KB volumes labelled `DISK 1`…`DISK 4`, several stories
    // to a disk, so the row is the one that OPENS — the largest, exactly like
    // the Atari ST compilations below. (`disk2.img` also holds `PLUNDERE.DAT`;
    // `disk3.img` `BORDERZO`, `CUTTHROA` and `SEASTALK`; `disk4.img` `NORDANDB`,
    // `HOLLYWOO` and `WISHBRIN`. Ask the picker or `--story` for the rest —
    // between them the four carry the *Lost Treasures II* line-up, though
    // nothing on the disks says so and only the builds below are measured.)
    //
    // They earn their rows twice over. *Trinity* here is r12/860926 — the Apple
    // IIgs and Commodore 128 build, where `Infocom Compilation 8`'s ST press is
    // r11/860509 — and *Sherlock* is r21/871214 against the Amiga and Macintosh
    // floppies' r26/880127 above. Two titles, four media each, and the DOS press
    // siding with a different sibling in each case.
    Medium { title: "A Mind Forever Voyaging (DOS)", file: "disk1.img", image: Some(DiskImage::Fat12Dos), version: 4, release: 77, serial: "850814" },
    Medium { title: "Trinity (DOS)", file: "disk2.img", image: Some(DiskImage::Fat12Dos), version: 4, release: 12, serial: "860926" },
    Medium { title: "Bureaucracy (DOS)", file: "disk3.img", image: Some(DiskImage::Fat12Dos), version: 4, release: 116, serial: "870602" },
    Medium { title: "Sherlock (DOS)", file: "disk4.img", image: Some(DiskImage::Fat12Dos), version: 5, release: 21, serial: "871214" },
    // An Atari ST compilation: four games in four folders, every one of them
    // called `STORY.DAT`, so the conventional-name tiebreak cannot separate them
    // and the largest is what opening the disk gives you — *Bureaucracy* v4 r86.
    // (The other three are Hitchhiker's v3 r56 s841221, Cutthroats v3 r23
    // s840809 and Leather Goddesses v3 r59 s860730; `blorb::medium`'s own suite
    // pins the whole list.)
    Medium { title: "Bureaucracy (Atari ST)", file: "Infocom Compilation 9 (19xx)(-).st", image: Some(DiskImage::Fat12AtariSt), version: 4, release: 86, serial: "870212" },
    // The other directoried ST compilation, and the one that gets to PLAY below:
    // four folders again, four more `STORY.DAT`s, and the largest is *Trinity*.
    // (Compilation 9's Bureaucracy opens on its licence form and never reaches
    // an ordinary prompt, which makes it a fine identity pin and a poor smoke.)
    Medium { title: "Trinity (Atari ST)", file: "Infocom Compilation 8 (19xx)(-).st", image: Some(DiskImage::Fat12AtariSt), version: 4, release: 11, serial: "860509" },
    // **The one story in the ST corpus whose BEHAVIOUR the profile moves.**
    // Compilation 7 in the survey's numbering, flat 8.3 names, and `BEYZORK.T`
    // is by some distance the largest file on it (262144 bytes against ~85K for
    // each Zork), so opening the disk gives you *Beyond Zork* — which is what
    // makes it usable as a plain row here. Every other ST story either ignores
    // `$1E` entirely or merely prints it; this one changes what it draws and
    // what it asks. `atari_st_profile.rs` is where that is measured.
    Medium { title: "Beyond Zork (Atari ST)", file: "Infocom Compilation 6 (19xx)(-).st", image: Some(DiskImage::Fat12AtariSt), version: 5, release: 49, serial: "870917" },
    // **And the other compilations in the survey's run** (SQ-1158), because
    // three named volumes out of nine is exactly the absence this file's guard
    // exists to refuse. Each opens with a different game — the largest story on
    // the volume, as above — and every one of them is a build some other row
    // here already carries on other media: Stationfall and Bureaucracy off the
    // Apple IIgs and DOS collections, A Mind Forever Voyaging off `disk1.img`,
    // Border Zone and Spellbreaker off nothing else in the corpus at all.
    Medium { title: "Stationfall (Atari ST)", file: "Infocom Compilation 1 (19xx)(-).st", image: Some(DiskImage::Fat12AtariSt), version: 3, release: 107, serial: "870430" },
    Medium { title: "A Mind Forever Voyaging (Atari ST)", file: "Infocom Compilation 2 (19xx)(-).st", image: Some(DiskImage::Fat12AtariSt), version: 4, release: 77, serial: "850814" },
    Medium { title: "Border Zone (Atari ST)", file: "Infocom Compilation 3 (19xx)(-).st", image: Some(DiskImage::Fat12AtariSt), version: 5, release: 9, serial: "871008" },
    Medium { title: "Bureaucracy (Atari ST, Compilation 4)", file: "Infocom Compilation 4 (19xx)(-).st", image: Some(DiskImage::Fat12AtariSt), version: 4, release: 116, serial: "870602" },
    Medium { title: "Spellbreaker (Atari ST)", file: "Infocom Compilation 7 (19xx)(-).st", image: Some(DiskImage::Fat12AtariSt), version: 3, release: 87, serial: "860904" },
    // …and Compilation 5, which opens with the build Compilation 8 opens with —
    // Trinity r11/860509, the ST press that is six months older than every other
    // Trinity in the corpus. Two volumes of one survey carrying one build is
    // worth a row of its own rather than a fold, because the fold that would
    // otherwise cover it rests on the nine images sharing a filename stem, and a
    // shared stem is not a release.
    Medium { title: "Trinity (Atari ST, Compilation 5)", file: "Infocom Compilation 5 (19xx)(-).st", image: Some(DiskImage::Fat12AtariSt), version: 4, release: 11, serial: "860509" },
    // ── The Apple II (SQ-0836) ───────────────────────────────────────────────
    //
    // ProDOS media, and the fifth filesystem lanthorn mounts. Every row here was
    // measured through `app::hints::load_mounted_story` on 2026-08-13, like the
    // rest of the table.
    //
    // The standalone Apple IIgs *Beyond Zork* — a GS/OS boot disk with `BZ.DAT`
    // on it — is the SAME build as the Amiga floppy and the bare `.z5` beside
    // them, which makes it this corpus's clearest "sometimes they do agree".
    Medium { title: "Beyond Zork (Apple IIgs)", file: "Beyond Zork (1988)(Infocom).2mg", image: Some(DiskImage::ProDos), version: 5, release: 57, serial: "871221" },
    // *The Lost Treasures of Infocom* (1993, Big Red Computer Club) is seven
    // Apple IIgs volumes. Volume 1 is the GS/OS launcher and carries no game at
    // all, so it is absent here; volumes 2–7 carry thirty between them and
    // each row below is the one that OPENS — no ProDOS release has a
    // conventional story name, so the largest wins (`blorb::prodos` pins the
    // full inventory of all seven).
    Medium { title: "Beyond Zork (Lost Treasures IIgs)", file: "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 2 of 7).2mg", image: Some(DiskImage::ProDos), version: 5, release: 57, serial: "871221" },
    Medium { title: "Stationfall (Apple IIgs)", file: "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 3 of 7).2mg", image: Some(DiskImage::ProDos), version: 3, release: 107, serial: "870430" },
    Medium { title: "The Lurking Horror (Apple IIgs)", file: "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 4 of 7).2mg", image: Some(DiskImage::ProDos), version: 3, release: 203, serial: "870506" },
    // **The row that earns its place.** *Trinity* off the IIgs collection is v4
    // r12 s860926; *Trinity* off `Infocom Compilation 8 (19xx)(-).st`, four rows
    // up, is v4 r11 s860509. Same game, two media, two builds — the project's
    // own rule on a third machine, and now pinned rather than assumed.
    Medium { title: "Trinity (Apple IIgs)", file: "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 5 of 7).2mg", image: Some(DiskImage::ProDos), version: 4, release: 12, serial: "860926" },
    Medium { title: "Sherlock (Apple IIgs)", file: "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 6 of 7).2mg", image: Some(DiskImage::ProDos), version: 5, release: 21, serial: "871214" },
    Medium { title: "Wishbringer (Apple IIgs)", file: "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 7 of 7).2mg", image: Some(DiskImage::ProDos), version: 3, release: 69, serial: "850920" },
    // ── The packed Apple volume (SQ-0852) ────────────────────────────────────
    //
    // **The one row whose story is not a file.** *Arthur*'s Apple press pages
    // its story out of five opaque segments (`ARTHUR.1/ARTHUR.D1` …
    // `ARTHUR.5/ARTHUR.D5`) by 512-byte block; no file on the volume IS a story
    // and the reassembly is `blorb::infocom_packed`'s. That it lands here, in
    // the table every other medium is pinned in, is the point — a packed volume
    // is a medium like any other once it is read.
    //
    // r63/890622, a THIRD Arthur: the Amiga floppy is r54/890606 and the bare
    // file r74/890714, so the project's "a disk image is a different release"
    // rule holds on a fourth machine. Measured through
    // `app::hints::load_mounted_story` on 2026-08-14, like every row above.
    //
    // `Journey.2mg` still has no row, and now for a reason the corpus can prove
    // rather than assume: it declares five segments and carries four, so 92 of
    // its 552 pages are not on the image and `blorb::infocom_packed` refuses it
    // rather than handing back a truncated game. `blorb::prodos` pins that
    // refusal. The two Journey rows below are the SAME release off pressings
    // that are complete, which is what makes the `.2mg` a short image rather
    // than an unreadable format.
    Medium { title: "Arthur (Apple II, packed)", file: "Arthur Quest 4 Excalibur.2mg", image: Some(DiskImage::ProDos), version: 6, release: 63, serial: "890622" },
    // ── The Apple II 5.25-inch press (SQ-0864) ───────────────────────────────
    //
    // **The two rows whose story is on no single file.** *Shogun* was pressed
    // on five 5.25-inch floppies and *Zork Zero* on four, and each carries only
    // its own quarter or fifth of the game — `shogun_s1.dsk` alone mounts as
    // ProDOS volume `SHOGUN.1` and honestly reports that it holds no story.
    // What loads them is `blorb::medium::MountedDisk::mount_set`, which is
    // handed the other volumes of the release (named by `app::disk_set`) and
    // asks `blorb::infocom_packed` the question no one disk can answer.
    //
    // They are ProDOS rows like the `.2mg`s above, and that is the finding
    // rather than a shortcut: a `.dsk` is the same filesystem with its sectors
    // in the 5.25-inch drive's order (`blorb::dos_order`), so it wears the same
    // row and announces the same Apple IIgs.
    //
    // A FIFTH Shogun and a FOURTH Zork Zero: r311/890510 against the Amiga's
    // r295/890321 and the bare file's r322/890706; r383/890602 against the
    // Macintosh's r296/881019, the Amiga's r366/890323 and the bare file's
    // r393/890714. The project's "a disk image is a different release" rule has
    // no counterexample left worth looking for.
    //
    // Measured through `app::hints::load_mounted_story` on 2026-08-14, like
    // every row above, and each reassembly is checked against the story's own
    // ZMSD §11.1.6 header checksum — `$E200` and `$6F7F` — by the reader.
    Medium { title: "Shogun (Apple II 5.25)", file: "shogun_s1.dsk", image: Some(DiskImage::ProDos), version: 6, release: 311, serial: "890510" },
    Medium { title: "Zork Zero (Apple II 5.25)", file: "zork_zero_1.dsk", image: Some(DiskImage::ProDos), version: 6, release: 383, serial: "890602" },
    // ── The Apple II *Journey*, twice, and the short image beside them ───────
    //
    // **A SIXTH Journey**, and the release SQ-0867 read off `Journey.2mg`'s one
    // surviving header page without being able to load it: r77 / 890616, against
    // the Amiga floppy's r30 / 890322 and the bare file's r83 / 890706. Two
    // complete pressings of it are here, and they are two different IMAGES of
    // one build — the five-volume 5.25-inch set (`journey_s1.dsk`…`s5`, ProDOS
    // volumes `JOURNEY.1`…`JOURNEY.5`) and the 3.5-inch consolidated `Journey.po`
    // (volume `JOURNEY.3.5`, the same five segments in five subdirectories, which
    // is *Arthur*'s layout). Both reassemble to release 77 with header checksum
    // `$B136`.
    //
    // `Journey.po` is also the row that earns `.po` its place in
    // `blorb::medium`'s extension census (SQ-0863): a bare ProDOS volume has
    // always mounted, and until this file arrived nothing in `stories/` was one,
    // so the picker's pre-filter had never heard of the spelling and the image
    // was openable by name and invisible in the list.
    Medium { title: "Journey (Apple II 5.25)", file: "journey_s1.dsk", image: Some(DiskImage::ProDos), version: 6, release: 77, serial: "890616" },
    Medium { title: "Journey (Apple II 3.5)", file: "Journey.po", image: Some(DiskImage::ProDos), version: 6, release: 77, serial: "890616" },
    // The other two bare volumes, for the same reason. `Arthur.po` is the SAME
    // dump as the `.2mg` above down to the story bytes and the picture entries
    // (`apple_release_artwork.rs` pins that agreement, which is what makes it a
    // control on the reader rather than a fourteenth fixture); `ZorkZero.po` is
    // the 3.5-inch consolidation of the four `zork_zero_*.dsk` floppies.
    Medium { title: "Arthur (Apple II 3.5)", file: "Arthur.po", image: Some(DiskImage::ProDos), version: 6, release: 63, serial: "890622" },
    Medium { title: "Zork Zero (Apple II 3.5)", file: "ZorkZero.po", image: Some(DiskImage::ProDos), version: 6, release: 383, serial: "890602" },
    // …and the fourth `.po`, which is not a bare volume and was declined for two
    // quests on that ground (SQ-0889). `Shogun.po` is a **DiskCopy 4.2** image —
    // 84-byte header, 819,200-byte volume, 19,200 bytes of sector tags, summing
    // to its 838,484 exactly — wearing a ProDOS extension over an ordinary 800 KB
    // `SHOGUN` volume. `blorb::prodos` now tries that placement using
    // `blorb::hfs`'s unwrap rather than a second one, and what comes out is the
    // 3.5-inch consolidation of the five `shogun_s*.dsk` floppies: the same
    // release 311 / serial 890510, packed across `SHOGUN.D1`…`D5` on one disk.
    // Two rows for one build is the point — this is the control that says the
    // wrapper landed on the right bytes rather than on merely plausible ones.
    Medium { title: "Shogun (Apple II 3.5)", file: "Shogun.po", image: Some(DiskImage::ProDos), version: 6, release: 311, serial: "890510" },
    // ── The raw self-booting Apple II press (SQ-0868) ────────────────────────
    //
    // **The first non-v6 Apple disk in the corpus**, and the first medium here
    // with no filesystem on it at all. Every `.dsk` above is a ProDOS volume
    // whose sectors are in the drive's order; this one has no volume directory
    // in any order and no DOS 3.3 VTOC either — Infocom's loader boots and reads
    // the story off known tracks with its own RWTS. `blorb::infocom_boot` finds
    // it by putting the sectors into DOS 3.3 *logical* order and verifying a run
    // of them against the story's own ZMSD §11.1.6 checksum, `$842E`, which no
    // other order produces.
    //
    // It matters beyond one game. `zvm-cli` declines v6 by design, so until this
    // row every Apple format lanthorn read could be mounted and never *played*
    // through the CLI — Arthur, Journey, Shogun and Zork Zero are all v6. This
    // is a Version 3 game on Apple II media, so the whole path is exercised end
    // to end for the first time; the `NARRATED` entry below is that proof.
    //
    // Measured through `app::hints::load_mounted_story` on 2026-08-14, like
    // every row above.
    Medium { title: "Planetfall (Apple II, self-booting)", file: "Planetfall r29 (clean copy from retail disk).dsk", image: Some(DiskImage::InfocomBootDisk), version: 3, release: 29, serial: "840118" },
    // ── The Commodore 1541 press (SQ-0869) ───────────────────────────────────
    //
    // Two machines, two presses, two layouts, and a story that is on two disks.
    // `blorb::d64` has the measurement; what these rows pin is that all of it
    // arrives through `app::hints::load_mounted_story` like every medium above.
    //
    // *Hitchhiker's* is the 1984 Commodore 64 floppy — a BASIC `SYS(2063)` stub
    // on track 17 and a story written 16 sectors to a track from track 5. It is
    // a **fourth** Hitchhiker's: v3 r47 s840914 against the DOS 360K's v3 r58
    // s851002, the Lost Treasures v5 r31 s871119 and the Atari ST's v3 r56, so
    // the project's "a disk image is a different release" rule now holds across
    // four media for this one game.
    Medium { title: "Hitchhiker's (Commodore 64)", file: "Hitchhikers_Guide_to_the_Galaxy_The_1984_Infocom.d64", image: Some(DiskImage::CommodoreD64), version: 3, release: 47, serial: "840914" },
    // *Trinity* is the 1986 Commodore **128** press — a `CBM` autoboot sector
    // and an interpreter that touches the C128 MMU at `$FF00` forty times — and
    // it is the first medium in this table whose story is on **no single disk**
    // for an arithmetical reason rather than a packaging one: Version 4 counts
    // its length in fours (ZMSD §11.1.6), so 262,064 bytes cannot fit on a
    // 174,848-byte floppy. `MountedDisk::mount_set` joins the two sides and
    // verifies the join against the story's own checksum.
    //
    // **And it is the same build as two other rows** — v4 r12 s860926, exactly
    // the *Trinity (Apple IIgs)* row above, which for once makes a Commodore
    // finding transferable. (The Atari ST's *Trinity* is r11 s860509 and is
    // not.)
    //
    // Both sides are listed, because opening EITHER must give the whole game:
    // side 2 carries no header at all, so the set has to work backwards from a
    // volume that cannot identify itself. Measured through
    // `app::hints::load_mounted_story` on 2026-08-14, like every row above.
    Medium { title: "Trinity (Commodore 128)", file: "TRINITY1.D64", image: Some(DiskImage::CommodoreD64), version: 4, release: 12, serial: "860926" },
    Medium { title: "Trinity (Commodore 128, side 2)", file: "TRINITY2.D64", image: Some(DiskImage::CommodoreD64), version: 4, release: 12, serial: "860926" },
    // *Plundered Hearts* is the 1987 Commodore floppy as a **raw GCR
    // bitstream** — a `.g64` rather than a `.d64`, which is a nibble of the
    // physical track rather than a dump of its decoded sectors (SQ-1095). Forty
    // tracks, no half-tracks, and five tracks at the end that are not sectors at
    // all; the story is seventeen sectors to a track from track 5, a third
    // layout in three Commodore presses.
    //
    // **And it is the same build as the bare story file beside it** — v3 r26
    // s870730, byte-identical to `plunderedhearts-r26-s870730.z3` for all
    // 128,962 bytes, which is what makes this row an oracle for the GCR decoder
    // rather than one more thing that looks plausible. Pinned in `blorb::g64`.
    Medium { title: "Plundered Hearts (Commodore, GCR)", file: "plundered_hearts[infocom_1987](r26)(!).g64", image: Some(DiskImage::CommodoreG64), version: 3, release: 26, serial: "870730" },
    // …and the second GCR bitstream in the corpus, which is also this table's
    // plainest lesson about names (SQ-1158). The file is called
    // `zork_ii[infocom_1984](r88)(!).g64` and its header says release **48**,
    // serial 840904 — r88 is *Zork I*'s number, and the dump wears it anyway.
    // The build is the one the Amiga floppy and the bare `zork2-r48-s840904.z3`
    // carry, so the Commodore press agrees with both; nothing about that is
    // readable off the filename, which is the whole thesis of this file.
    Medium { title: "Zork II (Commodore, GCR)", file: "zork_ii[infocom_1984](r88)(!).g64", image: Some(DiskImage::CommodoreG64), version: 3, release: 48, serial: "840904" },
];

/// Recognised disk images in `stories/` that [`MEDIA`] deliberately does NOT
/// pin, each with the reason — filename, then why it is not a press worth a row.
///
/// This is the other exit from
/// [`every_disk_image_in_stories_is_pinned_or_explained`], and its whole point
/// is that the reason is WRITTEN DOWN. An absence from [`MEDIA`] says nothing;
/// a row here is a claim someone made and can be checked.
const UNPINNED: &[(&str, &str)] = &[
    (
        "Journey.2mg",
        "declares five segments and carries four, so 92 of its 552 pages are not on \
         the image and `blorb::infocom_packed` refuses it rather than handing back \
         four fifths of a game. There is no build to pin — its surviving header page \
         reads r77/890616, which is what `Journey.po` and `journey_s1.dsk` above \
         actually load. `blorb::prodos` pins the refusal.",
    ),
    (
        "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 1 of 7).2mg",
        "the GS/OS launcher of the seven-volume Apple IIgs collection: it carries no \
         game at all, so there is no release to pin. Volumes 2-7 each have a row of \
         their own above.",
    ),
];

/// The pairs, and whether the two media carry the SAME build. Every `false`
/// here is a title where a finding measured off one medium says nothing about
/// the other.
const PAIRS: &[(&str, bool)] = &[
    ("Journey", false),
    ("Zork Zero", false),
    ("Shogun", false),
    ("Arthur", false),
    ("Beyond Zork", true),
    ("Sherlock", true),
    ("Zork I", true),
    ("Zork II", true),
    ("Zork III", true),
];

/// The v6 frame each medium produces after `turns` blank turns from boot, under
/// the profile its own medium resolves to: which window its prose is streaming
/// into, and that window's box as the game set it (x, y, w, h — native pixels,
/// 1-based like the Z-machine's own coordinates).
///
/// The prose window is the load-bearing number: it is the fact SQ-0755 turned
/// on, and the one place where Journey's two releases visibly disagree.
struct V6Frame {
    file: &'static str,
    turns: usize,
    prose_window: usize,
    box_px: (u16, u16, u16, u16),
}

const V6_FRAMES: &[V6Frame] = &[
    // r30 narrates through window 2 — window 0 is a leftover strip it never
    // prints into. r83 narrates through window 0 and has no window 2 at all.
    V6Frame { file: "Journey - The Quest Begins.adf", turns: 40, prose_window: 2, box_px: (265, 17, 368, 272) },
    V6Frame { file: "journey-r83-s890706.z6", turns: 40, prose_window: 0, box_px: (241, 1, 392, 304) },
    // Zork Zero's two builds place the story window two pixels apart and size it
    // four pixels differently — small, and enough to make a geometry finding off
    // one medium untrue of the other.
    V6Frame { file: "Zork Zero - The Revenge of Megaboz.adf", turns: 12, prose_window: 0, box_px: (89, 81, 464, 320) },
    V6Frame { file: "zork0-r393-s890714.z6", turns: 12, prose_window: 0, box_px: (87, 79, 468, 320) },
    // …and the Macintosh disk's r296, on the big colour Mac's 640×400 SCREEN —
    // the same screen r393 gets, because the Mac's COLOUR archive is the Amiga's
    // picture space (`wx := 2*GFXAM_X`). Only the monochrome archive is a
    // different screen (SQ-0838), pinned in `v6_macintosh_profile.rs`.
    //
    // **But the box is NOT the same, and this pin used to say it was** (SQ-1021).
    // A same-screen machine still lays windows out on its own CELL, and the
    // Macintosh's is 7x15 where every other machine's is 8x16 (SQ-0917). Window 0
    // is 21 Macintosh lines: 21 * 15 = **315**. The old pin read 320, which is
    // 20 * 16 — a whole number of lines of a cell this machine does not have, and
    // not a multiple of 15 at all (320/15 = 21.33), so no Macintosh could ever
    // produce it. It was a fabricated frame: this harness booted the Mac at 8x16
    // because it passed `None` for the cell, exactly as SQ-0901's Arthur frame was
    // fabricated by omitting `native_std_window`. The width does not move, because
    // the game places this window's left edge and width in pixels rather than in
    // columns — which is why only one number here changed.
    V6Frame { file: "Zork Zero Disk.image", turns: 12, prose_window: 0, box_px: (87, 79, 468, 315) },
    // Shogun and Arthur agree across their two builds. Pinned so that stays a
    // measured fact rather than an assumption.
    V6Frame { file: "James Clavell's Shogun.adf", turns: 12, prose_window: 0, box_px: (47, 33, 548, 368) },
    V6Frame { file: "shogun-r322-s890706.z6", turns: 12, prose_window: 0, box_px: (47, 33, 548, 368) },
    V6Frame { file: "Arthur - The Quest for Excalibur.adf", turns: 12, prose_window: 0, box_px: (1, 1, 640, 400) },
    V6Frame { file: "arthur-r74-s890714.z6", turns: 12, prose_window: 0, box_px: (1, 1, 640, 400) },
    // …and Arthur's PACKED Apple press, reassembled out of five segments. This
    // row is the real-game smoke for `blorb::infocom_packed`: a story whose
    // header checksum is exact could still be a story the app cannot drive, and
    // twelve turns of a reassembled r63 reaching a laid-out window is the answer
    // to that (SQ-0852).
    //
    // **560×384, and the move is the finding** (SQ-0863). This row read
    // (1, 1, 640, 400) for as long as the Apple's artwork was unreadable: with
    // nothing declaring a picture space the screen fell back to the profile's,
    // and 640×400 is what an ARTLESS v6 launch gets. The archive now speaks, and
    // an archive outranks a profile — the same order that lays Zork Zero out on
    // 480×300 off the standard Macintosh's monochrome plate (SQ-0838), and
    // `reset.rs`'s own chain: Blorb `Reso`, then the archive, then the machine.
    //
    // The space is 140×192 and the scale is (4, 2), both off the machine rather
    // than off what fits. `apple/yzip/rel.15/apple.equ` states the space in the
    // same breath as the dots it is counted in — `MAXWIDTH EQU 140 ; 560 / 4 =
    // max "pixels"` and `MAXHEIGHT EQU 192 ; 192 screen lines` — so one Apple
    // picture pixel is four double-hi-res dots wide and one scan line tall, and
    // 140×192 art covers all 560×192 dots the screen has. The vertical 2 is what
    // a scan line MEASURES rather than what it counts: the Apple's 192 active
    // lines fill the visible raster of a 4:3 monitor while its 560 dots fill the
    // width, so a line is (3/4)·(560/192) = 2.19 dots tall and the shape-
    // preserving vertical factor is 2. `PictSource::art_scale` carries the whole
    // derivation. 560×384 is also exactly 70×24 whole 8×16 cells.
    //
    // Arthur's Apple press is a THIRD build beside r54 and r74 and is entitled
    // to its own screen, exactly as Journey r30 narrates through a different
    // window than r83 (SQ-0755). Nothing else in this table moves: every other
    // medium's picture space is what it always was.
    V6Frame { file: "Arthur Quest 4 Excalibur.2mg", turns: 12, prose_window: 0, box_px: (1, 1, 560, 384) },
    // The 5.25-inch presses, on the same Apple screen and for the same reason
    // (SQ-0863). Their artwork is on no single floppy either — `SGTPICOF` names
    // an archive on five of Shogun's segments, four of Journey's and two of Zork
    // Zero's — so these rows are the smoke for `MountedDisk::pictures`'s
    // set-spanning arm as much as for the geometry: a release that draws nothing
    // lays out on the profile's 640×400 and would fail here.
    V6Frame { file: "shogun_s1.dsk", turns: 12, prose_window: 0, box_px: (1, 65, 560, 320) },
    V6Frame { file: "zork_zero_1.dsk", turns: 12, prose_window: 0, box_px: (1, 1, 560, 384) },
    // …and *Journey* r77, which is the row this table exists for. Its two
    // siblings disagree about which window they narrate through — r30 uses
    // window 2 and r83 window 0, the whole of SQ-0755 — so a third build of the
    // same game is exactly the case where measuring one medium proves nothing
    // about another. Measured: r77 sides with r83 and narrates through window 0,
    // in a 304×288 box parked at the right of the Apple's 560×384 screen.
    V6Frame { file: "journey_s1.dsk", turns: 40, prose_window: 0, box_px: (249, 1, 304, 288) },
];

/// Media whose game narrates its own release, and the words it uses. The
/// strongest possible statement of which build is running: the STORY says so,
/// not the header we read to pick the fixture.
const NARRATED: &[(&str, &[&str])] = &[
    ("Zork Zero - The Revenge of Megaboz.adf", &["Release 366 / Serial number 890323"]),
    ("zork0-r393-s890714.z6", &["Release 393 / Pix 393 / Serial number 890714"]),
    // The Macintosh disk goes one better than ZTUU: Zork Zero r296 does not
    // print the NUMBER, it prints the MACHINE. "Macintosh Interpreter version
    // 6.65" is the game's own reading of header `$1E`, and it says Macintosh
    // only because SQ-0838 told it 3 — the same disk answered "IBM Interpreter"
    // for as long as an HFS volume resolved to the IBM PC default.
    ("Zork Zero Disk.image", &["Macintosh Interpreter version", "Release 296 / Serial number 881019"]),
    ("James Clavell's Shogun.adf", &["Release 295 / Serial number 890321"]),
    ("shogun-r322-s890706.z6", &["Release 322 / Pix 322 / Serial number 890706"]),
    ("Zork I - The Great Underground Empire.adf", &["Revision 88 / Serial number 840726"]),
    ("zork1-r88-s840726.z3", &["Revision 88 / Serial number 840726"]),
    ("Zork II - The Wizard of Frobozz.adf", &["Version 48 / Serial number 840904"]),
    ("Zork III - The Dungeon Master.adf", &["Release 17 / Serial number 840727"]),
    // …and ZTUU prints the interpreter it was told it is running on, which off a
    // floppy is the Amiga's 4 (ZMSD §11.1.3). One line proving both halves of
    // "the medium picks the machine".
    ("Zork - The Undiscovered Underground.adf", &["Release 16 / Serial number 970828", "Interpreter 4 "]),
    // **The PC and the Atari ST open and play** (SQ-0833, SQ-0835). Mounting is
    // half a claim; this is the other half — the game boots off the floppy,
    // reaches its prompt, and names the release the disk carries. Zork Zero's
    // DOS press draws its own EGA art off the same disk on the way.
    ("floppy5.ima", &["Release 393 / Pix 393 / Serial number 890714"]),
    // Trinity off the ST floppy prints its interpreter number too, and **this is
    // the line SQ-0835 left here to be changed.** It read `Interpreter 1 ` for
    // one commit, while the container read an ST floppy and then ran it as a
    // DECSystem-20; it now reads 5, ZMSD §11.1.3's Atari ST, which is also what
    // Infocom's own ST interpreters write into `$1E` (`INTWRD DC.B 5`).
    //
    // Trinity is a Version 4 story, which is the interesting half: byte `$1E`
    // means nothing before Version 4, and the ST's own Version 3 build leaves it
    // zero and comments it "(UNUSED)". So this is a story that genuinely reads
    // the byte, and it reports the machine it is now correctly told it is on.
    ("Infocom Compilation 8 (19xx)(-).st", &["Release 11 / Serial Number 860509", "Interpreter 5 "]),
    // **And the Apple II press opens and plays** (SQ-0836). Mounting a ProDOS
    // volume, walking its directory tree and pulling a story out of a tree file
    // is half a claim; this is the other half — the game boots off the disk
    // image, reaches its prompt, and names the release the disk carries.
    //
    // Two of them, because the interesting pair is *Trinity*: the line below is
    // r12 s860926 off the IIgs collection, where the Atari ST row above prints
    // r11 s860509 for the same game. Same title, two floppies, two builds, and
    // both are now the story's own word rather than a header we read.
    ("Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 5 of 7).2mg", &["Release 12 / Serial Number 860926"]),
    ("Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 7 of 7).2mg", &["Release 69 / Serial Number 850920"]),
    // **And the raw self-booting press opens and plays** (SQ-0868) — the line
    // that closes the gap this quest was filed for.
    //
    // The two lines above are ProDOS volumes, and every ProDOS disk in the
    // corpus that is *not* a compilation is a Version 6 game, which `zvm-cli`
    // declines by design. So "lanthorn reads Apple II media" had, until this
    // row, never once meant "and plays a game off one through the CLI". This is
    // a Version 3 story on Apple II media: it boots, prints its banner, and
    // names release 29 / serial 840118 — the story's own word for the build that
    // came off 426 sectors nothing but the interleave and the checksum located.
    ("Planetfall r29 (clean copy from retail disk).dsk", &["Release 29 / Serial number 840118"]),
    // **And the Commodore press opens and plays** (SQ-0869), on both machines
    // and — the part that is new to this table — off a game that is on **two**
    // floppies.
    //
    // *Hitchhiker's* is the single-disk Commodore 64 press: it boots off 440
    // sectors that nothing but the 16-per-track plan and the checksum located,
    // and names release 47 / serial 840914.
    ("Hitchhikers_Guide_to_the_Galaxy_The_1984_Infocom.d64", &["Release 47 / Serial number 840914"]),
    // *Trinity* is the pair, and these two lines are the whole quest in one
    // assertion each. The story is 262,064 bytes and neither floppy holds
    // 174,849 of them, so **every character of this banner came off both disks**
    // — 344 sectors from side 1 and 680 from side 2, joined and checked against
    // the story's own `$16AB`.
    //
    // `Interpreter 7 ` is the second half. Read it against the Atari ST line
    // above, which SQ-0835 left here to be changed for exactly this reason:
    // Trinity is Version 4, so it genuinely reads `$1E`, and it now reports the
    // Commodore 128 rather than the DECSystem-20 the fall-through would have
    // told it it was. The same game, three media, three answers — 5 off the ST,
    // 7 off the Commodore, and the IIgs line above prints no number at all
    // because that row asks for the release and not the machine.
    ("TRINITY1.D64", &["Release 12 / Serial Number 860926", "Interpreter 7 "]),
    // …and opening the OTHER side gives the same game, which is the property a
    // set exists for. Side 2 has no Z-machine header anywhere on it: it cannot
    // say what game it is, what release, or even that it is Infocom. It is
    // joined to side 1 by the checksum and by nothing else.
    ("TRINITY2.D64", &["Release 12 / Serial Number 860926", "Interpreter 7 "]),
];

/// The resource Blorbs shipped beside these stories, and whether each declares
/// a `Reso` standard window. None of them carries an executable chunk, so a
/// Blorb is never a third build — it is artwork for whichever story file you
/// point at, and the release stays the one you opened.
const RESOURCE_BLORBS: &[(&str, bool)] = &[
    ("Journey.blb", true),
    ("Zork0.blb", true),
    ("Shogun.blb", true),
    ("Arthur.blb", true),
    // Beyond Zork is a v5 story: its Blorb carries sound, no scalable pictures
    // and so no standard window at all.
    ("beyondzork.blb", false),
];

/// Pane widths the render case sweeps. 80 is the game's own column count, where
/// the cell path's 1:1 chrome and the proportional placement coincide and a
/// defect can hide (SQ-0742's lesson); the other two do not coincide.
const WIDTHS: [u16; 3] = [80, 100, 138];

// ── Fixtures ─────────────────────────────────────────────────────────────────

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Is ANY of this suite's media present? `stories/` is gitignored, so CI and a
/// fresh worktree legitimately have none and every case here skips. This gates
/// the `ran > 0` guards so they only ever catch a drifted filename on a machine
/// that really does hold the media.
fn any_real_media_present() -> bool {
    MEDIA.iter().any(|m| stories_dir().join(m.file).exists())
}

/// How every assertion in this file names its subject. A failure that does not
/// say which release it loaded is the single most expensive kind of failure this
/// project has had.
fn ctx(m: &Medium) -> String {
    format!(
        "{} [{} — {}, release {}, serial {}]",
        m.file,
        m.title,
        match m.image {
            Some(DiskImage::Adf) => "Amiga floppy",
            Some(DiskImage::Hfs) => "Macintosh floppy",
            Some(DiskImage::Fat12Dos) => "DOS floppy",
            Some(DiskImage::Fat12AtariSt) => "Atari ST floppy",
            Some(DiskImage::ProDos) => "Apple ProDOS floppy",
            Some(DiskImage::InfocomBootDisk) => "Apple self-booting floppy",
            Some(DiskImage::CommodoreD64) => "Commodore 1541 floppy",
            Some(DiskImage::CommodoreG64) => "Commodore 1541 floppy (GCR bitstream)",
            Some(DiskImage::Iso9660) => "ISO 9660 CD-ROM",
            None => "story file",
        },
        m.release,
        m.serial
    )
}

/// The story bytes off `m`'s medium, or `None` (with a SKIP note) when the
/// gitignored fixture is not there.
fn story_bytes(m: &Medium) -> Option<Vec<u8>> {
    let path = stories_dir().join(m.file);
    match app::hints::load_mounted_story(&path) {
        Ok((loaded, mounted)) => {
            assert_eq!(
                mounted,
                m.image,
                "{}: the mount reports the medium, and it disagrees with the table",
                ctx(m)
            );
            Some(loaded.bytes().to_vec())
        }
        Err(_) => {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            None
        }
    }
}

fn header_release(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[2], bytes[3]])
}

fn header_serial(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[0x12..0x18]).into_owned()
}

/// The guard every case runs before it measures anything: the bytes in hand are
/// the build the table names. Nothing measured after this can be attributed to
/// the wrong release.
fn assert_is_the_pinned_release(m: &Medium, bytes: &[u8]) {
    assert_eq!(bytes[0], m.version, "{}: Z-machine version", ctx(m));
    assert_eq!(
        header_release(bytes),
        m.release,
        "{}: loaded release {} — this medium carries a DIFFERENT build than the table says",
        ctx(m),
        header_release(bytes)
    );
    assert_eq!(
        header_serial(bytes),
        m.serial,
        "{}: loaded serial {}",
        ctx(m),
        header_serial(bytes)
    );
}

/// Boot `m` exactly as `startup.rs` does — the profile comes from the medium,
/// the artwork from whatever that medium supplies — after checking the build.
fn boot(m: &Medium, honor_game_colours: bool) -> Option<GameSession> {
    let bytes = story_bytes(m)?;
    assert_is_the_pinned_release(m, &bytes);
    let path = stories_dir().join(m.file);
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    // The same chain, in the same order: the Blorb's `Reso`, the archive the
    // medium supplied, then the machine (SQ-0838). No medium here names an
    // archive by hand, so tier 3's link is the one that is absent.
    // SQ-1021/SQ-1022: every per-machine fact in one value, so this
    // harness cannot omit one — it was omitting the CELL.
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    let mut s = GameSession::new_for_machine(bytes, honor_game_colours, false, false, picture_dims, None, None, &boot)
    .unwrap_or_else(|e| panic!("{}: should boot without a ZError: {e:?}", ctx(m)));
    assert!(!s.quit, "{}: quit during boot", ctx(m));
    assert!(s.machine.fault_trace.is_none(), "{}: faulted during boot", ctx(m));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

/// Drive `turns` turns of "just keep going": Enter at a keypress, an empty line
/// at a line prompt. Deterministic, which is what a pinned frame needs.
fn drive(s: &mut GameSession, turns: usize, who: &str) {
    for t in 0..turns {
        match s.pending_input() {
            InputKind::Line => {
                let _ = s.submit("");
            }
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
        assert!(!s.quit, "{who}: quit at turn {t} of {turns}");
        assert!(s.machine.fault_trace.is_none(), "{who}: faulted at turn {t} of {turns}");
    }
}

/// The window this session's prose is currently streaming into, and its box as
/// the game set it. `None` for a non-v6 story, or when nothing has been printed.
fn prose_window(s: &GameSession) -> Option<(usize, (u16, u16, u16, u16))> {
    let v6 = s.machine.screen.v6.as_ref()?;
    v6.windows
        .iter()
        .enumerate()
        .find(|(_, w)| !w.streamed.is_empty())
        .map(|(i, w)| (i, (w.x_coord, w.y_coord, w.x_size, w.y_size)))
}

/// Which render path drew this frame, as `/dump-windows` reports it.
fn path_label(state: &app::state::AppState) -> String {
    state
        .v6_cell_map
        .borrow()
        .iter()
        .find(|e| e.label.starts_with("path:"))
        .map(|e| e.label.clone())
        .unwrap_or_else(|| "<no path recorded>".into())
}

/// The native-pixel rect the renderer treated as the STORY viewport this frame.
fn viewport_px(state: &app::state::AppState) -> Option<(u16, u16, u16, u16)> {
    state.v6_cell_map.borrow().iter().find(|e| e.label == "viewport").map(|e| e.native)
}

// ── Identity ─────────────────────────────────────────────────────────────────

/// The table itself, checked against the media. This is the finding SQ-0760
/// exists to record: what each medium in `stories/` actually contains.
#[test]
fn every_medium_loads_the_release_it_is_pinned_at() {
    let mut ran = 0;
    for m in MEDIA {
        let Some(bytes) = story_bytes(m) else { continue };
        assert_is_the_pinned_release(m, &bytes);
        ran += 1;
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but none were read");
}

/// **The guard, and the durable half of SQ-1158.** Every disk image in
/// `stories/` is either pinned in [`MEDIA`] or explained in [`UNPINNED`] —
/// nothing is allowed to be merely absent.
///
/// `Shogun.toast` sat unpinned here for months: a bare HFS volume (`BD` at
/// offset 0x400, volume name `Shogun`) that `DiskImage::detect` reads by
/// CONTENT, so its Toast-CD extension never mattered and it mounted and played
/// perfectly well. The SQ-1152 lane consulted this table, found no Macintosh
/// Shogun in it, and reasoned from that ABSENCE to "Shogun has no Macintosh
/// press in the corpus" — then built a capture decision on the premise and
/// reported it as fact. An absence in a hand-maintained table is
/// indistinguishable from a fact about the world, which is exactly CLAUDE.md's
/// "a guard beats a convention".
///
/// So this asks `DiskImage::detect` of every file in the directory and fails on
/// any recognised medium the table has never heard of. Two exits, both of them
/// something a person wrote down: a row in [`MEDIA`], or a row in [`UNPINNED`]
/// saying why the file is not a press worth pinning.
///
/// `stories/` is gitignored, so this skips vacuously when the directory is
/// absent (CI has no media at all) and still fails loudly wherever it exists.
///
/// FALSIFICATION: delete the `Shogun.toast` row from [`MEDIA`] and this fails
/// with `Shogun.toast — Hfs, v6 release 292 serial 890314` — the defect as
/// reported, and the release the lane could not find.
///
/// And *Shogun* was not alone, which is the argument for a guard rather than one
/// row: it named **twenty-one** further media on its first runs, every one of
/// them mountable and none of them here. Six Atari ST compilations, nine DOS
/// volumes (three more of *Lost Treasures*, four 720 KB floppies carrying the
/// second collection, and two standalone *Zork Zero* pressings), a Macintosh
/// *Sherlock* floppy and the *Masterpieces* CD, *Sherlock*'s Amiga floppy, a
/// Commodore GCR bitstream, and two ProDOS volumes that carry no build to pin
/// and now say so in [`UNPINNED`].
#[test]
fn every_disk_image_in_stories_is_pinned_or_explained() {
    let dir = stories_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("SKIP: gitignored story directory missing at {}", dir.display());
        return;
    };
    let mut files: Vec<PathBuf> =
        entries.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    files.sort();

    let named = |n: &str| MEDIA.iter().any(|m| m.file == n) || UNPINNED.iter().any(|u| u.0 == n);

    let mut detected = 0;
    let mut unexplained: Vec<String> = Vec::new();
    for path in &files {
        let Ok(raw) = std::fs::read(path) else { continue };
        let Some(image) = DiskImage::detect(&raw) else { continue };
        detected += 1;
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if named(&name) {
            continue;
        }
        let loaded = app::hints::load_mounted_story(path).ok().map(|(l, _)| l.into_bytes());
        // A later volume of a multi-disk release folds into the row its set is
        // pinned on, exactly as the picker folds a set into one row — but only
        // when the story that comes off it is the SAME BUILD as that row. A
        // shared filename stem is not a shared release: `disk1.img`…`disk4.img`
        // are one set by that measure and four different games.
        if let Some(bytes) = loaded.as_deref().filter(|b| b.len() > 0x18) {
            let folded = app::disk_set::members(path).unwrap_or_default().iter().any(|v| {
                v != path
                    && v.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                        MEDIA.iter().any(|m| {
                            m.file == n
                                && m.version == bytes[0]
                                && m.release == header_release(bytes)
                                && m.serial == header_serial(bytes)
                        })
                    })
            });
            if folded {
                continue;
            }
        }
        // Unpinned. Say what it is, so filling the row in needs no second run.
        let what = match loaded.as_deref() {
            Some(b) if b.len() > 0x18 => {
                format!("v{} release {} serial {}", b[0], header_release(b), header_serial(b))
            }
            Some(_) => "a story too short to identify".to_string(),
            None => "no story comes out of it".to_string(),
        };
        unexplained.push(format!("  {name} — {image:?}, {what}"));
    }

    assert!(
        unexplained.is_empty(),
        "{} disk image(s) in stories/ are recognised media that MEDIA does not name \
         and UNPINNED does not explain. Pin each one (mount it and read its own \
         header — never copy a sibling medium's release), or record why it is not \
         a press:\n{}",
        unexplained.len(),
        unexplained.join("\n")
    );
    assert!(
        detected > 0 || !any_real_media_present(),
        "media are present but detect() recognised none of them"
    );
}

/// The headline. For every title shipping on both media, the two releases are
/// pinned against **each other** — so a future agent cannot quietly measure one
/// and report it as the other, and an upgrade of either fixture fails here
/// rather than silently rebasing an investigation.
#[test]
fn a_floppy_and_the_story_file_beside_it_are_pinned_against_each_other() {
    let mut ran = 0;
    for (title, same_build) in PAIRS {
        let pair: Vec<&Medium> = MEDIA.iter().filter(|m| m.title == *title).collect();
        assert_eq!(pair.len(), 2, "{title}: PAIRS names a title that is not a pair in MEDIA");
        let (floppy, file) = (pair[0], pair[1]);
        assert!(
            floppy.image.is_some() && file.image.is_none(),
            "{title}: MEDIA must list the floppy first"
        );
        let (Some(fb), Some(sb)) = (story_bytes(floppy), story_bytes(file)) else { continue };
        assert_is_the_pinned_release(floppy, &fb);
        assert_is_the_pinned_release(file, &sb);
        ran += 1;

        let same = header_release(&fb) == header_release(&sb)
            && header_serial(&fb) == header_serial(&sb);
        assert_eq!(
            same, *same_build,
            "{title}: the floppy is release {} serial {} and the story file is release {} serial {} \
             — {}. A finding measured on one medium describes the other ONLY when these agree.",
            header_release(&fb),
            header_serial(&fb),
            header_release(&sb),
            header_serial(&sb),
            if *same_build { "they were pinned as the same build" } else { "they were pinned as different builds" },
        );
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no pair was compared");
}

/// **Every medium here is one the story picker actually OFFERS** (SQ-0849).
///
/// The report: *"i don't see any games with ima or st extensions in the story
/// list."* They were not there. Mounting worked — every case above proves it —
/// but the picker's directory scan pre-filtered on its own hand-written
/// extension list, which knew `.adf` and `.image` and had never heard of the DOS
/// and Atari ST rows SQ-0833 and SQ-0835 added. Those floppies were opened by
/// name and invisible in the list, which is the only place most people look.
///
/// So this drives `picker::scan_stories` over the real story directory and
/// insists that every medium in [`MEDIA`] that exists on disk comes back out of
/// it. The pre-filter now derives from `blorb::medium`'s format table, so a
/// format added there is offered here in the same commit.
///
/// `stories/` is gitignored, so this skips vacuously per missing file and the
/// `ran > 0` guard is gated on [`any_real_media_present`], exactly like its
/// neighbours — CI has no media at all and must not fail on their absence.
///
/// Measured on the local corpus, 2026-08-13: the scan listed **142** stories
/// before and **163** after — 8 `.ima`, 4 `.img` and 9 `.st` that were mountable
/// all along. The `.adf` count (9) and the `.image` count (1) did not move, and
/// three `.ima` in the directory are still *not* listed, because no story comes
/// out of them: the extension is a pre-filter, not a verdict.
///
/// FALSIFICATION: restore the disk spellings as a literal `"adf", "image"` in
/// `picker::STORY_EXTS` and this fails naming `floppy5.ima` — the user's symptom,
/// verbatim.
///
/// **The one exemption** (SQ-0844): a volume every build of which an *earlier
/// volume of its own multi-disk release* already offers contributes no rows, and
/// that is the point of folding a set together rather than listing its disks.
/// `Infocom Compilation 8` is exactly that — its Lurking Horror, Moonmist,
/// Stationfall and Trinity are the same four builds as `Compilation 5`'s and
/// `Compilation 1`'s, down to the IFID — so it is offered as nothing and every
/// game on it is still one keypress away. The exemption is granted per story and
/// only against the same *build*, so it cannot hide a game: an image whose
/// stories are not all offered elsewhere still has to be in the list, which is
/// what keeps the falsification above honest (`floppy5.ima`'s Zork Zero r393 is
/// on no other volume of the `floppy*` set).
#[test]
fn every_release_medium_is_offered_by_the_story_picker() {
    let dir = stories_dir();
    let data_base = std::env::temp_dir().join(format!("lanthorn-sq0849-{}", std::process::id()));
    let rows = app::picker::scan_stories(&dir, &data_base);
    let listed: Vec<PathBuf> = rows.iter().map(|e| e.path.clone()).collect();
    let _ = std::fs::remove_dir_all(&data_base);

    /// Every story on `path`, as the mount itself reports them.
    ///
    /// Through `app::hints::mounted_stories`, which is the seam the picker uses
    /// — so a release whose story is on no single volume (SQ-0864's 5.25-inch
    /// presses) is counted here exactly as the browser counts it, and this
    /// helper cannot quietly disagree with the list it is checking.
    fn stories_on(path: &Path) -> Vec<Vec<u8>> {
        app::hints::mounted_stories(path)
            .map(|(_, s)| s.into_iter().map(|(s, _)| s.bytes).collect())
            .unwrap_or_default()
    }

    let mut ran = 0;
    for m in MEDIA {
        let path = dir.join(m.file);
        if !path.is_file() {
            continue;
        }
        ran += 1;
        if listed.contains(&path) {
            continue;
        }
        // Not listed: allowed only when this is a volume of a set and every
        // build on it is offered by a *sibling volume of that same set*.
        let siblings = app::disk_set::members(&path).unwrap_or_default();
        let on_disk = stories_on(&path);
        let all_folded = !on_disk.is_empty()
            && on_disk.iter().all(|b| {
                let ifid = app::ifid::compute_ifid(b);
                rows.iter().any(|e| e.meta.ifid == ifid && siblings.contains(&e.path))
            });
        assert!(
            all_folded,
            "{} is mountable but the picker never offered it, and its games are \
             not all offered by another volume of its own release",
            ctx(m),
        );
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but none were scanned");

    // **And `.dsk` crossed the same line in SQ-0864.** It sat in the paragraph
    // below as the format with no reader, listed nowhere however many of them
    // were in the directory. Fourteen are, and they are three RELEASES rather
    // than fourteen disks — so what the browser must show is exactly three rows,
    // one per game, reached by naming the first volume of each. Five Shogun
    // floppies that each report the same reassembled build are five rows before
    // `dedupe_within_sets` and one after (SQ-0844), which is the set model
    // paying for itself.
    //
    // **Four rows since SQ-0868**, and the fourth is one disk rather than a set.
    // `Planetfall r29 (clean copy from retail disk).dsk` is a RAW self-booting
    // disk — no filesystem, a second format wearing the same `.dsk` spelling —
    // and it appears here having required no change to the picker, the scan or
    // the set model: the extension census is a union read off `blorb::medium`,
    // so the row was listed the day the reader landed. That is this assertion's
    // real subject, which is why it is a name list and not a count.
    //
    // It is also the set model's negative case (SQ-0844): its stem carries a
    // digit run (`r29`) and it is still one row and not a set of one, because no
    // sibling shares that stem. A set that folded it into Shogun's or Zork
    // Zero's would show up here as a missing name.
    let listed_dsk: Vec<&std::ffi::OsStr> = listed
        .iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("dsk"))
                == Some(true)
        })
        .filter_map(|p| p.file_name())
        .collect();
    let dir_has_dsk = std::fs::read_dir(&dir).into_iter().flatten().flatten().any(|e| {
        e.path().extension().and_then(|x| x.to_str()).map(|x| x.eq_ignore_ascii_case("dsk"))
            == Some(true)
    });
    if dir_has_dsk {
        assert_eq!(
            listed_dsk,
            [
                "journey_s1.dsk",
                "Planetfall r29 (clean copy from retail disk).dsk",
                "shogun_s1.dsk",
                "zork_zero_1.dsk",
            ],
            "four releases, one row each, and the lowest disk number keeps it"
        );
    }

    // …and `.2mg` moved from that list to this one in the same commit as its
    // reader (SQ-0836), which is the whole point of the extension column.
    //
    // The two segmented Apple presses were BOTH absent from the picker until
    // SQ-0852, because no file on either volume is a story. One of them is here
    // now and the other still is not, and the difference is the finding: *Arthur*
    // reassembles out of its five segments and is offered like any other medium,
    // while `Journey.2mg` declares five segments and carries four, so 92 of its
    // 552 pages are not on the image and `blorb::infocom_packed` refuses it
    // rather than offering four fifths of a game. The extension is a pre-filter,
    // not a verdict — and neither is the mount's silence.
    let dir_has_2mg = std::fs::read_dir(&dir).into_iter().flatten().flatten().any(|e| {
        e.path().extension().and_then(|x| x.to_str()).map(|x| x.eq_ignore_ascii_case("2mg"))
            == Some(true)
    });
    let listed_2mg: Vec<&std::ffi::OsStr> = listed
        .iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("2mg"))
                == Some(true)
        })
        .filter_map(|p| p.file_name())
        .collect();
    assert_eq!(
        !listed_2mg.is_empty(),
        dir_has_2mg,
        "ProDOS media are in the directory and the picker offered none of them"
    );
    let named = |n: &str| listed_2mg.iter().any(|l| *l == std::ffi::OsStr::new(n));
    if dir_has_2mg && stories_dir().join("Arthur Quest 4 Excalibur.2mg").exists() {
        assert!(
            named("Arthur Quest 4 Excalibur.2mg"),
            "the packed Apple volume reassembles into a story and must be offered as one"
        );
    }
    assert!(
        !named("Journey.2mg"),
        "Journey.2mg is missing its fifth segment, so no whole story comes out of it \
         and it must not be offered as if one did"
    );

    // …and `.po` crossed the line in SQ-0863, which is the same paragraph a
    // third time and the reason this case is written as a sweep. `Journey.po` is
    // a BARE ProDOS volume — no 2IMG wrapper — and this reader has opened one
    // since SQ-0836, so the image mounted, listed its story and offered its
    // artwork the moment it appeared in the directory, and was still invisible
    // in the list, because `.po` was in no format row's extensions. One spelling
    // added; nothing else moved.
    let po = stories_dir().join("Journey.po");
    if po.is_file() {
        assert!(
            listed.contains(&po),
            "a bare ProDOS volume mounts, so the picker must offer it: {listed:?}"
        );
    }
}

/// A `.blb` beside a story is artwork, never a third build — so "which release"
/// is decided entirely by the file you open.
#[test]
fn a_resource_blorb_beside_a_story_carries_no_release_of_its_own() {
    let mut ran = 0;
    for (name, declares_std_window) in RESOURCE_BLORBS {
        let path = stories_dir().join(name);
        if !path.exists() {
            eprintln!("SKIP: gitignored resource Blorb missing at {}", path.display());
            continue;
        }
        ran += 1;
        assert!(
            app::hints::load_story(&path).is_err(),
            "{name}: a resource Blorb must hold no executable, or it would be a third build"
        );
        let std_window =
            PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b)).std_window();
        assert_eq!(
            std_window.is_some(),
            *declares_std_window,
            "{name}: standard window is {std_window:?}"
        );
        if *declares_std_window {
            // …and it is the SAME 320×200 the Amiga profile supplies for a
            // native archive, which has no field to declare one (SQ-0736).
            assert_eq!(
                std_window,
                InterpreterProfile::Amiga.std_window(),
                "{name}: an Infocom Blorb's Reso declares the machine's own standard window"
            );
        }
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no Blorb was read");
}

/// The medium picks the machine, on the real files rather than a synthetic disk
/// (`interpreter_profile.rs` covers the synthetic side). This is the rule that
/// makes "the Amiga build" ambiguous in the first place, so it is pinned on the
/// same fixtures every other case here uses.
#[test]
fn the_medium_each_release_ships_on_picks_the_interpreter_profile() {
    let mut ran = 0;
    for m in MEDIA {
        let path = stories_dir().join(m.file);
        if !path.exists() {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            continue;
        }
        ran += 1;
        // SQ-0838 lifted the Macintosh's block: an HFS volume now resolves to
        // the Macintosh bundle (interpreter 3, black on white, a screen sized by
        // the archive it carries), all of it read out of Infocom's own Mac
        // interpreter rather than out of anyone's memory. Everything that is not
        // release media is still the IBM PC default.
        let expected = match m.image {
            Some(DiskImage::Adf) => InterpreterProfile::Amiga,
            Some(DiskImage::Hfs) => InterpreterProfile::Macintosh,
            // A DOS floppy IS the IBM PC, and the IBM PC bundle is the one that
            // deliberately announces no number of its own (SQ-0833) — so this
            // arm and the fall-through below reach the same profile by design
            // rather than by accident.
            Some(DiskImage::Fat12Dos) => InterpreterProfile::IbmPc,
            // **And the Atari ST is now its own machine** (SQ-0835's profile
            // half): interpreter 5, black on white, §8.3.1's palette, and no
            // standard window, because Infocom never wrote a Version 6
            // interpreter for the ST. Read this arm against the one above it —
            // same FAT12 filesystem, different machine, different answer.
            Some(DiskImage::Fat12AtariSt) => InterpreterProfile::AtariSt,
            // **And a ProDOS volume IS its own machine after all** (SQ-0857,
            // reversing SQ-0836 — this is the one row this quest deliberately
            // moved). ProDOS still names the Apple II *family*, and §11.1.3 still
            // numbers three machines in it (2 IIe, 9 IIc, 10 IIgs) — Infocom's
            // own Apple II YZIP picks between all three by DETECTING the machine
            // at boot, and that routine is byte-for-byte on `Journey.2mg` and
            // `Arthur Quest 4 Excalibur.2mg`. What reversed the conclusion is
            // that declining is not neutral: it lands an Apple II story on zvm's
            // Frotz rule, which names the DECSystem-20 or, on Version 6, the IBM
            // PC. §11.1.3 asks for "the machine it will run on", and of the three
            // the YZIP starts on, that is the IIgs. Argued in full at the row in
            // `blorb::medium`, with the thirty-story trace measurement behind it.
            Some(DiskImage::ProDos) => InterpreterProfile::AppleIIgs,
            // **The same machine, off a disk with no filesystem** (SQ-0868).
            // Read this arm against the one above it the way the two FAT12 arms
            // read against each other — except that here the answer is the SAME,
            // and that is the finding. §11.1.3's question is which machine the
            // interpreter runs on, not which filesystem the disk has, so two
            // Apple II rows disagreeing would be saying the number is a property
            // of the medium; SQ-0857 disproved exactly that out of Infocom's own
            // YZIP. Nothing observable rides on it on this disk either way —
            // `$1E` means nothing before Version 4 and *Planetfall* is v3 — which
            // is why the row is argued as consistency rather than as a claim.
            Some(DiskImage::InfocomBootDisk) => InterpreterProfile::AppleIIgs,
            // **A third machine that names a family, and the first told apart
            // ON the disk** (SQ-0869). `.d64` is a 1541 image and ZMSD §11.1.3
            // numbers two Commodores in it — 7 the 128, 8 the 64 — but unlike
            // ProDOS the corpus can say which each disk is: `TRINITY1.D64` opens
            // with the C128 `CBM` autoboot sector, `Hitchhikers_…d64` with a C64
            // BASIC `SYS(2063)` stub. The row answers 7 because the C64 press is
            // Version 3 and `$1E` means nothing before Version 4, so the only
            // Commodore story here that READS the byte is on the C128 disk.
            //
            // The profile behind it is deliberately the thinnest in the enum:
            // the number, and an explicit decline on the standard window, the
            // palette and the default colour pair, none of which anything here
            // establishes. Argued at `InterpreterProfile::Commodore128`.
            Some(DiskImage::CommodoreD64) => InterpreterProfile::Commodore128,
            // **The same floppy in a different container** (SQ-1095), and the
            // same answer for the reason the two Apple II arms above give: a
            // `.g64` and a `.d64` are one medium dumped two ways, and §11.1.3
            // asks which MACHINE the interpreter runs on. Nothing observable
            // rides on it here either — *Plundered Hearts* is Version 3 and
            // `$1E` means nothing before Version 4 — so this is consistency
            // with its neighbour rather than a fresh claim about the press.
            Some(DiskImage::CommodoreG64) => InterpreterProfile::Commodore128,
            // **The one row that states no machine because the MEDIUM has
            // none** (SQ-0871), which is a different thing from the IBM PC's
            // deliberate decline above it. A CD-ROM carries both machines'
            // builds in one filesystem, so a number stated by the row would be
            // wrong for half the disc; the machine is a per-FILE question the
            // Apple ISO 9660 extension answers, and a file it cannot speak for
            // leaves the rule already in force. This arm is what a caller with
            // no story in hand gets, and the IBM PC default is the right thing
            // for it to be.
            Some(DiskImage::Iso9660) => InterpreterProfile::IbmPc,
            None => InterpreterProfile::IbmPc,
        };
        assert_eq!(
            InterpreterProfile::resolve(&path, None, None, None),
            expected,
            "{}: the medium decides the profile",
            ctx(m)
        );
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no profile was resolved");
}

// ── What each medium actually does ───────────────────────────────────────────

/// Every v6 medium boots and reaches a stable frame, and that frame is pinned
/// per medium — in BOTH `honor_game_colours` modes, per the project's colour
/// convention (`true` is the shipped default).
///
/// Journey is the case with teeth: the floppy's r30 streams its prose into
/// window 2 and the story file's r83 into window 0.
#[test]
fn each_v6_medium_narrates_through_the_window_its_own_release_uses() {
    let mut ran = 0;
    for f in V6_FRAMES {
        let m = MEDIA.iter().find(|m| m.file == f.file).expect("frame names a medium in MEDIA");
        for honor in [true, false] {
            let Some(mut s) = boot(m, honor) else { continue };
            let who = format!("{} honor_game_colours={honor}", ctx(m));
            drive(&mut s, f.turns, &who);
            let found = prose_window(&s)
                .unwrap_or_else(|| panic!("{who}: nothing was streamed after {} turns", f.turns));
            assert_eq!(
                found.0, f.prose_window,
                "{who}: this release narrates through window {}, not {} — a rule aimed at the \
                 wrong window is right by luck on one release and wrong on the other (SQ-0755)",
                found.0, f.prose_window
            );
            assert_eq!(
                found.1, f.box_px,
                "{who}: window {} is at {:?}, pinned at {:?} (native px, 1-based)",
                f.prose_window, found.1, f.box_px
            );
            ran += 1;
        }
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no v6 frame was measured");
}

/// The games that print their own release print the one that was loaded — the
/// story's word, not ours. Off the ZTUU floppy that line also names interpreter
/// 4, so the medium's profile is visible in the game's own output.
#[test]
fn a_game_that_names_its_release_names_the_one_the_medium_carries() {
    let mut ran = 0;
    for (file, wanted) in NARRATED {
        let m = MEDIA.iter().find(|m| m.file == *file).expect("NARRATED names a medium in MEDIA");
        let Some(mut s) = boot(m, true) else { continue };
        ran += 1;
        // Past whatever intro the release opens with, to its first line prompt.
        let mut said = String::new();
        for _ in 0..24 {
            match s.pending_input() {
                InputKind::Line => {
                    said = s.submit("version").transcript;
                    break;
                }
                InputKind::Char => {
                    let _ = s.submit_char(13);
                }
                InputKind::Event => {
                    let _ = s.submit("");
                }
            }
        }
        let flat = said.split_whitespace().collect::<Vec<_>>().join(" ");
        for want in *wanted {
            assert!(
                flat.contains(want),
                "{}: the game answered VERSION with {flat:?}, which does not say {want:?}",
                ctx(m)
            );
        }
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no game was asked");
}

/// A copy of a release floppy in a directory of its own, deleted when it drops.
///
/// **The isolation is the point, not tidiness.** `stories/` already holds loose
/// `zork0.eg1`, `zork0.cg1` and `zork0.mg1` fixtures that came off these very
/// disks, and `PictureOverride` tries the host filesystem before the medium — so
/// a test that names `ZORK0.EG1` beside them proves nothing about the medium at
/// all, and on a case-insensitive filesystem does not even fail loudly. It
/// passed under a deliberately broken mount before this struct existed.
struct FloppyAlone {
    dir: PathBuf,
    image: PathBuf,
}

impl Drop for FloppyAlone {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn floppy_alone(tag: &str) -> Option<FloppyAlone> {
    let src = stories_dir().join("floppy5.ima");
    if !src.is_file() {
        eprintln!("SKIP: gitignored medium missing at {}", src.display());
        return None;
    }
    let dir = app::scratch_dir(tag);
    let image = dir.join("floppy5.ima");
    std::fs::copy(&src, &image).expect("the floppy is copied");
    Some(FloppyAlone { dir, image })
}

/// A release floppy supplies its own artwork, on **every** machine that pressed
/// one — and the PC's is a different codec from the Amiga's and the Mac's
/// (SQ-0833).
///
/// This is the pairing the medium guarantees and no configuration has to: open
/// `floppy5.ima` and *Zork Zero*'s story and its EGA art come off the same disk,
/// through the same mount, with nothing named by hand. `ZORK0.EG1` is the
/// LZW-coded PC archive where the Amiga and Macintosh floppies hand back the
/// big-endian Huffman one — `blorb::fat12` pins the flavour, and this pins that
/// the app reaches it and can draw from it.
///
/// It is also where the one-image limit is visible: this disk carries the EGA
/// rendition and *Zork Zero*'s CGA rendition is on floppy4, which this mount
/// cannot reach. A set model would; see the module docs of `blorb::fat12`.
#[test]
fn a_dos_release_floppy_supplies_its_own_pc_artwork() {
    let Some(disk) = floppy_alone("dos-art") else { return };
    let mut picts = PictSource::resolve(&disk.image, None);
    let dims = picts.all_pict_dims();
    assert!(dims.len() > 100, "the disk's own archive, {} pictures", dims.len());
    assert!(
        picts.image(1).is_some(),
        "picture 1 decodes straight off the floppy, with no extraction step"
    );
}

/// **Offered and openable, which are two claims** (SQ-0833 + SQ-0843).
///
/// The launch dialog enumerates a disk's files through `assets::files`, which
/// asks `blorb::medium` and therefore gained FAT12 for free; the `--pictures`
/// door then loads the chosen one through `PictureOverride`. Those were two
/// different code paths, and the second one still carried a hand-written
/// `looks_like_adf … else if looks_like_hfs` chain — so a DOS disk's `ZORK0.EG1`
/// would have appeared in the list and drawn nothing when picked.
///
/// FALSIFICATION: restore that chain in `graphics::read_off_the_medium` and this
/// fails at the `Loaded` assertion with `Missing` — the file the same disk had
/// just listed.
#[test]
fn artwork_on_a_dos_floppy_is_both_listed_and_loadable() {
    let Some(disk) = floppy_alone("dos-listed") else { return };
    let listed: Vec<String> = app::assets::files(&disk.image)
        .into_iter()
        .filter(app::assets::AssetFile::is_on_medium)
        .map(|f| f.name)
        .collect();
    assert_eq!(listed, ["ZORK0.EG1", "ZORK0.ZIP"], "the disk's own files, in disk order");

    // Every name the dialog can show has to be a name the door accepts.
    let over =
        app::graphics::PictureOverride::resolve_with_session(&disk.image, &disk.dir, Some("ZORK0.EG1"));
    assert!(
        matches!(over, app::graphics::PictureOverride::Loaded { .. }),
        "naming the disk's own archive loads it: {over:?}"
    );
    assert_eq!(
        over.flavour(),
        Some(blorb::infocom_pics::Flavour::Pc),
        "…and it is the PC's LZW archive, so the machine stays the IBM PC"
    );
}

/// …and the other half of the rule, on the same real floppy: an explicitly
/// requested interpreter number BEATS the medium (SQ-0839).
///
/// The ordering is a contract, not an implementation detail — the medium only
/// ever moves the DEFAULT — and it is worth proving where the game can be heard
/// saying so rather than only at `resolve`. ZTUU's Inform banner prints header
/// `$1E` outright, so asking for the IBM PC's 6 off an Amiga floppy is audible.
/// `zvm-cli`'s `disk_image` suite pins the identical pair through the CLI.
///
/// FALSIFICATION: reorder `InterpreterProfile::resolve` to consult the medium
/// before the explicit number and the game answers `Interpreter 4`, ignoring
/// what was asked for.
#[test]
fn an_explicit_interpreter_number_outranks_the_floppy_it_was_opened_from() {
    const ZTUU: &str = "Zork - The Undiscovered Underground.adf";
    let m = MEDIA.iter().find(|m| m.file == ZTUU).expect("ZTUU is in MEDIA");
    let Some(bytes) = story_bytes(m) else { return };
    assert_is_the_pinned_release(m, &bytes);

    // The medium says Amiga; the launch says IBM PC. The launch wins, and it
    // brings its whole profile with it — that is what asking for a number means.
    let path = stories_dir().join(m.file);
    let profile = InterpreterProfile::resolve(&path, Some(6), None, None);
    assert_eq!(profile, InterpreterProfile::IbmPc, "{}: explicit beats the medium", ctx(m));

    let mut s = GameSession::new_with_trace(
        bytes,
        true,
        false,
        // The IBM PC profile has no opinion, so the launch's own number is what
        // reaches the header — exactly as `startup.rs` composes it.
        profile.interpreter_number().or(Some(6)),
        false,
        Vec::new(),
        profile.std_window(),
        profile.default_colours(),
        None,
    )
    .unwrap_or_else(|e| panic!("{}: should boot without a ZError: {e:?}", ctx(m)));
    // SQ-1393: the machine's own colour table. `new_with_trace` is the
    // no-machine door and presents §8.3.1's own, so a harness that boots a
    // press states the press's table here.
    s.machine.set_palette(profile.palette());

    let mut said = String::new();
    for _ in 0..24 {
        match s.pending_input() {
            InputKind::Line => {
                said = s.submit("version").transcript;
                break;
            }
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
    }
    let flat = said.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("Interpreter 6 "),
        "{}: asked for interpreter 6 off an Amiga floppy, and the game answered {flat:?}",
        ctx(m)
    );
}

/// …and the floppy build reaches the RENDERER, which is where its differently
/// placed windows could be dropped on the floor. Nothing committed had ever
/// rendered a frame produced by a disk image at all.
///
/// Two things per medium, at three pane widths and in both colour modes: the
/// frame takes the ordinary gameplay path (a release whose chrome trips the
/// painted-menu gate would be routed elsewhere — SQ-0742), and the story
/// viewport the renderer picks out is the box of the window THIS release
/// narrates through. On the Journey floppy that is window 2; on the story file
/// beside it, window 0.
#[test]
fn each_v6_medium_renders_the_story_viewport_its_own_release_lays_out() {
    let mut ran = 0;
    for f in V6_FRAMES {
        let m = MEDIA.iter().find(|m| m.file == f.file).expect("frame names a medium in MEDIA");
        for honor in [true, false] {
            let Some(mut s) = boot(m, honor) else { continue };
            let who = format!("{} honor_game_colours={honor}", ctx(m));
            drive(&mut s, f.turns, &who);
            let model = s.screen();
            // The window table is 1-based (ZMSD §8.8.1); the renderer works in
            // 0-based screen pixels, so the story viewport is that box shifted.
            let want = (f.box_px.0 - 1, f.box_px.1 - 1, f.box_px.2, f.box_px.3);
            for cols in WIDTHS {
                let state = render_hybrid(&model, honor, cols, 51);
                assert_eq!(
                    path_label(&state),
                    "path:hybrid-ring",
                    "{who} w={cols}: an ordinary gameplay frame must take the chrome ring"
                );
                assert_eq!(
                    viewport_px(&state),
                    Some(want),
                    "{who} w={cols}: the story viewport must be window {}'s box",
                    f.prose_window
                );
                ran += 1;
            }
        }
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no frame was rendered");
}

/// A hybrid render at a plausible kitty cell (8×18); `Picker::halfblocks()`
/// reports a 1×2 cell, a layout regime that reproduces nothing (SQ-0548).
#[allow(deprecated)]
fn render_hybrid(
    model: &app::engine::ScreenModel,
    honor: bool,
    cols: u16,
    rows: u16,
) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    state
}
