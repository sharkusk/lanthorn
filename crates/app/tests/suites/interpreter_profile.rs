//! SQ-0719 (second half) / SQ-0736: the interpreter profile — which machine
//! lanthorn presents itself to the story as — and the 1× artwork that selecting
//! it dissolves.
//!
//! The reported symptom: *Zork Zero* booted straight off its Amiga release
//! floppy drew its art at half size. The cause was two individually-correct
//! rules meeting. Blorb §11 says a resource file with no `Reso` chunk declares
//! no scalable images, so lanthorn shows them at actual size (SQ-0715, which is
//! why scopa and mysterious01 correctly stay at 1:1). A native Amiga `Pic.data`
//! archive has no `Reso` chunk because **the format has no such concept** — so
//! absence of evidence was being read as evidence of absence, and 320×200 art
//! landed unscaled on the 640×400 screen the game was still laying itself out
//! against.
//!
//! The fix is not a special case for `.adf`: it is that the machine, not the
//! container, knows the standard window. A story off an Amiga floppy is an
//! Amiga, and an Amiga's Version 6 standard window is 320×200 — the very same
//! thing every Infocom Blorb's `Reso` chunk declares, which is why a Blorb copy
//! of the same game never had the problem.
//!
//! Everything here is synthetic: the disk image is built in-process and the
//! story is a hand-assembled Version 6 stub, so nothing depends on the user's
//! own media or on the gitignored `stories/`.

use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::GameSession;

// ── A disk image, built by hand ───────────────────────────────────────────────
// The same minimal OFS writer `adf_disk_image.rs` uses; the filesystem itself is
// covered by `blorb`'s own unit tests, and this only needs a mountable image.

const BSIZE: usize = 512;
const DATA_TABLE_TOP: usize = BSIZE - 204;
const OFS_DATA_HEADER: usize = 24;
const DD_BLOCKS: usize = 1760;

fn build_adf(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut image = vec![0u8; DD_BLOCKS * BSIZE];
    image[0..3].copy_from_slice(b"DOS");
    image[3] = 0; // OFS, as both Zork Zero disks are
    let mut next = 881;
    let put32 = |img: &mut Vec<u8>, block: usize, off: usize, v: u32| {
        let at = block * BSIZE + off;
        img[at..at + 4].copy_from_slice(&v.to_be_bytes());
    };
    for (name, data) in files {
        let header = next;
        next += 1;
        put32(&mut image, header, 0, 2); // T_HEADER
        put32(&mut image, header, 4, header as u32);
        put32(&mut image, header, BSIZE - 4, 0xFFFF_FFFD); // ST_FILE
        put32(&mut image, header, BSIZE - 188, data.len() as u32);
        let at = header * BSIZE + BSIZE - 80;
        image[at] = name.len() as u8;
        image[at + 1..at + 1 + name.len()].copy_from_slice(name.as_bytes());

        let chunks: Vec<&[u8]> = data.chunks(BSIZE - OFS_DATA_HEADER).collect();
        assert!(chunks.len() <= 72, "{name} needs an extension block");
        for (i, chunk) in chunks.iter().enumerate() {
            let db = next;
            next += 1;
            put32(&mut image, db, 0, 8); // T_DATA
            put32(&mut image, db, 4, header as u32);
            put32(&mut image, db, 8, i as u32 + 1);
            put32(&mut image, db, 12, chunk.len() as u32);
            let at = db * BSIZE + OFS_DATA_HEADER;
            image[at..at + chunk.len()].copy_from_slice(chunk);
            put32(&mut image, header, DATA_TABLE_TOP - 4 * i, db as u32);
        }
        put32(&mut image, header, 8, chunks.len() as u32);
    }
    image
}

/// A minimal Version 6 story whose `main` routine quits immediately — enough for
/// the session constructor's v6 arm to boot without faulting, which is all these
/// assertions need. (Mirrors `session.rs`'s own `v6_boot_stub_story`, with the
/// dictionary in static memory and a printable serial so `blorb::adf`'s
/// content-based story recogniser accepts it off a disk image — AmigaDOS has no
/// extensions to go on.)
fn v6_boot_stub_story() -> Vec<u8> {
    let mut buf = vec![0u8; 0x0800];
    buf[0x00] = 6; // version
    buf[0x04] = 0x04;
    buf[0x05] = 0x00; // high_mem_base = 0x0400
    // 0x06/0x07 = main's packed address; routines_offset is 0, so 0x0100 packs to 0x0040.
    buf[0x06] = 0x00;
    buf[0x07] = 0x40;
    buf[0x08] = 0x05;
    buf[0x09] = 0x00; // dictionary = 0x0500, in static memory (empty)
    buf[0x0500] = 0;
    buf[0x0501] = 4;
    buf[0x0502] = 0;
    buf[0x0503] = 0;
    buf[0x0A] = 0x01;
    buf[0x0B] = 0x00; // object table
    buf[0x0C] = 0x03;
    buf[0x0D] = 0x00; // globals
    buf[0x0E] = 0x04;
    buf[0x0F] = 0x00; // static memory base
    buf[0x12..0x18].copy_from_slice(b"890323"); // serial
    buf[0x18] = 0x00;
    buf[0x19] = 0x60; // abbreviations
    buf[0x0100] = 0; // main: 0 locals…
    buf[0x0101] = 0xBA; // …then quit
    buf
}

/// A one-picture Amiga archive: id 7, 4×2. Same bytes as `adf_disk_image.rs`
/// uses; the codec is `blorb::infocom_pics`' own business.
fn fake_pic_data() -> Vec<u8> {
    const ENTRY_SIZE: usize = 14;
    const HUFF_LEN: usize = 256;
    let mut f = vec![0u8; 16];
    f[0] = 1; // part number
    f[2] = 0x00;
    f[3] = 0x0F; // Huffman-tree offset, in words
    f[5] = 1; // one picture
    f[8] = ENTRY_SIZE as u8;
    let data_off = 16 + ENTRY_SIZE + HUFF_LEN;
    f.extend_from_slice(&[
        0, 7, // id
        0, 4, // width
        0, 2, // height
        0, 3, // EF_TRANS | EF_PHUFF
        (data_off >> 16) as u8,
        (data_off >> 8) as u8,
        data_off as u8,
        0, 0, 0, // no palette
    ]);
    let mut tree = vec![0u8; HUFF_LEN];
    tree[0] = 128 + 2;
    tree[1] = 1;
    tree[2] = 128 + 1;
    tree[3] = 128 + 18;
    f.extend_from_slice(&tree);
    f.extend_from_slice(&[0, 0, 1]); // minSize
    f.extend_from_slice(&[0, 0, 4]); // midSize
    f.push(0b0111_0110);
    f
}

fn write_image(name: &str, image: &[u8]) -> std::path::PathBuf {
    let path = app::scratch_dir(&format!("profile-{name}")).join(format!("lanthorn-profile-{name}"));
    std::fs::write(&path, image).expect("write the disk image");
    path
}

// ── Selection ─────────────────────────────────────────────────────────────────

/// SQ-0734's precedence, all three arms, on real files.
#[test]
fn the_medium_picks_the_machine_and_an_explicit_number_overrules_it() {
    let disk = write_image(
        "select",
        &build_adf(&[("Story.data", &v6_boot_stub_story()), ("Pic.data", &fake_pic_data())]),
    );
    let plain = write_image("plain", &v6_boot_stub_story());

    // 2. The medium: a story off an Amiga floppy is an Amiga. No configuration.
    assert_eq!(InterpreterProfile::resolve(&disk, None, None, None), InterpreterProfile::Amiga);
    // 3. Everything else is an IBM PC — today's behaviour, named.
    assert_eq!(InterpreterProfile::resolve(&plain, None, None, None), InterpreterProfile::IbmPc);
    // 1. An explicit interpreter number always wins, in both directions: it names
    //    the machine, not merely the byte, which is what makes asking for one
    //    coherent instead of half-applied.
    assert_eq!(InterpreterProfile::resolve(&disk, Some(6), None, None), InterpreterProfile::IbmPc);
    assert_eq!(InterpreterProfile::resolve(&plain, Some(4), None, None), InterpreterProfile::Amiga);

    let _ = std::fs::remove_file(&disk);
    let _ = std::fs::remove_file(&plain);
}

// ── The 1× artwork (SQ-0736) ──────────────────────────────────────────────────

/// The whole reported defect, end to end over the real loading path.
///
/// FALSIFICATION: make `InterpreterProfile::resolve` return `IbmPc` for the disk
/// image (or drop the `.or_else(profile.std_window())` in `startup.rs`) and this
/// fails with the user's own symptom — picture 7 reported as 4×2, its actual
/// size, on a screen the game is told is 640×400.
#[test]
fn a_story_off_a_disk_image_draws_its_art_at_the_screens_scale() {
    let disk = write_image(
        "scale",
        &build_adf(&[("Story.data", &v6_boot_stub_story()), ("Pic.data", &fake_pic_data())]),
    );

    let bytes = match app::hints::load_story(&disk).expect("the story mounts") {
        app::hints::LoadedStory::ZCode(b) => b,
        other => panic!("expected Z-code, got {other:?}"),
    };
    let profile = InterpreterProfile::resolve(&disk, None, None, None);
    let mut picts = PictSource::resolve(&disk, None);
    let dims = picts.all_pict_dims();
    assert_eq!(dims, vec![(7, 4, 2)], "the archive's own, art-native dimensions");

    // The native archive declares no standard window — the format cannot — so
    // the machine answers. This is exactly the line `startup.rs` runs.
    assert_eq!(picts.std_window(), None, "no Reso chunk exists to read");
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());

    let session = GameSession::new_with_trace(
        bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .expect("the stub boots");

    assert_eq!(
        session.machine.picture_dims(7),
        Some((8, 4)),
        "picture_data must report unit-space sizes; 4×2 here is the 1× bug",
    );
    // …and it is scaled against a screen that really is 640×400 (80×25 cells at
    // the 8×16 Version 6 cell), so the game's layout arithmetic agrees with it.
    assert_eq!(session.machine.mem.read_byte(0x20), 25, "rows");
    assert_eq!(session.machine.mem.read_byte(0x21), 80, "columns");
    assert_eq!(v6_screen_px, Some((320, 200)), "and it came from the Amiga's standard window");

    let _ = std::fs::remove_file(&disk);
}

/// The other side of the same rule, and the regression this must never cause: a
/// resource file that genuinely declares no scalable images keeps 1:1 art
/// (SQ-0715/SQ-0718 — scopa, mysterious01). The IBM PC profile has no opinion
/// about standard windows, so nothing here changes.
#[test]
fn a_blorbless_story_that_is_not_a_disk_image_still_draws_at_actual_size() {
    let plain = write_image("noreso", &v6_boot_stub_story());
    let profile = InterpreterProfile::resolve(&plain, None, None, None);
    assert_eq!(profile, InterpreterProfile::IbmPc);

    let dims = vec![(7u16, 4u16, 2u16)];
    let v6_screen_px = PictSource::resolve(&plain, None).std_window().or_else(|| profile.std_window());
    assert_eq!(v6_screen_px, None, "no declaration from either container or machine");

    let session = GameSession::new_with_trace(
        v6_boot_stub_story(),
        true,
        false,
        profile.interpreter_number(),
        false,
        dims.clone(),
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .expect("the stub boots");
    assert_eq!(session.machine.picture_dims(7), Some((4, 2)), "actual size, one image pixel per screen pixel");

    let _ = std::fs::remove_file(&plain);
}

// ── The rest of the bundle ────────────────────────────────────────────────────

/// The Amiga profile is a machine, not a byte: the number it advertises, the
/// default page and ink it reports, and the palette its colour numbers name all
/// arrive together.
#[test]
fn the_amiga_profile_reports_an_amiga_to_the_game() {
    let disk = write_image(
        "header",
        &build_adf(&[("Story.data", &v6_boot_stub_story()), ("Pic.data", &fake_pic_data())]),
    );
    let profile = InterpreterProfile::resolve(&disk, None, None, None);

    let session = GameSession::new_with_trace(
        v6_boot_stub_story(),
        true,
        false,
        profile.interpreter_number(),
        false,
        Vec::new(),
        profile.std_window(),
        profile.default_colours(),
        None,
    )
    .expect("the stub boots");

    // ZMSD §11.1.3: 4 = Amiga.
    assert_eq!(session.machine.mem.read_byte(0x1E), 4, "header $1E");
    // ZMSD §8.3.3 + the release floppies' own `DEF_BACK`/`DEF_FORE` (SQ-0822):
    // dark grey page, white ink.
    assert_eq!(session.machine.mem.read_byte(0x2C), 12, "header $2C default background");
    assert_eq!(session.machine.mem.read_byte(0x2D), 9, "header $2D default foreground");

    let _ = std::fs::remove_file(&disk);
}

/// And the IBM PC profile is exactly the absence of all of that — the property
/// the whole v6 corpus's byte-identity rests on. Every knob defers, so a story
/// that is not on Amiga media boots the machine it always did.
#[test]
fn the_ibm_pc_profile_changes_nothing_it_touches() {
    let plain = write_image("ibmpc", &v6_boot_stub_story());
    let (profile, source) = InterpreterProfile::resolve_with_source(&plain, None, None, None);
    assert_eq!(profile.interpreter_number(), None);
    assert_eq!(profile.std_window(), None);
    // SQ-0928/SQ-0939: what changes nothing is the LAUNCH, not the machine. This
    // file named no medium, so the profile fell through here and the source
    // licenses no colours — the pair the IBM PC states, and the EGA table it
    // resolves colour numbers through, are real and simply out of reach.
    // `startup` downgrades an unlicensed launch to §8.3.1's own table before the
    // process-global palette is ever set, which is what keeps every Inform game on
    // the standard colours.
    assert_eq!(profile.palette(), zvm::screen::Palette::IbmXzip, "the machine's own table");
    assert_eq!(source, app::interpreter::ProfileSource::Fallback);
    assert!(!source.licenses_machine_colours(true), "nothing rescues a fallback");
    assert_eq!(profile.default_colours(), Some((6, 9)), "the machine's own fact");

    // Frotz's rule still supplies $1E, untouched: 6 for Version 6…
    let v6 = GameSession::new_with_trace(
        v6_boot_stub_story(),
        true,
        false,
        profile.interpreter_number(),
        false,
        Vec::new(),
        None,
        None,
        None,
    )
    .expect("the stub boots");
    assert_eq!(v6.machine.mem.read_byte(0x1E), 6, "IBM PC on Version 6");

    let _ = std::fs::remove_file(&plain);
}

// ── the two tables that must not drift (SQ-0872) ──────────────────────────────

/// `blorb::medium` and `zvm::interpreter` both name §11.1.3 interpreter numbers,
/// and neither can see the other: `blorb` takes zero external dependencies, and
/// `zvm` must not depend on it. That is the right shape — a NUMBER is a compact,
/// published encoding that needs no shared type, which is exactly why it works as
/// a lingua franca between two crates kept deliberately independent — but it
/// means the values are stated twice, and a value stated twice can diverge.
///
/// So this is the seam, asserted. `blorb` answers *which machine a disk implies*;
/// `zvm` answers *what that machine IS* — the colour pair, the palette, the §8.3
/// screen rules. This crate is the only one that sees both, so the agreement is
/// pinned here.
///
/// FALSIFICATION: change `blorb::medium::AMIGA_INTERPRETER_NUMBER` to 5 and the
/// first row fails with `the Amiga's number: blorb says 5, zvm says 4`.
#[test]
fn blorb_and_zvm_agree_on_every_machine_they_both_name() {
    for (label, from_blorb, from_zvm) in [
        (
            "the Amiga's number",
            blorb::medium::AMIGA_INTERPRETER_NUMBER,
            zvm::interpreter::AMIGA_INTERPRETER_NUMBER,
        ),
        (
            "the Macintosh's number",
            blorb::medium::MACINTOSH_INTERPRETER_NUMBER,
            zvm::interpreter::MACINTOSH_INTERPRETER_NUMBER,
        ),
        (
            "the Atari ST's number",
            blorb::medium::ATARI_ST_INTERPRETER_NUMBER,
            zvm::interpreter::ATARI_ST_INTERPRETER_NUMBER,
        ),
        (
            "the Apple IIgs's number",
            blorb::medium::APPLE_IIGS_INTERPRETER_NUMBER,
            zvm::interpreter::APPLE_IIGS_INTERPRETER_NUMBER,
        ),
        (
            "the Commodore 128's number",
            blorb::medium::COMMODORE_128_INTERPRETER_NUMBER,
            zvm::interpreter::COMMODORE_128_INTERPRETER_NUMBER,
        ),
    ] {
        assert_eq!(from_blorb, from_zvm, "{label}: blorb says {from_blorb}, zvm says {from_zvm}");
    }

    // …and every number any MEDIUM hands out must name a machine zvm models, or a
    // disk launch would present the IBM PC's colours under another machine's byte
    // — the exact half-wiring SQ-0872 exists to close, arriving from the medium
    // instead of from the CLI.
    for d in blorb::medium::DiskImage::all() {
        let Some(n) = d.interpreter_number() else { continue };
        assert!(
            zvm::interpreter::machine(n).is_some(),
            "{d:?} hands out interpreter {n}, which zvm models no machine for",
        );
        // …and the app's own bundle reaches that same machine off the same byte.
        assert_eq!(
            InterpreterProfile::for_interpreter_number(n).interpreter_number(),
            Some(n),
            "{d:?}: the profile for {n} must state {n} back",
        );
    }
}

/// The app's bundle and zvm's table are one machine seen from two crates, so
/// every knob a STORY can read has to match — that is what makes the TUI and
/// `zvm-cli` present the same machine off the same disk (SQ-0872).
///
/// FALSIFICATION: change the Macintosh row's `default_colours` in
/// `zvm::interpreter` to `Some((2, 9))` and this fails at `Macintosh $2C/$2D`.
#[test]
fn the_profiles_bundle_is_the_zvm_tables_row() {
    for (profile, number) in [
        (InterpreterProfile::Macintosh, 3u8),
        (InterpreterProfile::Amiga, 4),
        (InterpreterProfile::AtariSt, 5),
        (InterpreterProfile::Commodore128, 7),
        (InterpreterProfile::AppleIIe, 2),
        (InterpreterProfile::AppleIIc, 9),
        (InterpreterProfile::AppleIIgs, 10),
    ] {
        let row = zvm::interpreter::machine(number).expect("modelled");
        assert_eq!(profile.interpreter_number(), Some(number), "{profile:?} $1E");
        assert_eq!(profile.default_colours(), row.default_colours, "{profile:?} $2C/$2D");
        assert_eq!(profile.palette(), row.palette, "{profile:?} palette");
    }
}


// ── SQ-0928: system colours are licensed by the MEDIUM ────────────────────────

/// A machine's §8.3.3 pair describes a MACHINE. Running a story off its release
/// disk makes that description true of the launch; opening a bare file does not.
///
/// This is the case the whole design turns on, and it only became load-bearing
/// when the IBM PC gained a pair: `InterpreterProfile::resolve` answers `IbmPc`
/// for every story with no medium — every modern Inform game anyone opens — so
/// without the gate, blue under white would be painted across the entire
/// non-Infocom corpus.
#[test]
fn only_original_media_licenses_a_machines_own_colours() {
    use app::interpreter::ProfileSource;

    // A story file with no medium: the fallback, and it can never be licensed —
    // not even by the opt-in, because there is no machine there to be faithful to.
    let bare = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../zvm/tests/fixtures/czech.z5");
    assert!(bare.exists(), "the redistributable fixture is checked in");
    let (prof, src) = InterpreterProfile::resolve_with_source(&bare, None, None, None);
    assert_eq!(prof, InterpreterProfile::IbmPc);
    assert_eq!(src, ProfileSource::Fallback);
    assert!(!src.licenses_machine_colours(false));
    assert!(!src.licenses_machine_colours(true), "the opt-in cannot conjure a machine");

    // The machine still STATES its pair — that is a fact about the IBM PC, and
    // separating the fact from the licence is the point.
    assert_eq!(prof.default_colours(), Some((6, 9)), "blue under white");

    // A number NAMED BY HAND reaches $1E and stops there, until the player says
    // they meant the whole machine.
    let (prof, src) = InterpreterProfile::resolve_with_source(&bare, Some(4), None, None);
    assert_eq!(prof, InterpreterProfile::Amiga);
    assert_eq!(src, ProfileSource::Asked);
    assert!(!src.licenses_machine_colours(false), "a typed number is not original media");
    assert!(src.licenses_machine_colours(true), "…until --colour machine says so");
}

/// The same, on a real release disk: the medium licenses it with no flag at all.
///
/// Gitignored fixture, so it skips vacuously — with a guard, because a skip that
/// reads as a pass is worth nothing.
#[test]
fn a_release_floppy_licenses_them_with_no_flag() {
    use app::interpreter::ProfileSource;

    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories");
    let mut seen = 0;
    for (file, want) in [
        ("Zork I - The Great Underground Empire.adf", InterpreterProfile::Amiga),
        ("James Clavell's Shogun.adf", InterpreterProfile::Amiga),
    ] {
        let path = dir.join(file);
        if !path.is_file() {
            eprintln!("SKIP: gitignored disk missing at {}", path.display());
            continue;
        }
        seen += 1;
        let (prof, src) = InterpreterProfile::resolve_with_source(&path, None, None, None);
        assert_eq!(prof, want, "{file}");
        assert_eq!(src, ProfileSource::Medium, "{file} came off original media");
        assert!(src.licenses_machine_colours(false), "{file}: the disk is the licence");
        assert!(prof.default_colours().is_some(), "{file}: and the machine states a pair");
    }
    let any_present = ["Zork I - The Great Underground Empire.adf", "James Clavell's Shogun.adf"]
        .iter()
        .any(|f| dir.join(f).is_file());
    assert!(!any_present || seen > 0, "disks are present but none was read");
}


// ── SQ-0930: the two meanings of `interpreter_number: None` ───────────────────

/// **A DOS medium NAMES the IBM PC**, and reading its `None` as "no machine" cost
/// two visible things at once.
///
/// `blorb::medium`'s DOS row answers `None` because the machine's §11.1.3 number is
/// a version RULE, not because the disk is silent — its own comment says so. The
/// ISO row's `None` means the opposite: a hybrid disc is both machines and a number
/// would be wrong for half of it. Reading them alike made a DOS floppy resolve as a
/// FALLBACK, so the story was told DECSystem-20 (`$1E` = 1) and the machine's own
/// page never applied — on the one medium that unambiguously states it.
#[test]
fn a_dos_medium_names_the_ibm_pc_even_though_its_number_is_a_rule() {
    use app::interpreter::ProfileSource;

    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories");
    let mut seen = 0;
    for file in ["floppy1.ima", "disk1.img"] {
        let path = dir.join(file);
        if !path.is_file() {
            eprintln!("SKIP: gitignored disk missing at {}", path.display());
            continue;
        }
        seen += 1;
        let (profile, source) = InterpreterProfile::resolve_with_source(&path, None, None, None);
        assert_eq!(profile, InterpreterProfile::IbmPc, "{file}");
        assert_eq!(source, ProfileSource::Medium, "{file}: the disk names the machine");
        assert!(source.licenses_machine_colours(false), "{file}: so its colours apply");

        // …and the header says so. Deferring to zvm's version rule here advertised
        // 1, which is the DECSystem-20.
        let cfg = app::config::Config {
            interpreter_profile: profile,
            interpreter_source: source,
            ..app::config::Config::default()
        };
        assert_eq!(
            cfg.advertised_interpreter_number(),
            Some(app::interpreter::IBM_PC_INTERPRETER_NUMBER),
            "{file}: $1E must say IBM PC, not DECSystem-20",
        );
    }
    let any_present = ["floppy1.ima", "disk1.img"].iter().any(|f| dir.join(f).is_file());
    assert!(!any_present || seen > 0, "DOS media are present but none was read");
}

/// **An unmodelled number is a fallback, not a machine the player asked for.**
///
/// `for_interpreter_number` lands every number this table does not model on
/// `IbmPc`. That was inert while the variant stated nothing; it is not now that it
/// states blue under white, and `--interpreter 1 --colour machine` would otherwise
/// paint a DECSystem-20 in the IBM PC's own colours. The number still reaches
/// `$1E` — the story asked and §11.1.3 has an answer — and only the presentation
/// is withheld.
#[test]
fn an_unmodelled_number_never_borrows_the_ibm_pcs_colours() {
    use app::interpreter::ProfileSource;

    let bare = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../zvm/tests/fixtures/czech.z5");
    for n in [1u8, 11] {
        let (profile, source) = InterpreterProfile::resolve_with_source(&bare, Some(n), None, None);
        assert_eq!(profile, InterpreterProfile::IbmPc, "-I {n} still falls back here");
        assert_eq!(source, ProfileSource::Fallback, "-I {n} names no machine we model");
        assert!(
            !source.licenses_machine_colours(true),
            "-I {n}: not even --colour machine may lend it the IBM PC's page",
        );
    }
    // 6 is the IBM PC itself, so asking for it IS asking for a machine.
    let (_, source) = InterpreterProfile::resolve_with_source(&bare, Some(6), None, None);
    assert_eq!(source, ProfileSource::Asked);
    assert!(source.licenses_machine_colours(true), "and the opt-in reaches it");
}
