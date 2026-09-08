//! SQ-0794 — an EGA or CGA rendition is drawn through the colours its video card
//! fixed, not through the archive's stock fallback.
//!
//! REPORTED, on `stories/zork0-r393-s890714.z6` with `pictures = "zork0.eg1"`,
//! once SQ-0790 had the plate landing at its true aspect: Zork Zero's proscenium
//! arch came out pink and olive where the same frame from `zork0.mg1` is bronze,
//! and the CGA rendition was worse — flat green and cyan line art.
//!
//! MEASURED CAUSE, which `blorb::infocom_pics` had already written down before
//! there was a symptom. A PC EGA or CGA directory record is **12 bytes**, with a
//! pad byte where MCGA and the Amiga keep a 3-byte palette pointer, because
//! those adapters fixed their colours in the video hardware and there was nothing
//! to store. So every picture in such an archive answers "no palette of my own",
//! `PictSource` read that as Blorb §11.3 *adaptive*, no non-adaptive draw ever
//! established a Current Palette, and all of it expanded through
//! `DEFAULT_PALETTE` — which is the EGA table with **one entry wrong**, index 6
//! dark yellow `(170, 170, 0)` where the hardware shows brown `(170, 85, 0)`.
//! Zork Zero's EGA artwork dithers that brown against bright red to make its
//! bronze, so one entry took the whole plate olive. CGA fared worse still:
//! `DEFAULT_PALETTE` slots 2 and 3 are green and cyan, and a `.CG1` uses
//! precisely 2 and 3.
//!
//! The fix is `InfocomPics::hardware_palette`, which reads the rendition off the
//! directory (no picture carries a palette ⇒ EGA or CGA; every picture with
//! pixels sets `EF_MONO` ⇒ CGA), and a `PictSource` that draws through it and
//! keeps such an archive out of the adaptive machinery entirely.
//!
//! MCGA (`.MG1`) and the Amiga/Mac `Pic.data` carry real per-picture palettes and
//! are untouched — pinned below rather than assumed.
//!
//! Every fixture here is gitignored, so each case **skips vacuously** when absent.
//!
//! **Palette**: `boot_named` names the table on the `MachineBoot` it resolves, so
//! every session here carries its own (SQ-1393); the two archive-only cases name
//! none, because they never resolve a colour number. Nothing is process-wide any
//! more, so no case can move the table another case reads.

use std::collections::BTreeSet;
use std::path::PathBuf;

use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};
use blorb::infocom_pics::{InfocomPics, Rgb, CGA_PALETTE, DEFAULT_PALETTE, EGA_PALETTE};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn read(name: &str) -> Option<Vec<u8>> {
    let p = stories_dir().join(name);
    match std::fs::read(&p) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored fixture missing at {}", p.display());
            None
        }
    }
}

fn native(name: &str) -> Option<PictSource> {
    Some(PictSource::from_native(
        InfocomPics::parse(read(name)?).expect("a native Infocom archive parses"),
    ))
}

/// Boot Zork Zero against `archive` and play far enough for the border to be up,
/// exactly as `startup.rs` builds a session: the archive supplies the standard
/// window and the per-axis art scale, and nothing else differs between
/// renditions.
fn boot(archive: &str, honor_game_colours: bool) -> Option<GameSession> {
    let story = read("zork0-r393-s890714.z6")?;
    let mut picts = native(archive)?;
    let picture_dims = picts.all_pict_dims();
    // The chain `startup.rs` runs: a Blorb's `Reso`, else the archive's own
    // picture space (SQ-0838 — 320x200 for MCGA/Amiga, 640x200 for EGA/CGA, and
    // 480x300 for the standard Macintosh's mono plate). The screen is that space
    // times the density below, which is 640x400 for every rendition here.
    // SQ-1021/SQ-1022: the machine's facts in one value. No medium names a machine
    // here, so the profile is `IbmPc` — which is what the `None` interpreter number
    // and `None` colours below already meant, and whose `std_window()` is `None`, so
    // completing the chain changes nothing and stops it being a chain.
    let boot = app::machine_boot::MachineBoot::resolve(
        app::interpreter::InterpreterProfile::IbmPc,
        &picts,
        None,
        None,
        None,
        true,
        app::native_font::FaceSet::none(),
        zvm::screen::Palette::Standard,
        None,
    );
    let mut session = GameSession::new_for_machine(
        story,
        honor_game_colours,
        false,
        false,
        picture_dims,
        None,
        None,
        &boot,
    )
    .expect("Zork Zero (v6) loads and boots without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..2 {
        match session.pending_input() {
            InputKind::Line => session.submit("look"),
            InputKind::Char => session.submit_char(b' '),
            InputKind::Event => session.submit(""),
        };
    }
    Some(session)
}

/// Every opaque colour on Zork Zero's full-screen border canvas (window 7).
fn frame_colours(session: &GameSession) -> BTreeSet<Rgb> {
    let c = session.pictures_canvas.get(&7).expect("Zork Zero's border is window 7");
    c.img
        .pixels()
        .filter(|p| p.0[3] != 0)
        .map(|p| [p.0[0], p.0[1], p.0[2]])
        .collect()
}

/// Every colour a `[1, 2, 1] / 4` tent of three EGA entries can produce — the set
/// SQ-0797's column fusion can legally put on an EGA frame, and nothing wider.
/// Rounding matches `graphics::blend_half_width_columns` exactly (`(sum + 2) / 4`,
/// per channel).
fn ega_tent_closure() -> BTreeSet<Rgb> {
    let mut out = BTreeSet::new();
    for l in EGA_PALETTE {
        for m in EGA_PALETTE {
            for r in EGA_PALETTE {
                let mut c = [0u8; 3];
                for k in 0..3 {
                    let sum = u32::from(l[k]) + 2 * u32::from(m[k]) + u32::from(r[k]);
                    c[k] = ((sum + 2) / 4) as u8;
                }
                out.insert(c);
            }
        }
    }
    out
}

/// The reported defect, on the reported fixture: the EGA arch is bronze, which
/// on a sixteen-colour card means brown dithered against bright red.
///
/// Falsified by restoring the fallback (`native_image(pics, resnum, None)` in
/// `PictSource::get`): brown vanishes from the frame and dark yellow takes its
/// place — 18202 pixels of it, the exact count SQ-0794 measured — which is the
/// olive the report describes.
#[test]
fn zork_zeros_ega_frame_is_drawn_in_ega_brown_not_dark_yellow() {
    for honor_game_colours in [true, false] {
        let Some(session) = boot("zork0.eg1", honor_game_colours) else { return };
        let seen = frame_colours(&session);
        assert!(
            seen.contains(&EGA_PALETTE[6]),
            "honor={honor_game_colours}: the EGA frame must contain brown {:?}",
            EGA_PALETTE[6]
        );
        assert!(
            !seen.contains(&DEFAULT_PALETTE[6]),
            "honor={honor_game_colours}: dark yellow {:?} is the defect, and must be gone",
            DEFAULT_PALETTE[6]
        );
        // Nothing outside the card's sixteen may reach the frame — but "the
        // card's sixteen" now means what an EGA CARD put on the glass, not what
        // the archive stored. SQ-0797 fuses a 640-wide rendition's column dither
        // with a `[1, 2, 1] / 4` tent, because its pixels are half as wide, and
        // bronze is by construction a colour the palette does not contain. So the
        // claim is one step weaker and still catches every palette defect: every
        // frame colour must be a tent of THREE EGA entries. Dark yellow reaching
        // the artwork would have to arrive as some blend of the sixteen, and the
        // assertion below rules it out at full strength either way.
        let fused = ega_tent_closure();
        for c in &seen {
            assert!(
                fused.contains(c),
                "honor={honor_game_colours}: {c:?} is not a tent of EGA colours"
            );
        }
    }
}

/// CGA is two colours, not four: its only 640-wide mode was 640x200 mode 6. The
/// archive's indices 2 and 3 are the lit state and black, and `DEFAULT_PALETTE`
/// renders them green and cyan.
///
/// **The lit state is the CARD's light grey**, `#AAAAAA`, not pure white
/// (SQ-0956): `machine-screenshots/dos-zorkzero-cga.png` measures `#A0A0A0` for
/// every lit pixel of the frame, artwork and text alike, which is a DOS
/// emulator's rendering of EGA entry 7. The Macintosh's mono plate keeps a real
/// white and is the control — `v6_macintosh_profile` holds that half.
///
/// Falsified the same way as the EGA case: the frame comes back as
/// `(0, 170, 0)` and `(0, 170, 170)` and nothing else.
#[test]
fn zork_zeros_cga_frame_is_the_cards_two_states() {
    for honor_game_colours in [true, false] {
        let Some(session) = boot("zork0.cg1", honor_game_colours) else { return };
        let seen = frame_colours(&session);
        assert_eq!(
            seen,
            BTreeSet::from([[0, 0, 0], [170, 170, 170]]),
            "honor={honor_game_colours}: a CGA frame is the card's two states only"
        );
        assert!(!seen.contains(&DEFAULT_PALETTE[2]), "the fallback's green is the defect");
        assert!(!seen.contains(&DEFAULT_PALETTE[3]), "the fallback's cyan is the defect");
    }
}

/// The other half of the fix, and the one the Current-Palette machinery cares
/// about (SQ-0743, Blorb §11.3): an EGA or CGA picture is not adaptive. Its
/// record has no palette pointer to be silent with — the colours were in the
/// card — so nothing may ever splice a `PLTE` into it, including a Current
/// Palette carried in from a host Save State.
#[test]
fn a_hardware_palette_archive_has_no_adaptive_pictures() {
    for (archive, hardware) in [
        ("zork0.eg1", true),
        ("zork0.cg1", true),
        ("zork0.mg1", false),
        ("zork0.pic", false),
    ] {
        let Some(raw) = read(archive) else { continue };
        let pics = InfocomPics::parse(raw).expect("parses");
        let adaptive_by_record = pics.adaptive_pictures();
        assert!(
            !adaptive_by_record.is_empty(),
            "{archive}: every one of these archives has palette-less records — that is the trap"
        );
        assert_eq!(pics.hardware_palette().is_some(), hardware, "{archive} hardware table");

        let src = PictSource::from_native(InfocomPics::parse(read(archive).unwrap()).unwrap());
        for id in adaptive_by_record {
            assert_eq!(
                src.is_adaptive(u32::from(id)),
                !hardware,
                "{archive} picture {id}: a hardware rendition defers to nobody, an MCGA/Amiga one does"
            );
        }
    }
}

/// MCGA and the Amiga are UNCHANGED. Both carry their colours per picture, so
/// neither takes a hardware table and neither loses a single adaptive picture —
/// and the frame Zork Zero's MCGA archive draws is still its bronze, in the
/// 12-bit palette the format stores (every channel a multiple of 17), with none
/// of the EGA card's four levels showing through.
#[test]
fn the_mcga_and_amiga_renditions_are_untouched() {
    for archive in ["zork0.mg1", "zork0.pic"] {
        let Some(raw) = read(archive) else { continue };
        let pics = InfocomPics::parse(raw).expect("parses");
        assert_eq!(pics.hardware_palette(), None, "{archive} carries its own colours");
        // Zork Zero's 16 compass overlays are the canonical adaptive set (SQ-0743).
        let adaptive = pics.adaptive_pictures();
        for id in 9..=24u16 {
            assert!(adaptive.contains(&id), "{archive}: compass overlay {id} is still adaptive");
        }
    }

    for honor_game_colours in [true, false] {
        let Some(session) = boot("zork0.mg1", honor_game_colours) else { return };
        let seen = frame_colours(&session);
        assert!(
            seen.iter().all(|c| c.iter().all(|ch| ch % 17 == 0)),
            "honor={honor_game_colours}: an MCGA palette is 4 bits per channel"
        );
        assert!(
            seen.len() > 4,
            "honor={honor_game_colours}: the MCGA frame is a full bronze ramp, not two colours"
        );
        assert!(
            !seen.contains(&CGA_PALETTE[1]) && !seen.contains(&EGA_PALETTE[6]),
            "honor={honor_game_colours}: no hardware table may leak into an MCGA frame"
        );
    }
}

/// SQ-0806 — a two-colour rendition tells the story the interpreter has no
/// colours, so it never asks for the white page that would paint out its own art.
///
/// REPORTED, on `stories/zork0-r393-s890714.z6` with `pictures = "zork0.cg1"`:
/// the background comes out white and the black-and-white border art blends into
/// it.
///
/// MEASURED CAUSE. A `.CG1` is a two-colour STENCIL. On Zork Zero's border:
/// 46,336 opaque white pixels, 17,152 opaque black, and 192,512 transparent —
/// one row through the pillars runs transparent at the screen edge, opaque white
/// for the column's lit face, transparent again across the story area. Its white
/// is PAINT and its transparency reveals a colour the artwork never stored; a
/// white page destroys both. Zork Zero asks for one regardless of card, because
/// it issues `set_colour(fg=2, bg=9)` for every video card alike and the story
/// file cannot see which archive was loaded — pinned below.
///
/// The lever is `honor_game_colours`, which already means "declare the
/// interpreter colourless" (§8.3.2). NOT the interpreter number: header `$1E`
/// steers far more of a v6 game than colour, and advertising 1 (DECSystem-20)
/// costs Shogun its whole right border.
#[test]
fn a_two_colour_rendition_is_told_the_interpreter_has_no_colours() {
    use zvm::screen::ZColour;

    // The premise: told it has colours, the game sets the same pair whatever the
    // card — so nothing about the ARCHIVE is what stops it.
    for archive in ["zork0.cg1", "zork0.eg1", "zork0.mg1"] {
        let Some(session) = boot(archive, true) else { return };
        let v6 = session.machine.screen.v6.as_ref().expect("v6 window table");
        assert_eq!(
            (v6.windows[0].fg, v6.windows[0].bg),
            (ZColour::Standard(2), ZColour::Standard(9)),
            "{archive}: Zork Zero sets black-on-white whatever the card"
        );
    }

    // …and told it has none, it asks for nothing at all — on windows 0 (story),
    // 1 (status) and 7 (border) alike, which are Zork Zero's three coloured
    // windows. The host theme then owns the ground the stencil reveals.
    for archive in ["zork0.cg1", "zork0.mg1"] {
        let Some(session) = boot(archive, false) else { return };
        let v6 = session.machine.screen.v6.as_ref().expect("v6 window table");
        for w in [0usize, 1, 7] {
            assert_eq!(
                (v6.windows[w].fg, v6.windows[w].bg),
                (ZColour::Default, ZColour::Default),
                "{archive}: a colourless interpreter is never asked for window {w}'s colours"
            );
        }
    }
}

/// The archive says two-colour from its CONTENT, not from a filename a rename
/// could make a lie — this is what `startup` reads to force the flag off.
#[test]
fn a_cga_archive_reports_itself_monochrome_and_the_others_do_not() {
    for (archive, want) in
        [("zork0.cg1", true), ("zork0.eg1", false), ("zork0.mg1", false), ("zork0.pic", false)]
    {
        let Some(raw) = read(archive) else { continue };
        let pics = InfocomPics::parse(raw).expect("parses");
        assert_eq!(pics.is_monochrome(), want, "{archive}: InfocomPics::is_monochrome");
        let Some(src) = native(archive) else { continue };
        assert_eq!(src.is_monochrome(), want, "{archive}: PictSource::is_monochrome");
    }
}

/// Boot a v6 story against a NAMED archive the way `startup.rs` does, with the
/// interpreter number overridable so the alternative SQ-0806 rejected can be
/// measured rather than asserted. `honour` is the config's value; the archive
/// still gets its say through [`PictSource::declines_game_colours`].
fn boot_named(
    story: &str,
    archive: &str,
    release: (u16, &str),
    honour: bool,
    interpreter: Option<u8>,
) -> Option<GameSession> {
    let pics = InfocomPics::parse(read(archive)?).expect("a native Infocom archive parses");
    let (loaded, _) = app::hints::load_mounted_story(&stories_dir().join(story))
        .map_err(|_| eprintln!("SKIP: gitignored story missing: {story}"))
        .ok()?;
    let bytes = loaded.bytes().to_vec();
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), release.0, "{story}: release");
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), release.1, "{story}: serial");
    let profile = InterpreterProfile::for_art_flavour(pics.flavour());
    let mut picts = PictSource::from_native(pics);
    let honoured = honour
        && !picts.declines_game_colours(profile.default_colours());
    let picture_dims = picts.all_pict_dims();
    // SQ-1021/SQ-1022: every per-machine fact in one value. This site names a
    // profile, so it also gains `profile.std_window()` — the link the hand-written
    // chain above was missing.
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        interpreter.or_else(|| profile.interpreter_number()),
        honoured.then(|| profile.default_colours()).flatten(),
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    let mut session = GameSession::new_for_machine(
        bytes,
        honoured,
        false,
        false,
        picture_dims,
        None,
        None,
        &boot,
    )
    .unwrap_or_else(|e| panic!("{story} + {archive}: should boot without a ZError: {e:?}"));
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..4 {
        match session.pending_input() {
            InputKind::Line => session.submit("look"),
            InputKind::Char => session.submit_char(b' '),
            InputKind::Event => session.submit(""),
        };
    }
    Some(session)
}

/// The opaque pixels of the flank either side of the story window, off the
/// graphics canvas the render composes — `(left, right)`.
fn flank_pixels(session: &GameSession) -> (u64, u64) {
    use app::engine::{Engine as _, WinNode};
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
    let story = layout.story.expect("Shogun's gameplay screen has a story window");
    let (lx, rx) = (story.x_px as u32, story.x_px as u32 + story.w_px as u32);
    let tally = |x0: u32, x1: u32| -> u64 {
        (0..gfx.height())
            .map(|y| (x0..x1).filter(|&x| gfx.get_pixel(x, y)[3] != 0).count() as u64)
            .sum()
    };
    (tally(0, lx), tally(rx, gfx.width()))
}

/// **SHOGUN MUST NOT BE TRADED FOR THE MACINTOSH** — the non-regression SQ-0846
/// owes SQ-0806, pinned as its own case so a later change cannot quietly swap
/// one for the other.
///
/// SQ-0806's comment records the alternative it turned down: *"Through the
/// honour flag rather than the interpreter number, which would look like the
/// tidier fix and is not: header `$1E` steers far more of a v6 game than colour,
/// and advertising 1 (DECSystem-20) costs Shogun its entire RIGHT border."* This
/// case holds both ends of that sentence down.
///
/// **Measured here**, on `shogun-r322-s890706.z6` at the first prompt, counting
/// opaque pixels either side of the story window on the composed graphics canvas:
///
/// | rendition    | interpreter | left flank | right flank |
/// |--------------|-------------|------------|-------------|
/// | `shogun.cg1` | 6 (IBM PC)  | 22,220     | **22,794**  |
/// | `shogun.cg1` | 1 (DEC-20)  | 22,220     | **0**       |
/// | `shogun.eg1` | 6 (IBM PC)  | 18,400     | **18,400**  |
/// | `shogun.eg1` | 1 (DEC-20)  | 18,400     | **0**       |
///
/// so the cost of the tidier fix is not "~11,000 pixels" as the comment
/// estimated — it is the whole flank, on both renditions, with the left one
/// standing untouched beside it to prove the game is still running.
///
/// **And the honour flag is free.** The same measurement is identical with
/// colours honoured and declined, which is exactly why SQ-0806 could reach for
/// it: Shogun's border is a function of the machine it thinks it is on, not of
/// whether it was offered colours.
///
/// The Macintosh carve-out is checked in the same breath, because the way to
/// regress this is to widen it: a `.cg1` named beside a bare story file must
/// still decline colours exactly as it did. Off a DOS PRESS it no longer does —
/// SQ-0956 gives the card a screen of its own instead — and that is the other
/// half asserted here, because the two launches are now two different answers.
#[test]
fn shoguns_flanks_survive_the_rule_that_spared_them() {
    for archive in ["shogun.cg1", "shogun.eg1"] {
        let Some(pics) = read(archive).map(|r| InfocomPics::parse(r).expect("parses")) else {
            continue;
        };
        let profile = InterpreterProfile::for_art_flavour(pics.flavour());
        let src = PictSource::from_native(pics);
        assert_eq!(profile, InterpreterProfile::IbmPc, "{archive}: a DOS rendition is an IBM PC");
        assert_eq!(
            // SQ-0928: a `.cg1`/`.eg1` named beside a bare story file is not
            // original media, so this launch has no machine pair — which is what
            // "a PC states no colours of its own" was really asserting. The IBM PC
            // itself now states blue under white; it just does not reach a launch
            // that only named an archive.
            src.declines_game_colours(None),
            src.is_monochrome(),
            "{archive}: SQ-0806's rule, unmoved — this launch named no machine",
        );
        assert!(
            // SQ-0956: and it answers NO once the launch reaches the machine — off
            // a DOS press, where the medium licenses the IBM PC's pair. Declining
            // there was the defect: the card has a screen (black under light grey,
            // `Palette::IbmCga`) and a story that can still call `set_colour`.
            !src.declines_game_colours(profile.default_colours()),
            "{archive}: a licensed machine keeps its colours — the card states its own",
        );
        assert_eq!(
            src.two_colour_card(),
            src.is_monochrome(),
            "{archive}: and a DOS two-colour archive is a CGA card, which is what \
             installs the two-state palette",
        );

        for honour in [true, false] {
            let Some(kept) = boot_named("shogun-r322-s890706.z6", archive, (322, "890706"), honour, None)
            else {
                continue;
            };
            assert_eq!(
                kept.machine.mem.read_byte(0x1E),
                6,
                "{archive} (honour={honour}): the rule must never move header $1E",
            );
            let (left, right) = flank_pixels(&kept);
            assert!(
                right > 10_000,
                "{archive} (honour={honour}): the right flank is gone — {right} opaque pixels",
            );
            assert_eq!(
                (left, right),
                match archive {
                    "shogun.cg1" => (22_220, 22_794),
                    _ => (18_400, 18_400),
                },
                "{archive} (honour={honour}): the flanks, measured",
            );

            // …and the alternative, so the number above is a finding and not a
            // hope: advertise DECSystem-20 and the right flank is simply gone.
            let Some(lost) =
                boot_named("shogun-r322-s890706.z6", archive, (322, "890706"), honour, Some(1))
            else {
                continue;
            };
            let (dec_left, dec_right) = flank_pixels(&lost);
            assert_eq!(dec_left, left, "{archive}: interpreter 1 keeps the LEFT flank");
            assert_eq!(dec_right, 0, "{archive}: …and loses the right one entirely");
        }
    }
}
