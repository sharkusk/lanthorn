//! SQ-1037/SQ-1036 — the machine's own system typeface, off a boot disk the
//! player supplied, and the order that ranks it against the release's own.
//!
//! # What the machine did
//!
//! `mac/xzip.lst` names two faces and uses both: `ZSTD: TextFont (stdFont)` with
//! `stdFont := geneva`, and `ZMONO: TextFont (monaco)`. One screen shows them
//! together — `machine-screenshots/mac-zorkzero-game.png`, *Zork Zero* release
//! **296 / serial 881019** off `stories/Zork Zero Disk.image`, the standard
//! Macintosh monochrome press, twelve turns in:
//!
//! * the status bar's `Banquet Hall` steps a **uniform 7 px** per character —
//!   ink runs begin at image x = 70, 77, 84, 91, 98, 105, 112, then `H` at 126
//!   after the blank, which is 8 characters at exactly 7;
//! * the prose two lines below advances **7, 7, 5** for `n`, `o`, `t` and runs
//!   `nother frantic day at the castle; Lord Dimwit` from x = 169 to x = 448 —
//!   **280 px of ink** for 44 characters, where any 7-wide pen gives 308;
//! * consecutive prose baselines are **15 rows** apart (y = 136, 151, 166, 181,
//!   196, 211), which is the `lineHeight := 15` the listing declares.
//!
//! *Zork Zero* brackets that bar in `@set_font 4` / `@set_font 1` and never
//! touches the style word, so the fixed-pitch bit is how the two halves are told
//! apart — `zvm` folds font 4 into §8.7.1's bit 3 and everything downstream asks
//! one question.
//!
//! # Fixtures
//!
//! `unit_tests/sysfont.hfs` is a synthetic Mac OS System volume, committed with
//! its generator, carrying the three discriminators a real System disk carries:
//! a face of the RIGHT HEIGHT in the WRONG FAMILY (`FONT` 12), the right family
//! at the WRONG HEIGHT (`FONT` 394), and the one the machine drew with (`FONT`
//! 396). Nothing here reads `~/.lanthorn/`: a case that did would pass or fail
//! on what the person running it happens to own.
//!
//! `unit_tests/relfont.hfs` is its sibling and plays the RELEASE medium — an
//! application carrying the fixed-pitch `FONT` 524, which is a `FaceFit::Cell`
//! face and therefore the machine's ALTERNATE. (`unit_tests/macfont.hfs`, the
//! older fixture, cannot stand in: its `FONT` 524 carries SQ-0916's deliberately
//! narrow `D`, so it reads as a typeface — the opposite of the real resource.)
//!
//! The real-media cases below drive `stories/Zork Zero Disk.image`, which is
//! gitignored, and skip vacuously without it.

use app::interpreter::InterpreterProfile as P;
use app::native_font::{FaceRequest, FaceSet, TextFace};
use app::system_fonts::UserDisks;
use std::path::{Path, PathBuf};

fn unit_tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../unit_tests")
}

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// A scratch directory of our own, standing in for `~/.lanthorn/`.
struct Disks {
    dir: PathBuf,
}

impl Disks {
    fn new(tag: &str) -> Disks {
        Disks { dir: app::scratch_dir(&format!("sq1037-{tag}")) }
    }

    /// Copy one of the committed fixtures in under `name`.
    fn with(self, name: &str, fixture: &str) -> Disks {
        let bytes = std::fs::read(unit_tests_dir().join(fixture))
            .unwrap_or_else(|e| panic!("unit_tests/{fixture} is committed and readable: {e}"));
        std::fs::write(self.dir.join(name), bytes).expect("write fixture");
        self
    }

    /// Write arbitrary bytes in under `name` — for the hostile-input case.
    fn with_bytes(self, name: &str, bytes: &[u8]) -> Disks {
        std::fs::write(self.dir.join(name), bytes).expect("write");
        self
    }

    fn disks(&self, prefer: Option<&str>) -> UserDisks {
        UserDisks { dir: self.dir.clone(), prefer: prefer.map(str::to_string) }
    }
}

impl Drop for Disks {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The cascade, asked about a story on `story_path`.
fn cascade(
    story_path: &Path,
    profile: P,
    art_scale: Option<(u32, u32)>,
    disks: Option<&UserDisks>,
) -> FaceSet {
    app::native_font::resolve(&FaceRequest {
        story_path,
        entry: None,
        profile,
        source: app::interpreter::ProfileSource::Medium,
        art_scale,
        disks,
    })
}

// ── the order ───────────────────────────────────────────────────────────────

/// **The order, stated as an outcome.** A release face on the story's medium, a
/// system face off the player's disk, and neither — three launches, three
/// answers, one function deciding all of them.
///
/// The Macintosh is the machine where both rungs answer at once and they land in
/// DIFFERENT roles: the release's `FONT` 524 IS the 7x15 cell and becomes the
/// fixed-pitch alternate, the System disk's Geneva 12 is a typeface and becomes
/// the body face. That is `mac/xzip.lst`'s own division of labour, not a rule
/// invented here.
#[test]
fn the_release_face_the_system_face_and_neither() {
    let release = unit_tests_dir().join("relfont.hfs");
    let boot = Disks::new("order").with("System.img", "sysfont.hfs");

    // Rung 1 alone: the release medium, no boot disk in sight.
    let alone = cascade(&release, P::Macintosh, None, None);
    assert_eq!(
        alone.body().map(|f| (f.width, f.height)),
        Some((7, 15)),
        "with no system disk the release's own fixed face draws everything, exactly as before",
    );
    assert_eq!(alone.body(), alone.fixed(), "…in both roles, which is what 'as before' means");
    assert_eq!(alone.body_origin(), Some(&app::native_font::FaceOrigin::Release));

    // Rungs 1 and 2: the system disk supplies the BODY and displaces nothing.
    let both = cascade(&release, P::Macintosh, None, Some(&boot.disks(None)));
    assert_eq!(
        both.body().map(|f| (f.width, f.height)),
        Some((9, 15)),
        "the System disk's proportional face is the body face",
    );
    assert!(both.body().is_some_and(|f| f.proportional), "and it is a TYPEFACE, not a cell");
    assert_eq!(
        both.fixed().map(|f| (f.width, f.height)),
        Some((7, 15)),
        "while the release's `FONT` 524 keeps the fixed-pitch role it is drawn for",
    );
    match both.body_origin() {
        Some(app::native_font::FaceOrigin::SystemDisk { disk, name }) => {
            assert_eq!(disk, "System.img", "the disk that answered is reported");
            assert_eq!(name, "FONT 396", "and so is the face — Geneva at 12pt, family 3");
        }
        other => panic!("the body face must name the disk it came off: {other:?}"),
    }

    // Rung 3: nothing at all. A path that is not a medium reaches no face, and the
    // renderer stays on the built-in.
    let nothing = cascade(&unit_tests_dir().join("README.md"), P::Macintosh, None, None);
    assert_eq!(nothing, FaceSet::none(), "no medium, no face — `vga16` answers as it always did");
}

/// A machine that names no system face reads a boot disk and takes nothing off
/// it, whatever the disk carries.
///
/// The IBM PC is every bare `.z6` a player opens, and a Macintosh System disk
/// sitting in `~/.lanthorn/` must not be able to change what one of those looks
/// like.
#[test]
fn a_machine_that_names_no_system_face_takes_nothing() {
    let boot = Disks::new("nameless").with("System.img", "sysfont.hfs");
    for profile in [P::IbmPc, P::AtariSt, P::AppleIIgs] {
        assert_eq!(profile.v6_system_face(), None, "{profile:?} names no system face");
        assert!(
            app::system_fonts::named_faces_in(&boot.disks(None), profile).is_empty(),
            "{profile:?}: and therefore reads nothing off a disk that has one",
        );
    }
}

// ── which face, out of a family of sizes ────────────────────────────────────

/// **The family is the machine's; the SIZE is the declared line height's.**
///
/// A System disk carries a whole family — Geneva at 9, 10, 12, 14, 18, 20 and 24
/// point on `MacOS_6.0.8_System_Startup.img` — and exactly one of them is what the
/// interpreter painted. `mac/xzip.lst` says which by declaring `lineHeight := 15`,
/// and `machine-screenshots/mac-zorkzero-game.png` measures the same 15 between
/// consecutive prose baselines.
///
/// The fixture holds both ways of getting this wrong, which is why it holds three
/// faces rather than one.
#[test]
fn the_declared_line_height_picks_the_size_and_the_family_picks_the_face() {
    let boot = Disks::new("size").with("System.img", "sysfont.hfs");

    // Non-vacuity: all three really are on the disk and really do parse.
    let all = app::system_fonts::scan(&boot.dir);
    let names: Vec<&str> = all.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["FONT 12", "FONT 394", "FONT 396"], "the fixture carries all three");

    // The FAMILY filter drops `FONT` 12 — family 0, and fifteen rows tall, so
    // height alone would have admitted it.
    let named = app::system_fonts::named_faces_in(&boot.disks(None), P::Macintosh);
    let named_names: Vec<&str> = named.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(named_names, ["FONT 394", "FONT 396"], "only family 3 is Geneva's");
    assert_eq!(
        all.iter().find(|f| f.name == "FONT 12").map(|f| f.height),
        Some(15),
        "non-vacuity: the rejected face IS the declared height, so the FAMILY is what rejected it",
    );

    // And the HEIGHT rule drops `FONT` 394 — right family, ten point, twelve rows.
    let faces = cascade(&unit_tests_dir().join("relfont.hfs"), P::Macintosh, None, Some(&boot.disks(None)));
    assert_eq!(
        faces.body().map(|f| (f.width, f.height)),
        Some((9, 15)),
        "the size the machine declared a line for is the size it drew with",
    );
}

// ── SQ-1039's trap: the TEXT scale, on the colour press ─────────────────────

/// **The declared cell does not move, on EITHER Macintosh press** (SQ-1039).
///
/// This is the trap the whole quest was filed against. Geneva 12 is fifteen rows
/// and the Macintosh cell is 7x15, so admitting it must leave `$26`/`$27` exactly
/// where they were — and it does only because the face is scaled by the TEXT
/// scale. The colour press draws `CPic.data` at 320x200 with an art scale of
/// (2, 2) while painting text at one native pixel per face pixel; scaling by the
/// ARTWORK there declares fifteen rows as thirty and lays the game out on a grid
/// half as tall as the machine's.
///
/// The monochrome press is `Pic.data` at 480x300, art scale (1, 1), and cannot
/// falsify any of this — 15 x 1 is 15 whichever number you used. So the colour
/// press is the case, and the mono one is here to show the difference is real.
#[test]
fn geneva_leaves_the_declared_cell_alone_on_both_macintosh_presses() {
    let boot = Disks::new("scale").with("System.img", "sysfont.hfs");
    let release = unit_tests_dir().join("relfont.hfs");

    for (press, art_scale) in [("monochrome Pic.data", (1u32, 1u32)), ("colour CPic.data", (2, 2))] {
        let faces = cascade(&release, P::Macintosh, Some(art_scale), Some(&boot.disks(None)));
        assert_eq!(
            faces.body().map(|f| (f.width, f.height)),
            Some((9, 15)),
            "{press}: non-vacuity — the fifteen-row SYSTEM face is the body, not the 7-wide \
             alternate, so the cell below is being asked about a face that could move it",
        );
        let cell = app::native_font::declared_cell(P::Macintosh, &faces, art_scale);
        assert_eq!(
            cell,
            zvm::interpreter::MACINTOSH_V6_CELL,
            "{press}: the story is told 7x15, which is what `mac/xzip.lst` declares",
        );
        let tf = TextFace::new(P::Macintosh, faces, Some(art_scale));
        assert_eq!(tf.scale(), (1, 1), "{press}: text is one native pixel per face pixel");
    }

    // And the Amiga, whose RELEASE face IS in the picture space, genuinely does
    // scale — the contrast that makes the assertion above a claim rather than a
    // constant. Its SYSTEM face is a third answer again (SQ-1053), pinned in
    // `amiga_rom_face`.
    assert_eq!(
        P::Amiga.release_face_space().text_scale((2, 2)),
        (2, 2),
        "the Amiga doubles its release face with its artwork",
    );
    assert_eq!(
        P::Macintosh.release_face_space().text_scale((2, 2)),
        (1, 1),
        "the Macintosh does not",
    );
}

// ── the fixed-pitch alternate ───────────────────────────────────────────────

/// **A fixed-pitch run keeps the machine's alternate, and its pitch** (SQ-1036).
///
/// `machine-screenshots/mac-zorkzero-game.png`: `Banquet Hall` occupies twelve
/// cells of exactly 7 px — 84 px — while the same twelve characters in Geneva do
/// not. Both numbers come from one `TextFace`, because the pen and the face are
/// one question asked once.
#[test]
fn a_fixed_pitch_run_takes_the_alternate_and_the_body_run_does_not() {
    const FIXED: u8 = 8; // §8.7.1 bit 3
    let boot = Disks::new("alt").with("System.img", "sysfont.hfs");
    let faces = cascade(&unit_tests_dir().join("relfont.hfs"), P::Macintosh, None, Some(&boot.disks(None)));
    let tf = TextFace::new(P::Macintosh, faces, None);

    assert!(tf.proportional(), "non-vacuity: the body pen really is Geneva's");
    assert!(!tf.draws_proportionally(FIXED), "…and a fixed-pitch run is not drawn with it");
    assert!(tf.draws_proportionally(0), "while a roman one is");

    assert_eq!(
        tf.face_for(FIXED).map(|f| (f.width, f.height)),
        Some((7, 15)),
        "a fixed-pitch run is drawn in the machine's alternate",
    );
    assert_eq!(
        tf.face_for(0).map(|f| (f.width, f.height)),
        Some((9, 15)),
        "and a roman one in its body face",
    );

    // The PEN agrees with the face, which is the half that a second implementation
    // would get wrong (SQ-1026/SQ-1035).
    let bar = "Banquet Hall";
    assert_eq!(
        tf.run_px_styled(bar, FIXED),
        bar.len() as u32 * 7,
        "twelve cells of the declared 7 — the status bar's own measurement",
    );
    assert_ne!(
        tf.run_px_styled(bar, 0),
        tf.run_px_styled(bar, FIXED),
        "and the body pen is a different number, or none of this was worth doing",
    );

    // With no alternate the bit is the no-op it has always been.
    let alone = TextFace::new(P::Macintosh, cascade(&unit_tests_dir().join("relfont.hfs"), P::Macintosh, None, None), None);
    assert_eq!(
        alone.run_px_styled(bar, FIXED),
        alone.run_px_styled(bar, 0),
        "no alternate, no second pitch — every configuration that shipped before this",
    );
}

/// The wrap cache can SEE the fixed-pitch pen (SQ-1034 + SQ-1036).
///
/// `TextFace::wrap_fingerprint` digests every advance the pen can answer, and the
/// transcript wrap keys on it. A digest that stopped at the four emphasis
/// combinations would hash a Geneva line for a Monaco run and keep a stale wrap.
#[test]
fn the_wrap_fingerprint_moves_when_the_fixed_pen_does() {
    let boot = Disks::new("fp").with("System.img", "sysfont.hfs");
    let release = unit_tests_dir().join("relfont.hfs");
    let with_alt = TextFace::new(
        P::Macintosh,
        cascade(&release, P::Macintosh, None, Some(&boot.disks(None))),
        None,
    );
    let body_only = {
        // The same body face, with the alternate taken away: built by hand rather
        // than by a second cascade, so the ONLY difference is the fixed pen.
        let faces = cascade(&release, P::Macintosh, None, Some(&boot.disks(None)));
        let body = faces.body().cloned().expect("the system disk supplied one");
        TextFace::new(P::Macintosh, FaceSet::release(body, P::Macintosh, None), None)
    };
    assert_eq!(
        with_alt.face_for(0),
        body_only.face_for(0),
        "non-vacuity: the two agree on every roman advance",
    );
    assert_ne!(
        with_alt.wrap_fingerprint(),
        body_only.wrap_fingerprint(),
        "and differ in the digest, because the fixed-pitch pen is part of what wraps",
    );
}

// ── which disk, when several answer ─────────────────────────────────────────

/// Several disks COMPOSE, and the config key breaks the tie (SQ-1037).
///
/// The user keeps Workbench 1.2 and 1.3 side by side deliberately, and every
/// pick-one rule is bad in a way they can see: first-found is filesystem order,
/// newest-version needs a version parsed off a name they may have renamed,
/// most-fonts is arbitrary. So the pool is ordered by a fact a person can read —
/// the filename — and `system_font_disk` promotes one to the front.
#[test]
fn two_disks_pool_and_the_config_key_orders_them() {
    let boot = Disks::new("prefer")
        .with("A-Startup.img", "sysfont.hfs")
        .with("Z-Startup.img", "sysfont.hfs");

    let by_name = app::system_fonts::named_faces_in(&boot.disks(None), P::Macintosh);
    assert_eq!(by_name.len(), 4, "two sizes of the family off each of two disks: {by_name:?}");
    assert_eq!(by_name[0].disk, "A-Startup.img", "with no preference the pool is ordered by name");

    let preferred = app::system_fonts::named_faces_in(&boot.disks(Some("z-start")), P::Macintosh);
    assert_eq!(
        preferred[0].disk, "Z-Startup.img",
        "a case-insensitive piece of the filename promotes that disk",
    );
    assert_eq!(preferred.len(), 4, "and EXCLUDES nothing — a preference must not lose you a face");

    // A preference naming a disk that is not there changes nothing, rather than
    // emptying the pool.
    let absent = app::system_fonts::named_faces_in(&boot.disks(Some("workbench")), P::Macintosh);
    assert_eq!(absent, by_name, "an unmatched preference falls back to the plain order");

    // End to end: the face the cascade draws with names the disk the key chose.
    let faces = cascade(
        &unit_tests_dir().join("relfont.hfs"),
        P::Macintosh,
        None,
        Some(&boot.disks(Some("Z-"))),
    );
    match faces.body_origin() {
        Some(app::native_font::FaceOrigin::SystemDisk { disk, .. }) => {
            assert_eq!(disk, "Z-Startup.img", "the report names the disk that answered");
        }
        other => panic!("expected a system-disk origin: {other:?}"),
    }
}

// ── the Amiga names topaz, and topaz only ───────────────────────────────────

/// **A Workbench floppy carries eight faces and the Amiga wants ONE of them.**
///
/// This is the case the name filter exists for, and it is not theoretical: the
/// Workbench 1.2 and 1.3 disks parse to `ruby`, `opal`, `sapphire`, `diamond`,
/// `garnet`, `emerald` and `topaz`, and every one but topaz is proportional. Take
/// "any face that fits" off that disk and an Amiga game is drawn in `ruby 8` —
/// which at the Amiga's own text scale of (2, 2) is a fifteen-wide face on a
/// sixteen-row line, so it passes the size rule and is a `FaceFit::Metric` face.
/// Nothing but the NAME rejects it.
///
/// Driven against the player's real disks under `~/.lanthorn/`, which are theirs
/// and not ours, so this skips vacuously exactly as a `stories/` case does. The
/// CI-safe half of the same claim is `blorb::amiga_font`'s own `drawer_of` tests
/// and the machine table's row.
#[test]
fn an_amiga_story_is_offered_topaz_and_never_the_workbench_display_faces() {
    let dir = app::system_fonts::user_media_dir();
    let all = app::system_fonts::scan(&dir);
    let amiga: Vec<&app::system_fonts::SystemFace> =
        all.iter().filter(|f| f.machine == P::Amiga).collect();
    if amiga.is_empty() {
        eprintln!("SKIP: no AmigaDOS boot disk under {}", dir.display());
        return;
    }
    // Non-vacuity: the disk really does carry faces this could wrongly take.
    assert!(
        amiga.iter().any(|f| f.proportional),
        "a Workbench disk carries proportional display faces: {amiga:?}",
    );

    let offered =
        app::system_fonts::named_faces_in(&UserDisks { dir, prefer: None }, P::Amiga);
    assert!(
        !offered.is_empty(),
        "topaz is on every Workbench disk, so the machine's own face is found: {amiga:?}",
    );
    for face in &offered {
        assert_eq!(
            blorb::amiga_font::drawer_of(&face.name).map(str::to_ascii_lowercase),
            Some("topaz".to_string()),
            "only the drawer the machine names is offered — {} is not it",
            face.name,
        );
    }
    assert!(
        offered.len() < amiga.len(),
        "and the filter really removed something: {} of {} faces",
        offered.len(),
        amiga.len(),
    );
}

/// And what a Workbench FLOPPY offers, the Amiga still declines — honestly and by
/// the one fitness rule (SQ-1037, SQ-1053).
///
/// A Workbench disk carries `fonts/topaz/11`: fixed-pitch, 8 wide, 11 rows. At
/// the Amiga's system-face scale of (1, 2) that is 8x22 against an 8x16 cell, so
/// it is neither the cell (`FaceFit::Cell` needs the height too) nor a typeface
/// (`FaceFit::Metric` needs a varying advance), and `fit` refuses it.
///
/// The machine's own topaz **8** is a different face in a different place — 8x8,
/// in Kickstart ROM, on no floppy at all — and SQ-1053 is what reads it. The
/// contrast is the non-vacuity below: the SIZE is the whole of the refusal here,
/// and the size the machine actually drew with sails through.
#[test]
fn the_amigas_disk_topaz_is_not_its_version_six_cell_and_declines() {
    let cell = P::Amiga.v6_font_cell();
    assert_eq!((cell.w(), cell.h()), (8, 16), "the machine table's Amiga cell");

    // `fonts/topaz/11` as the parser reports it: fixed pitch, 8x11.
    let topaz = blorb::bitmap_font::BitmapFont {
        width: 8,
        height: 11,
        baseline: 9,
        bold_smear: 1,
        proportional: false,
        lo: b'A',
        glyphs: (0..4)
            .map(|_| blorb::bitmap_font::Glyph { width: 8, rows: vec![0b0101_0101; 11] })
            .collect(),
    };
    // At the Amiga's SYSTEM face scale, which is the one a `FONTS:` face is judged
    // at (SQ-1053): (1, 2) on a doubled press, so eleven face rows are twenty-two
    // native against a sixteen-row cell.
    let system = P::Amiga.system_face_space().text_scale((2, 2));
    assert_eq!(system, (1, 2), "topaz is drawn in the machine's hires text space");
    assert_eq!(
        app::native_font::fit(&topaz, P::Amiga, system),
        None,
        "8x11 is neither the 8x16 cell nor a typeface, so nothing draws with it",
    );
    // Non-vacuity: the same face IS admitted where it does happen to be the cell,
    // so the refusal above is the height and not a blanket no.
    let mut as_cell = topaz.clone();
    as_cell.height = 8;
    for g in &mut as_cell.glyphs {
        g.rows = vec![0b0101_0101; 8];
    }
    assert_eq!(
        app::native_font::fit(&as_cell, P::Amiga, system),
        Some(app::native_font::FaceFit::Cell),
        "…and an 8x8 face at that scale IS the cell exactly — which is topaz 8",
    );
}

// ── a player's disk is untrusted input ──────────────────────────────────────

/// **A malformed or hostile disk image faults quietly** (SQ-1037).
///
/// `~/.lanthorn/` is whatever the player put there. A truncated floppy, an image
/// whose header claims an enormous font, a file that is only pretending to be a
/// volume — none of them may panic, hang or allocate without bound, and none may
/// stop a game from starting. The bound itself is
/// `blorb::bitmap_font::MAX_ROW_WIDTH`, which exists so a corrupt header cannot
/// demand an arbitrarily large per-row allocation.
#[test]
fn a_hostile_disk_image_is_refused_rather_than_trusted() {
    let good = std::fs::read(unit_tests_dir().join("sysfont.hfs")).expect("fixture");

    // 1. Truncated: the first kilobyte of a real volume.
    // 2. A header that says HFS over nothing at all.
    // 3. Bytes that are not a volume in any format.
    // 4. Every byte of a real volume flipped — structure-shaped, values wrong.
    // 5. Empty.
    let flipped: Vec<u8> = good.iter().map(|b| !b).collect();
    let mut lying = vec![0u8; 64 * 1024];
    lying[1024] = 0x42; // 'BD' at the MDB, and nothing behind it
    lying[1025] = 0x44;

    let boot = Disks::new("hostile")
        .with_bytes("truncated.img", &good[..1024])
        .with_bytes("lying.img", &lying)
        .with_bytes("garbage.img", &(0u8..=255).cycle().take(70_000).collect::<Vec<u8>>())
        .with_bytes("flipped.img", &flipped)
        .with_bytes("empty.img", b"");

    // The scan is the whole surface: it mounts, parses and reports, and a caller
    // gets a list — possibly empty — rather than an error or a crash.
    let found = app::system_fonts::scan(&boot.dir);
    assert!(
        found.iter().all(|f| f.width as usize <= blorb::bitmap_font::MAX_ROW_WIDTH),
        "nothing wider than the bound a corrupt header could otherwise demand: {found:?}",
    );

    // And the cascade over the same directory answers without one of them
    // reaching the renderer.
    let faces = cascade(
        &unit_tests_dir().join("relfont.hfs"),
        P::Macintosh,
        None,
        Some(&boot.disks(None)),
    );
    assert_eq!(
        faces.body().map(|f| (f.width, f.height)),
        Some((7, 15)),
        "the release's own face still answers; no rubbish was admitted over it",
    );

    // Non-vacuity: the same directory with a GOOD disk in it does find one, so the
    // empty answers above are the guards working rather than a broken scan.
    let ok = Disks::new("hostile-ok").with("System.img", "sysfont.hfs");
    assert!(!app::system_fonts::scan(&ok.dir).is_empty(), "the scan itself works");
}

// ── the real Macintosh press ────────────────────────────────────────────────

/// End to end on the medium the capture was taken from (SQ-1036).
///
/// `stories/Zork Zero Disk.image` — release **296**, serial **881019**, the
/// standard Macintosh monochrome press. Gitignored, so this skips vacuously.
///
/// The System disk is the committed fixture rather than the player's own, so the
/// numbers below are the CASCADE's behaviour and not a statement about Geneva; the
/// Geneva measurement lives in the module docs and in
/// `machine-screenshots/mac-zorkzero-game.png`.
#[test]
fn the_macintosh_press_takes_the_system_face_for_its_body_and_keeps_monaco_for_its_bar() {
    let path = stories_dir().join("Zork Zero Disk.image");
    if !path.is_file() {
        eprintln!("SKIP: gitignored Macintosh medium absent at {}", path.display());
        return;
    }
    let bytes = match app::hints::load_story(&path).expect("Story.data mounts") {
        app::hints::LoadedStory::ZCode(b) => b,
        other => panic!("expected Z-code, got {other:?}"),
    };
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 296, "this disk carries r296");
    assert_eq!(&bytes[0x12..0x18], b"881019", "…serial 881019");

    let (profile, source) = P::resolve_with_source(&path, None, None, None);
    assert_eq!(profile, P::Macintosh, "the medium names the Macintosh");
    assert_eq!(source, app::interpreter::ProfileSource::Medium);

    let boot_disk = Disks::new("real").with("System.img", "sysfont.hfs");
    let picts = app::graphics::PictSource::resolve(&path, None);
    let faces = app::native_font::resolve(&FaceRequest {
        story_path: &path,
        entry: None,
        profile,
        source,
        art_scale: picts.art_scale(),
        disks: Some(&boot_disk.disks(None)),
    });
    let machine = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        faces,
        profile.palette(),
        None,
    );

    // The frame's shape, before anything is measured against it (CLAUDE.md).
    assert_eq!(machine.screen_px, Some((320, 200)), "the press's own picture space");
    assert_eq!(
        machine.cell,
        zvm::interpreter::MACINTOSH_V6_CELL,
        "and the story is told 7x15 — admitting a body face must not move it",
    );

    let tf = machine.text_face();
    assert!(tf.proportional(), "the body pen is the system face's");
    assert_eq!(
        tf.face_for(8).map(|f| (f.width, f.height)),
        Some((7, 15)),
        "and `FONT` 524 off the game's own disk answers a fixed-pitch run",
    );
    assert_eq!(
        tf.run_px_styled("Banquet Hall", 8),
        84,
        "twelve characters at the declared 7, as mac-zorkzero-game.png measures the bar",
    );
}

/// **The game's own `@set_font 4` is what marks the bar**, and it reaches the
/// runs as §8.7.1's fixed-pitch bit (SQ-1036).
///
/// Twelve turns into `stories/Zork Zero Disk.image` (r296/881019) the status
/// window carries `Banquet Hall`, `Moves:`, `Score:` and `Flatheadia`, and the
/// game printed every one of them between `@set_font 4` and `@set_font 1` while
/// leaving the style word at zero. Without the fold there is nothing in a run to
/// tell the bar from the prose, and a machine with two faces cannot choose.
#[test]
fn zork_zero_marks_its_status_bar_with_font_four() {
    let path = stories_dir().join("Zork Zero Disk.image");
    if !path.is_file() {
        eprintln!("SKIP: gitignored Macintosh medium absent at {}", path.display());
        return;
    }
    let bytes = match app::hints::load_story(&path).expect("Story.data mounts") {
        app::hints::LoadedStory::ZCode(b) => b,
        other => panic!("expected Z-code, got {other:?}"),
    };
    let (profile, source) = P::resolve_with_source(&path, None, None, None);
    let mut picts = app::graphics::PictSource::resolve(&path, None);
    let dims = picts.all_pict_dims();
    let faces = app::native_font::resolve(&FaceRequest {
        story_path: &path,
        entry: None,
        profile,
        source,
        art_scale: picts.art_scale(),
        disks: None,
    });
    let machine = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        faces,
        profile.palette(),
        None,
    );
    let mut s =
        app::session::GameSession::new_for_machine(bytes, true, false, false, dims, None, None, &machine)
            .expect("Zork Zero boots off the Macintosh disk");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    // TWELVE turns: the intro cards, then the Banquet Hall frame the capture shows.
    for _ in 0..12 {
        match s.pending_input() {
            app::session::InputKind::Line => {
                s.submit("");
            }
            app::session::InputKind::Char => {
                let _ = s.submit_char(13);
            }
            app::session::InputKind::Event => {
                s.submit("");
            }
        }
    }
    let v6 = s.machine.screen.v6.as_ref().expect("a v6 screen");
    let bar = &v6.windows[1].texts;
    // The frame's shape, guarded before it is measured.
    assert!(
        bar.iter().any(|r| r.text.contains("Banquet Hall")),
        "twelve turns reaches the Banquet Hall frame: {:?}",
        bar.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
    );
    assert!(
        bar.iter().all(|r| r.style & 8 != 0),
        "and every run the game painted there is marked fixed-pitch",
    );
    assert!(
        v6.windows[0].texts.iter().chain(v6.windows[0].streamed.iter()).all(|r| r.style & 8 == 0),
        "while nothing in the prose window is",
    );
}
