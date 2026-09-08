//! SQ-1053 — the Amiga draws in **topaz 8**, and topaz 8 is in Kickstart ROM.
//!
//! # What the machine did
//!
//! `machine-screenshots/amiga-shogun-game.png` is *Shogun* on a real Amiga,
//! captured at 2x the machine's 320x200 frame. Over the word `Erasmus` in "This
//! is the bridge of the Erasmus, a Dutch merchant":
//!
//! * the glyph band holds **10 distinct scanlines across a 20-row line pitch** —
//!   every face row is drawn twice, so the face is **8 rows** on a 16-row line;
//! * the underline runs **60 px over 7 characters**, so the pen steps **~8 native
//!   pixels** per character.
//!
//! That is an 8x8 face drawn at a text scale of **(1, 2)**, which is exactly the
//! 8x16 cell the machine declares. It is the Amiga's 640x200 hires mode: a text
//! pixel is 1:1 with the native 640 across, and a square-pixel screen doubles the
//! 200 rows to 400.
//!
//! # Why the cascade could not reach it before
//!
//! SQ-1037 built the release medium → user boot disk → built-in cascade, and on
//! the Amiga every rung declined. The reason was the MEDIA, not the cascade: a
//! Workbench 1.2/1.3 floppy carries `fonts/topaz/11` (8x11) and six PROPORTIONAL
//! display faces — ruby, opal, sapphire, diamond, garnet, emerald — and no
//! Infocom interpreter drew with any of them. The 8x8 the machine actually
//! painted with is in Kickstart, so only *Arthur* — which ships its own
//! `char.data` on its own floppy — ever got a real face.
//!
//! # Two faces on one machine, wanting two different scales
//!
//! Arthur's `char.data` is authored in the game's 320-wide PICTURE space and is
//! measured at (2, 2): ten face rows become the twenty-row declared line
//! `machine-screenshots/amiga-arthur-text.png` shows. ROM topaz is authored in
//! the 640-wide HIRES space and wants (1, 2). So a face's scale is a property of
//! its PROVENANCE, not of the machine's row — `zvm::interpreter::V6FaceSpace`
//! gained `Hires` and `V6SystemFace::face_space` states which faces take it.
//!
//! # Fixtures
//!
//! `unit_tests/kickfont.rom` is a **synthetic** Kickstart-shaped image, committed
//! with its generator (`unit_tests/mk_kickfont_rom.py`). A real Kickstart is
//! copyrighted Commodore code and is never committed here; the cases that want
//! one read the player's own `~/.lanthorn/` and skip vacuously, exactly as a
//! `stories/` case does.
//!
//! The fixture carries the three discriminators: `topaz/8` (right name, right
//! size), `topaz/9` (right name, wrong size — Kickstart 1.2 really does carry a
//! second topaz of that geometry) and `ruby/8` (wrong name, and otherwise
//! IDENTICAL to `topaz/8`, so the only thing that can refuse it is the name).

use app::interpreter::InterpreterProfile as P;
use app::native_font::{FaceFit, FaceOrigin, FaceRequest, FaceSet, TextFace};
use app::system_fonts::UserDisks;
use blorb::bitmap_font::BitmapFont;
use std::path::{Path, PathBuf};

/// The face the Amiga's Version 6 interpreter drew prose with, as
/// `blorb::amiga_font` names a ROM face: `<drawer>/<size>`.
const TOPAZ_8: &str = "topaz/8";
/// Kickstart 1.2's other face — the right family at a size the machine did not
/// draw with, which is the discriminator `sysfont.hfs`'s `FONT` 394 plays on the
/// Macintosh side.
const TOPAZ_9: &str = "topaz/9";
/// A Workbench display face, standing in for the seven a real floppy carries.
const RUBY_8: &str = "ruby/8";

/// A 256 KiB Kickstart maps at `$FC0000`; a 512 KiB one at `$F80000`.
const KICK_256K: usize = 256 * 1024;

fn unit_tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../unit_tests")
}

fn fixture() -> Vec<u8> {
    let path = unit_tests_dir().join("kickfont.rom");
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("{} is committed and readable: {e}", path.display()))
}

/// A scratch directory of our own standing in for `~/.lanthorn/`, so nothing here
/// depends on what the person running the tests happens to own.
struct Media {
    dir: PathBuf,
}

impl Media {
    fn new(tag: &str) -> Media {
        Media { dir: app::scratch_dir(&format!("sq1053-{tag}")) }
    }

    fn with_bytes(self, name: &str, bytes: &[u8]) -> Media {
        std::fs::write(self.dir.join(name), bytes).expect("write");
        self
    }

    fn with_rom(self, name: &str) -> Media {
        let bytes = fixture();
        self.with_bytes(name, &bytes)
    }

    fn disks(&self) -> UserDisks {
        UserDisks { dir: self.dir.clone(), prefer: None }
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The cascade, asked about a story on `story_path` with `media` standing in for
/// the player's own `~/.lanthorn/`.
fn cascade(story_path: &Path, profile: P, art_scale: (u32, u32), disks: Option<&UserDisks>) -> FaceSet {
    app::native_font::resolve(&FaceRequest {
        story_path,
        entry: None,
        profile,
        source: app::interpreter::ProfileSource::Medium,
        art_scale: Some(art_scale),
        disks,
    })
}

fn named(faces: &[(String, BitmapFont)], name: &str) -> BitmapFont {
    faces
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("{name} is in the ROM: {:?}", faces.iter().map(|(n, _)| n).collect::<Vec<_>>()))
        .1
        .clone()
}

// ── 1. finding a face in a ROM ──────────────────────────────────────────────

/// **A ROM is scanned by SHAPE, and the base comes from the image's length.**
///
/// Nothing here is pinned to an address or a revision: 1.2/1.3 are 256 KiB
/// mapped at `$FC0000` and 2.0+ are 512 KiB at `$F80000`, and both are simply
/// `$1000000` minus their own size — which is the rule `rom_base` states. A
/// length no Kickstart comes in has no base and is never scanned.
#[test]
fn a_kickstart_maps_where_its_own_length_says_it_does() {
    assert_eq!(blorb::amiga_font::rom_base(KICK_256K), Some(0x00FC_0000));
    assert_eq!(blorb::amiga_font::rom_base(512 * 1024), Some(0x00F8_0000));
    assert_eq!(blorb::amiga_font::rom_base(1024 * 1024), Some(0x00F0_0000));
    for wrong in [0, 1, 1024, 128 * 1024, KICK_256K - 1, KICK_256K + 1, 880 * 1024] {
        assert_eq!(blorb::amiga_font::rom_base(wrong), None, "{wrong} is no Kickstart");
    }
}

/// **Every `TextFont` in the image, named `<drawer>/<size>`.**
///
/// The `<drawer>/<size>` spelling is not decoration: it is exactly what
/// `blorb::amiga_font::drawer_of` reads off a `FONTS:` path, so a face out of ROM
/// is ranked by the SAME name rule as a face off a floppy rather than by a second
/// one. SQ-1011 shipped inert twice over a rule that lived in two places.
#[test]
fn the_rom_yields_its_faces_named_by_drawer_and_size() {
    let raw = fixture();
    let faces = blorb::amiga_font::faces_in_rom(&raw);
    let names: Vec<&str> = faces.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, [TOPAZ_8, TOPAZ_9, RUBY_8], "the fixture's three discriminators");

    let topaz8 = named(&faces, TOPAZ_8);
    assert_eq!((topaz8.width, topaz8.height), (8, 8), "the face the machine drew with");
    assert!(!topaz8.proportional, "…fixed pitch, which is what topaz is");
    assert_eq!((named(&faces, TOPAZ_9).width, named(&faces, TOPAZ_9).height), (10, 9));
    assert_eq!((named(&faces, RUBY_8).width, named(&faces, RUBY_8).height), (8, 8));

    for name in [TOPAZ_8, TOPAZ_9] {
        assert_eq!(blorb::amiga_font::drawer_of(name), Some("topaz"), "{name}");
    }
    assert_eq!(blorb::amiga_font::drawer_of(RUBY_8), Some("ruby"));

    // The GLYPHS came through, not merely the header: the generator draws row `y`
    // of code `c` as the byte `(c + y) & 0xFF`, and every column of the strike is
    // that byte, so a decoding error anywhere between `tf_CharLoc` and the blit
    // shows up here rather than as a font of blanks.
    let a = topaz8.glyph(b'A').expect("the face covers printable ASCII");
    assert_eq!(a.width, 8, "a fixed face advances by `tf_XSize`");
    assert_eq!(a.rows, (0u8..8).map(|y| b'A' + y).collect::<Vec<u8>>(), "glyph A, row by row");

    // Non-vacuity for the name SEARCH: the fixture puts `topaz/8`'s name pointer
    // in one slot of the uninitialised `tf_Message` preamble and `topaz/9`'s in
    // another, because a ROM image does not set those link fields and blorb looks
    // for the name rather than indexing a fixed offset. Both were found.
    assert_eq!(faces.len(), 3, "including the one whose name sits in the other slot");
}

// ── 2. the face space is the FACE's, not the machine row's ──────────────────

/// **One machine, two faces, two scales** (SQ-1053).
///
/// The Amiga's own releases author a face in the 320-wide picture space — that is
/// `V6FaceSpace::Art`, and it is what makes Arthur's ten face rows a twenty-row
/// declared line. Its OPERATING SYSTEM face is drawn in the 640x200 hires mode,
/// which is `Hires`. A single number per machine can express one of those and
/// silently mis-scales the other; the Macintosh cannot falsify it, because
/// Geneva and Monaco agree.
#[test]
fn a_faces_scale_follows_its_provenance_and_not_the_machine_row() {
    assert_eq!(
        P::Amiga.release_face_space().text_scale((2, 2)),
        (2, 2),
        "a release face doubles with the artwork it is drawn beside",
    );
    assert_eq!(
        P::Amiga.system_face_space().text_scale((2, 2)),
        (1, 2),
        "the system face is 1:1 across the hires 640 and doubled down the 200 rows",
    );
    // Undoubled, the two coincide — the case that proves the vertical two is the
    // ARTWORK's own doubling and not a second constant.
    assert_eq!(P::Amiga.system_face_space().text_scale((1, 1)), (1, 1));

    // The Macintosh answers the same either way, which is why this went unnoticed
    // until a machine had a system face to read.
    for space in [P::Macintosh.release_face_space(), P::Macintosh.system_face_space()] {
        assert_eq!(space.text_scale((2, 2)), (1, 1), "the Macintosh paints text at 1:1");
    }
}

/// **`fit` admits topaz 8 because it IS the cell once drawn — and only then.**
///
/// The three tests are separate on purpose. Judged at the RELEASE space of (2, 2)
/// the same face measures 16x16 native against an 8x16 cell and is refused, which
/// is the non-vacuity that makes the admission a claim about the scale rather
/// than about the bytes.
#[test]
fn topaz_eight_is_the_amigas_cell_once_it_is_drawn() {
    let raw = fixture();
    let faces = blorb::amiga_font::faces_in_rom(&raw);
    let topaz8 = named(&faces, TOPAZ_8);
    let topaz9 = named(&faces, TOPAZ_9);

    let system = P::Amiga.system_face_space().text_scale((2, 2));
    let release = P::Amiga.release_face_space().text_scale((2, 2));
    assert_eq!((system, release), ((1, 2), (2, 2)), "non-vacuity: the two scales differ");

    assert_eq!(
        app::native_font::fit(&topaz8, P::Amiga, system),
        Some(FaceFit::Cell),
        "8x8 at (1, 2) is the 8x16 cell exactly",
    );
    assert_eq!(
        app::native_font::fit(&topaz8, P::Amiga, release),
        None,
        "…and at the artwork's own scale it is a 16x16 face on an 8-wide cell",
    );
    assert_eq!(
        app::native_font::fit(&topaz9, P::Amiga, system),
        None,
        "topaz 9 is 10x9 — eighteen native rows against sixteen — so it declines",
    );
    // The `Cell` verdict is not a licence to move what the STORY is told: a fixed
    // face IS the declared cell and never replaces it (SQ-1009's rule, unchanged).
    let set = cascade(Path::new("/nonexistent.z6"), P::Amiga, (2, 2), None);
    assert_eq!(
        app::native_font::declared_cell(P::Amiga, &set, (2, 2)),
        P::Amiga.v6_font_cell(),
        "no face at all still declares the machine's 8x16",
    );
}

// ── 3. the cascade end to end ───────────────────────────────────────────────

/// **An Amiga story with a Kickstart under `~/.lanthorn/` draws in topaz 8.**
///
/// This is the outcome the quest exists for. `Shogun` and `Zork Zero` ship no
/// face on their Amiga floppies, so before this they drew in `crate::render::vga16`
/// however many Workbench disks the player owned.
///
/// Falsified by returning `V6FaceSpace::Art` from `V6SystemFace::face_space`:
/// the body face comes back `None` and the renderer silently keeps `vga16`,
/// which is the symptom reported.
#[test]
fn the_cascade_draws_an_amiga_story_with_rom_topaz() {
    let media = Media::new("cascade").with_rom("Kick12.rom");
    // A path with no medium behind it, so ONLY the system rung can answer — the
    // release rung has nothing to read and the built-in is no face at all.
    let faces = cascade(Path::new("/nonexistent.z6"), P::Amiga, (2, 2), Some(&media.disks()));

    let body = faces.body().expect("the ROM answered");
    assert_eq!((body.width, body.height), (8, 8), "topaz 8");
    assert_eq!(
        faces.body_origin(),
        Some(&FaceOrigin::SystemDisk { disk: "Kick12.rom".to_string(), name: TOPAZ_8.to_string() }),
        "and the report names the ROM and the face, not just 'a system face'",
    );

    let tf = TextFace::new(P::Amiga, faces, Some((2, 2)));
    assert_eq!(tf.fit(), Some(FaceFit::Cell), "a fixed face IS the cell");
    assert_eq!(tf.scale(), (1, 2), "drawn one native pixel across and two down");
    assert_eq!(tf.cell(), P::Amiga.v6_font_cell(), "the story is still told 8x16");
    assert!(!tf.proportional(), "and the pen is the cell's, not a typeface's");
    for ch in ['i', 'm', 'W', ' '] {
        assert_eq!(tf.advance(ch), 8, "{ch:?}: a fixed-pitch pen steps one cell");
    }
    assert!(
        tf.draws_scaled(0),
        "the renderer takes the scaled blit — the cell path would decline an 8-row face",
    );
    assert_eq!(tf.line_px(), 16, "eight face rows, each drawn twice — the capture's 10-of-20");
}

/// **A Kickstart under a Macintosh story says nothing at all.**
///
/// The same "present but never used" confusion SQ-1018 cost a bug report for,
/// one layer out — and the guard that keeps a ROM face from leaking onto a
/// machine that never had one.
#[test]
fn a_kickstart_answers_only_the_amiga() {
    let media = Media::new("machine").with_rom("Kick12.rom");
    let all = app::system_fonts::scan(&media.dir);
    assert_eq!(all.len(), 3, "non-vacuity: the ROM really does parse: {all:?}");
    assert!(all.iter().all(|f| f.machine == P::Amiga), "every row is the Amiga's: {all:?}");
    assert_eq!(app::system_fonts::scan_for(&media.dir, P::Macintosh), Vec::new());
    assert_eq!(app::system_fonts::scan_for(&media.dir, P::IbmPc), Vec::new());

    let faces = cascade(Path::new("/nonexistent.z6"), P::Macintosh, (1, 1), Some(&media.disks()));
    assert_eq!(faces, FaceSet::none(), "a Macintosh story reads a Kickstart and draws nothing");
}

/// **The NAME filter is still the only thing keeping a display face out**, and it
/// is load-bearing rather than decorative (SQ-1037).
///
/// The fixture's `ruby/8` is byte-for-byte the geometry of `topaz/8`, so it
/// passes `fit` outright — asserted here, because a decoy that failed for some
/// other reason would make this case prove nothing. What refuses it is the
/// machine's own `V6SystemFace::AmigaDrawer("topaz")`, and nothing else.
#[test]
fn only_the_name_keeps_a_workbench_display_face_out() {
    let media = Media::new("name").with_rom("Kick12.rom");
    let raw = fixture();
    let ruby = named(&blorb::amiga_font::faces_in_rom(&raw), RUBY_8);
    assert_eq!(
        app::native_font::fit(&ruby, P::Amiga, P::Amiga.system_face_space().text_scale((2, 2))),
        Some(FaceFit::Cell),
        "non-vacuity: this face would be admitted on sight if anything offered it",
    );

    let offered = app::system_fonts::named_faces_in(&media.disks(), P::Amiga);
    let names: Vec<&str> = offered.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, [TOPAZ_8, TOPAZ_9], "only the drawer the machine names is offered");

    // And the SIZE rule inside the cascade picks between the two topazes, exactly
    // as it picks Geneva 12 out of a System file's seven sizes.
    let faces = cascade(Path::new("/nonexistent.z6"), P::Amiga, (2, 2), Some(&media.disks()));
    assert_eq!(faces.body().map(|f| (f.width, f.height)), Some((8, 8)), "and 8 wins over 9");
}

/// **And the RENDERER draws it — every face row twice.**
///
/// The cell blit stamps a face 1:1 and filters on `f.height == ch`, so an 8-row
/// face on a 16-row cell fails it and falls silently back to
/// `crate::render::vga16` — a face resolved, admitted, and never seen, which is
/// exactly how SQ-1011 shipped inert twice. `TextFace::draws_scaled` is what
/// routes it to the per-glyph blit instead.
///
/// Falsified by restoring `tf.filter(|t| t.draws_proportionally(style))` in
/// `render::bitfont`: the canvas comes back holding `vga16`'s `A` and none of the
/// face's own rows.
#[test]
fn the_renderer_draws_topaz_at_the_faces_own_scale() {
    let media = Media::new("blit").with_rom("Kick12.rom");
    let faces = cascade(Path::new("/nonexistent.z6"), P::Amiga, (2, 2), Some(&media.disks()));
    let tf = TextFace::new(P::Amiga, faces, Some((2, 2)));
    let (cw, ch) = (u32::from(tf.cell().w()), u32::from(tf.cell().h()));
    assert_eq!((cw, ch), (8, 16), "non-vacuity: an 8-row face on a 16-row cell");

    let ink = image::Rgba([255, 255, 255, 255]);
    let page = image::Rgba([0, 0, 0, 255]);
    let mut canvas = image::RgbaImage::from_pixel(cw, ch, page);
    app::render::bitfont::blit_glyph_styled(&mut canvas, 'A', 0, 0, cw, ch, ink, Some(page), 0, Some(&tf));

    // The generator draws row `y` of code `c` as the byte `(c + y) & 0xFF`,
    // MSB-leftmost — so canvas rows 2y and 2y+1 are both face row y.
    let painted: Vec<u8> = (0..ch)
        .map(|y| {
            (0..cw).fold(0u8, |acc, x| {
                acc | if canvas.get_pixel(x, y) == &ink { 0x80 >> x } else { 0 }
            })
        })
        .collect();
    let want: Vec<u8> = (0..8u8).flat_map(|y| [b'A' + y, b'A' + y]).collect();
    assert_eq!(painted, want, "the face's own eight rows, each drawn twice");
}

// ── 4. the release's own face still outranks the machine's ─────────────────

/// **Arthur's floppy still wins.** Rung 1 is the release's own medium and a ROM
/// is rung 2, so a game that shipped a face keeps drawing in it.
///
/// Release **54 / serial 890606**, `stories/Arthur - The Quest for Excalibur.adf`,
/// resolved at launch (no turns driven — the cascade reads the medium, not the
/// screen). The floppy is gitignored, so this skips vacuously; the advance table
/// and the twenty-row line it produces are pinned by
/// `v6_arthur_amiga_proportional`, which must stay green alongside this.
#[test]
fn arthurs_own_face_outranks_a_kickstart() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../stories/Arthur - The Quest for Excalibur.adf");
    if !path.is_file() {
        eprintln!("SKIP: gitignored floppy absent at {}", path.display());
        return;
    }
    let media = Media::new("arthur").with_rom("Kick12.rom");
    let (profile, source) = app::interpreter::InterpreterProfile::resolve_with_source(&path, None, None, None);
    assert_eq!(profile, P::Amiga, "non-vacuity: the medium names the Amiga");

    let disks = media.disks();
    let faces = app::native_font::resolve(&FaceRequest {
        story_path: &path,
        entry: None,
        profile,
        source,
        art_scale: Some((2, 2)),
        disks: Some(&disks),
    });
    assert_eq!(faces.body_origin(), Some(&FaceOrigin::Release), "the release's own face wins");
    assert_eq!(
        faces.body().map(|f| (f.width, f.height)),
        Some((10, 10)),
        "char.data, not topaz",
    );
    let tf = TextFace::new(profile, faces, Some((2, 2)));
    assert_eq!(tf.scale(), (2, 2), "…and it keeps the RELEASE space, which doubles it");
    assert_eq!(tf.cell(), zvm::screen::V6Cell::new(8, 20), "the twenty-row line, unchanged");
}

/// **The real-game smoke: *Shogun* and *Zork Zero* on the Amiga.**
///
/// The two Version 6 releases that ship no face on their own Amiga floppies, and
/// therefore the two the machine's system topaz exists for. Booted exactly as
/// `startup.rs` boots — the profile from the medium the mount returned, the
/// screen-size chain, `art_scale` alongside — with the SYNTHETIC ROM standing in
/// for the player's Kickstart, so nothing here depends on `~/.lanthorn/`.
///
/// No turns are driven: the cascade reads the medium and the machine, not the
/// screen, so the frame this would reach is irrelevant to what it asserts. The
/// releases are pinned below because a floppy is a fixture with a release on it.
///
/// `stories/` is gitignored, so this skips vacuously.
#[test]
fn an_amiga_release_with_no_face_of_its_own_boots_on_topaz() {
    let media = Media::new("smoke").with_rom("Kick12.rom");
    let mut drove = 0;
    for (disk, release, serial) in [
        ("James Clavell's Shogun.adf", 295u16, "890321"),
        ("Zork Zero - The Revenge of Megaboz.adf", 366u16, "890323"),
    ] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(disk);
        let Ok((loaded, _)) = app::hints::load_mounted_story(&path) else {
            eprintln!("SKIP: gitignored floppy absent: {disk}");
            continue;
        };
        let bytes = loaded.bytes().to_vec();
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), release, "{disk}: release");
        assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), serial, "{disk}: serial");

        let (profile, source) =
            app::interpreter::InterpreterProfile::resolve_with_source(&path, None, None, None);
        assert_eq!(profile, P::Amiga, "{disk}: the medium names the Amiga");
        let picts = app::graphics::PictSource::resolve(&path, None);
        let disks = media.disks();
        let faces = app::native_font::resolve(&FaceRequest {
            story_path: &path,
            entry: None,
            profile,
            source,
            art_scale: picts.art_scale(),
            disks: Some(&disks),
        });
        // Non-vacuity, and the whole reason this rung exists: the floppy itself
        // carries no face the machine can draw prose with.
        let alone = app::native_font::resolve(&FaceRequest {
            story_path: &path,
            entry: None,
            profile,
            source,
            art_scale: picts.art_scale(),
            disks: None,
        });
        assert_eq!(alone, FaceSet::none(), "{disk}: the release ships no usable face");

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
        // The picture-space window, which the art scale turns into the 640x400 unit
        // screen — printed so a reader can check it against a `/dump-windows`
        // capture rather than trusting a number this file produced (CLAUDE.md).
        eprintln!(
            "{disk}: r{release}/{serial}, {profile:?}, picture window {:?} x art {:?} \
             = 640x400 native, cell {}x{}",
            machine.screen_px, machine.art_scale, machine.cell.w(), machine.cell.h(),
        );
        assert_eq!(machine.art_scale, Some((2, 2)), "{disk}: a 320-wide press doubles");
        assert_eq!(
            machine.cell,
            P::Amiga.v6_font_cell(),
            "{disk}: the story is still told 8x16 — a fixed face declares nothing",
        );
        let tf = machine.text_face();
        assert_eq!(
            tf.face().map(|f| (f.width, f.height)),
            Some((8, 8)),
            "{disk}: drawn in topaz 8",
        );
        assert_eq!(tf.scale(), (1, 2), "{disk}: at the hires text scale");
        assert!(tf.draws_scaled(0), "{disk}: through the scaled blit");
        drove += 1;
    }
    if drove == 0 {
        eprintln!("SKIP: no Amiga v6 floppy present");
    }
}

// ── 5. a ROM is untrusted input ────────────────────────────────────────────

/// **A malformed, truncated or hostile ROM faults quietly** (SQ-1053).
///
/// `~/.lanthorn/` is whatever the player put there, and a ROM dump is a file off
/// the internet more often than not. None of these may panic, hang or allocate
/// without bound, and none may stop a game from starting.
#[test]
fn a_hostile_rom_is_refused_rather_than_trusted() {
    let good = fixture();

    let mut truncated = good.clone();
    truncated.truncate(1024);
    let mut flipped: Vec<u8> = good.iter().map(|b| !b).collect();
    // …still the right LENGTH, so it reaches the identification rather than the
    // size rule: structure-shaped, every value wrong.
    flipped.truncate(KICK_256K);
    let mut no_jump = good.clone();
    no_jump[2] = 0x00;
    let mut wild = good.clone();
    // `topaz/8`'s `tf_CharData`, pointed outside the mapped ROM entirely.
    wild[0x0200 + 34..0x0200 + 38].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
    let mut absurd = good.clone();
    // …and a record claiming a face taller and wider than any Amiga font, with a
    // strike stride to match. The allocation this would demand is the thing a
    // bound exists for.
    absurd[0x0200 + 20..0x0200 + 22].copy_from_slice(&0xFFFFu16.to_be_bytes());
    absurd[0x0200 + 24..0x0200 + 26].copy_from_slice(&0xFFFFu16.to_be_bytes());
    absurd[0x0200 + 38..0x0200 + 40].copy_from_slice(&0xFFFFu16.to_be_bytes());
    // Identified as a Kickstart and junk from there on: `tf_Flags` reads as a ROM
    // font at EVERY offset, so this walks the geometry guard 131,071 times.
    let mut junk = good.clone();
    junk[0x100..].fill(0xFF);

    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("truncated", truncated),
        ("all zero", vec![0u8; KICK_256K]),
        ("all ones", vec![0xFFu8; KICK_256K]),
        ("byte-flipped", flipped),
        ("no opening JMP", no_jump),
        ("a story file", b"\x08\x00\x00\x00garbage".repeat(64)),
        ("wrong length", good[..KICK_256K - 2].to_vec()),
        ("identified but junk", junk),
    ];
    for (what, bytes) in &cases {
        assert_eq!(
            blorb::amiga_font::faces_in_rom(bytes),
            Vec::new(),
            "{what}: refused rather than trusted",
        );
    }

    // A bad field is refused per-FACE, not per-image: the two records beside it
    // still parse, which is the honest answer and the one a player can act on.
    for (what, bytes) in [("a wild pointer", &wild), ("absurd geometry", &absurd)] {
        let got: Vec<String> =
            blorb::amiga_font::faces_in_rom(bytes).into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            got,
            vec![TOPAZ_9.to_string(), RUBY_8.to_string()],
            "{what}: took its own face and no other",
        );
    }

    // And end to end, through the directory scan a launch really performs: a
    // directory of nothing but hostile ROMs answers empty and does not panic.
    let media = Media::new("hostile");
    let mut media = media;
    for (i, (_, bytes)) in cases.iter().enumerate() {
        media = media.with_bytes(&format!("bad{i}.rom"), bytes);
    }
    assert_eq!(app::system_fonts::scan(&media.dir), Vec::new());
    assert_eq!(
        cascade(Path::new("/nonexistent.z6"), P::Amiga, (2, 2), Some(&media.disks())),
        FaceSet::none(),
        "a launch with a drawer full of junk draws exactly what it drew before",
    );
}

// ── 6. the player's own Kickstart, when they have one ──────────────────────

/// **The oracle: a REAL Kickstart yields topaz 8.**
///
/// Every case above runs against bytes this repo wrote, which can only tell us
/// the code does what it was asked. This one asks whether the question was right,
/// against `~/.lanthorn/*.rom` — the player's own media, so it skips vacuously
/// exactly as a `stories/` case does, and it must never be the only thing pinning
/// a claim.
///
/// Measured on the machine this was developed against: `Kick12.rom`, 262,144
/// bytes, Kickstart 1.2, base `$FC0000` — two `TextFont` records, `topaz/8` (8x8)
/// and `topaz/9` (10x9), both naming `topaz.font`, and no false positives across
/// the whole 256 KiB.
#[test]
fn a_real_kickstart_yields_topaz_eight() {
    let dir = app::system_fonts::user_media_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("SKIP: no {} at all", dir.display());
        return;
    };
    let roms: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("rom")))
        .collect();
    if roms.is_empty() {
        eprintln!("SKIP: no Kickstart ROM under {}", dir.display());
        return;
    }
    let mut found = false;
    for rom in &roms {
        let Ok(bytes) = std::fs::read(rom) else { continue };
        let faces = blorb::amiga_font::faces_in_rom(&bytes);
        let names: Vec<&str> = faces.iter().map(|(n, _)| n.as_str()).collect();
        eprintln!("{}: {} bytes, faces {names:?}", rom.display(), bytes.len());
        if blorb::amiga_font::rom_base(bytes.len()).is_none() {
            continue; // not a Kickstart-sized dump; the length rule already said so
        }
        assert!(
            faces.iter().any(|(n, f)| n == TOPAZ_8 && (f.width, f.height) == (8, 8)),
            "{}: a Kickstart carries topaz 8, and this found {names:?}",
            rom.display(),
        );
        assert!(
            faces.len() <= 4,
            "{}: a shape scan that matched half the ROM is not a shape scan: {names:?}",
            rom.display(),
        );
        found = true;
    }
    if !found {
        return; // every `.rom` there was some other kind of dump
    }

    // And through the PRODUCTION lookup, over the player's real directory —
    // Workbench floppies, System disks and all. This is the whole path `startup.rs`
    // walks: the extension pre-filter, the mount that a ROM never reaches, the
    // machine's name, the declared line height, and `fit`.
    let disks = UserDisks { dir: dir.clone(), prefer: None };
    let offered = app::system_fonts::named_faces_in(&disks, P::Amiga);
    let names: Vec<&str> = offered.iter().map(|f| f.name.as_str()).collect();
    eprintln!("offered to an Amiga story from {}: {names:?}", dir.display());
    assert!(
        offered.iter().any(|f| f.name == TOPAZ_8),
        "the machine's own face is offered off the player's own media: {names:?}",
    );
    for face in &offered {
        assert_eq!(
            blorb::amiga_font::drawer_of(&face.name).map(str::to_ascii_lowercase),
            Some("topaz".to_string()),
            "only the drawer the machine names is ever offered — {} is not it",
            face.name,
        );
    }
    let faces = cascade(Path::new("/nonexistent.z6"), P::Amiga, (2, 2), Some(&disks));
    assert_eq!(
        faces.body().map(|f| (f.width, f.height)),
        Some((8, 8)),
        "…and an Amiga story with no face of its own draws in topaz 8",
    );
}
