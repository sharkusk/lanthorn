//! SQ-0801 — what the EGA rendition's own transparency does to fmvpoker's cards.
//!
//! Reported as a defect: under `pictures = "FMVPOKER.EG1"` the red outline of every
//! numbered card is missing at the top-left and bottom-right, where the rank plate's
//! background paints over it. Under `fmvpoker.blb` it is intact. `FMVPOKER.EG1` is a
//! byte-identical copy of `zork0.eg1` and `fmvpoker.blb` is Zork Zero's Blorb, so
//! this is one artwork set through two renditions.
//!
//! **It is the artwork, not the compositor.** EGA is the only rendition that uses
//! the flag word's transparency fields at all: `zork0.mg1`, `.cg1` and `.pic` set
//! `EF_TRANS` on every picture with the colour nibble zero, while `zork0.eg1` leaves
//! 132 of its 396 pictures wholly opaque (`flags = 0`) and names colour 1, 2 or 3 on
//! 128 more. The rank plates fmvpoker draws — 133, 134, 135, 138, 140 (36x11) and
//! 144, 145, 146, 149, 151 (34x11) — are among the opaque ones, and the game draws
//! each one at its card's own origin, so an opaque plate necessarily covers the
//! corner the card drew there. Infocom's own YZIP does the same thing with the same
//! bytes (`apple/yzip/pic.asm`: bit 0 clear → `TRANSCLR = $FF`, no colour matches),
//! so the original EGA release looked like this too. There is nothing to fix, and
//! `the_rank_plates_are_opaque_in_the_artwork` is here so a later "fix" that
//! invents transparency for them has to argue with the format first.
//!
//! **And alpha is not being dropped**, which was the other candidate and the one
//! with a blast radius: every native archive marks nearly every picture transparent,
//! so an alpha lost between `Picture::rgba_with` and the canvas would affect Zork
//! Zero, Arthur, Journey and Shogun under any native rendition — invisibly, because
//! a picture transparent on colour 0 can look right with its alpha dropped whenever
//! colour 0 is black against a black ground. EGA's colour 1 is blue and the ground
//! behind the cards is not, so `a_non_zero_transparent_index_reaches_the_canvas`
//! measures the thing that could not have been measured anywhere else.
//!
//! `stories/` is gitignored (CLAUDE.md), so both cases skip vacuously without it.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use crate::fixture_paths::fixture_path;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// fmvpoker one hand in, drawn from `FMVPOKER.EG1` exactly as `startup.rs` wires a
/// named archive up: the native source, its art scale, and the Infocom standard
/// window a native archive has no chunk to declare.
fn fmvpoker_ega_dealt() -> Option<GameSession> {
    let story = fixture_path("fmvpoker.z6");
    let (Ok(bytes), Ok(art)) = (
        std::fs::read(&story),
        std::fs::read(fixture_path("FMVPOKER.EG1")),
    ) else {
        eprintln!("SKIP: gitignored fmvpoker.z6 / FMVPOKER.EG1 missing under {}", stories_dir().display());
        return None;
    };
    let pics = blorb::infocom_pics::InfocomPics::parse(art).expect("FMVPOKER.EG1 is a native archive");
    let mut picts = PictSource::from_native(pics);
    let dims = picts.all_pict_dims();
    // SQ-1021/SQ-1022: the machine's facts in one value. No medium names a machine
    // here, so the profile is `IbmPc` — which is what the `None` interpreter number
    // and `None` colours already meant, and whose `std_window()` is `None`, so
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
    // The archive's own 640x200 picture space; with an art scale of (1, 2) that is
    // the same 640x400 unit screen every rendition lands on (SQ-0838).
    let mut s = GameSession::new_for_machine(bytes, true, false, false, dims, None, None, &boot)
    .expect("a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    match s.pending_input() {
        InputKind::Char => s.submit_char(13),
        _ => s.submit(""),
    };
    s.submit_char(b'p');
    for _ in 0..24 {
        s.submit_char(b' ');
    }
    Some(s)
}

/// Window 0's canvas — where every one of fmvpoker's draws lands.
fn story_canvas(s: &GameSession) -> image::RgbaImage {
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let plate = layout.story_gfx.expect("fmvpoker draws its table into window 0");
    let WinNode::Graphics(g) = &plate.node else { panic!("story_gfx is a Graphics leaf") };
    g.canvas.as_ref().clone()
}

/// The five cards' origins on the unit screen, as the game's own `draw_picture`
/// names them: picture 101 at (42,69), (155,69), (268,69), (381,69), (494,69),
/// 1-based, so one less on the canvas.
const CARD_X: [u32; 5] = [41, 154, 267, 380, 493];
const CARD_Y: u32 = 68;

#[test]
fn a_non_zero_transparent_index_reaches_the_canvas() {
    let Some(s) = fmvpoker_ega_dealt() else { return };
    let pics = blorb::infocom_pics::InfocomPics::parse(
        std::fs::read(fixture_path("FMVPOKER.EG1")).unwrap(),
    )
    .unwrap();

    // Premise: the card body really is transparent on colour ONE — the case no
    // other rendition has. (Its corners are rounded, so the colour is there to see.)
    let card = pics.decode(101).unwrap();
    assert_eq!(
        card.transparent,
        Some(1),
        "premise: zork0.eg1's card body drops colour 1, not colour 0 (flags 0x1001)"
    );
    assert_eq!((card.width, card.height), (96, 64), "premise: 96x64 EGA pixels");
    assert!(
        card.indices[..8].contains(&1),
        "premise: the top-left corner is cut out of the card with colour 1"
    );

    // So if alpha were dropped anywhere between `rgba_with` and the canvas, those
    // corners would arrive painted rather than cut away.
    let canvas = story_canvas(&s);
    for x in CARD_X {
        let p = canvas.get_pixel(x, CARD_Y).0;
        assert_eq!(
            p[3], 0,
            "the card's cut-away corner at ({x},{CARD_Y}) is opaque — the transparent colour the \
             archive named is not reaching the canvas (SQ-0801). Pixel: {p:?}"
        );
    }
}

#[test]
fn the_rank_plates_are_opaque_in_the_artwork() {
    let Some(_s) = fmvpoker_ega_dealt() else { return };
    let pics = blorb::infocom_pics::InfocomPics::parse(
        std::fs::read(fixture_path("FMVPOKER.EG1")).unwrap(),
    )
    .unwrap();

    // The ten plates fmvpoker draws over its cards' corners: every one has
    // `flags = 0`, which the format, Frotz and Infocom's own YZIP all read as "no
    // transparent colour" — so covering the card's outline is what they are for.
    for id in [133u16, 134, 135, 138, 140, 144, 145, 146, 149, 151] {
        let e = pics.entry(id).unwrap_or_else(|| panic!("picture {id} is in FMVPOKER.EG1"));
        assert_eq!(
            e.flags, 0,
            "picture {id} — a card rank plate — is opaque in zork0.eg1. If this ever reads \
             otherwise the artwork changed, not the interpreter (SQ-0801)"
        );
        assert_eq!(
            pics.decode(id).unwrap().transparent,
            None,
            "picture {id} names no transparent colour, so it paints its whole rectangle"
        );
    }

    // And the plate really does cover the card's outline: it is 36 of the card's 96
    // columns wide and its background is the card FACE colour, so what it removes is
    // the border and nothing else. (The MCGA plate at the same place is 18x11 with a
    // transparent surround, which is why the Blorb rendition keeps its corner.)
    let plate = pics.decode(140).unwrap();
    let card = pics.decode(101).unwrap();
    assert_eq!((plate.width, plate.height), (36, 11));
    assert_eq!(
        plate.indices[0], 7,
        "the plate's background is colour 7 — the same light grey as the card face"
    );
    assert_eq!(
        card.indices[card.width as usize + 2],
        4,
        "premise: the card's own outline (colour 4) runs under where the plate is drawn"
    );
}
