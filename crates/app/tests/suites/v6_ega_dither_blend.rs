//! SQ-0797 — a 640-wide rendition's dither blends, because its pixels are half as
//! wide.
//!
//! REPORTED, on `stories/zork0-r393-s890714.z6` with `pictures = "zork0.eg1"`,
//! once SQ-0794 had the EGA colours right: the proscenium arch still reads as
//! salmon-and-olive SPECKLE where the same frame from `zork0.mg1` is bronze.
//!
//! MEASURED CAUSE. EGA has no bronze, so Zork Zero's artist made one. The arch is
//! a column-by-column dither — 22,496 pixels of bright red (index 12) against
//! 18,202 of brown (index 6) on the boot frame — and on a 640×200 EGA screen
//! those columns are half as wide as an MCGA pixel, so the card fused each pair
//! into a colour the palette does not contain. Bocfel says the same of Zork
//! Zero's EGA hint background (`z6/draw_border.cpp:745`): "no single pixel of the
//! artwork is the colour the eye actually sees". lanthorn keeps all 640 columns —
//! `PictSource::art_scale` maps them at (1, 2), onto exactly the rectangle a
//! 320-wide plate covers — so every art pixel survived as a distinct unit pixel
//! and the dither reached the screen at full contrast.
//!
//! WHY NOT LEAVE IT TO THE RENDERER. Because the renderer cannot do it. Every
//! unit-space→pane scale in the v6 path is `FilterType::Nearest` on purpose
//! (`render/graphics.rs`, `render/screen.rs`), and a nearest resample never
//! blends: below 640 px it drops columns, above 640 it replicates them. What the
//! player saw was therefore a function of their terminal width —
//! `the_fusion_is_not_a_function_of_the_pane_width` measures exactly that and is
//! the case that chose the design.
//!
//! NOT CGA. A `.CG1`'s 640-wide art is genuine one-bit line work — Zork Zero's
//! CGA pillar is a lit column of mirrored tiles (SQ-0808) and SQ-0806 hands its
//! two colours to the terminal — and blending one-bit line work only makes it
//! grey. `PictSource::is_monochrome` is the test, read off the archive's own
//! `EF_MONO` flags rather than off a filename.
//!
//! THE SIDE ART (SQ-0815). Reported the moment SQ-0797 landed — *"i noticed the
//! ega dither is still there for the side-art"* — and the measurement says the
//! blend reaches it: across the flank columns either side of the story window,
//! horizontal speckle runs **62.86 raw and 12.74 fused**, and the raster
//! composite and the tiled border extension both carry the fused pixels rather
//! than re-fetching the picture. `the_blend_reaches_the_side_art` and
//! `the_border_extension_tiles_fused_pixels` pin both halves of that, because
//! nothing in SQ-0797 measured the flank at all — its cases all read the whole
//! frame, where the arch's 22k dithered pixels dominate the average.
//!
//! What the flank keeps is not a missed blend but a different DITHER. The tent
//! is a notch at one frequency: it zeroes a period-2 alternation exactly, which
//! is why the boot frame's interior fuses to a horizontal speckle of **0.00**.
//! Zork Zero's pillar shaft is error-diffusion dithered instead — seven EGA
//! entries in irregular runs — and a broadband dither has energy at every
//! frequency, most of which a three-tap kernel does not touch. Widening it does
//! fuse the shaft (`[1, 2, 2, 2, 1] / 8` takes the flank to 6.98 against the MCGA
//! flank's 6.05, and the whole frame's distance to MCGA from 27.79 to 26.04) and
//! it also mushes the compass rose's N/W/E/S lettering on the same plate, which
//! is 640-wide line art the card genuinely resolved. So the kernel stays.
//!
//! Every fixture here is gitignored, so each case **skips vacuously** when absent.

use std::collections::BTreeSet;
use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};
use blorb::infocom_pics::InfocomPics;

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

/// Boot Zork Zero against `archive` and play far enough for the border to be up —
/// the same wiring `startup.rs` uses and `v6_hardware_palette` measures colour
/// with: the archive supplies the standard window and the per-axis art scale, and
/// nothing else differs between renditions.
fn boot(archive: &str, honor_game_colours: bool) -> Option<GameSession> {
    let story = read("zork0-r393-s890714.z6")?;
    let mut picts =
        PictSource::from_native(InfocomPics::parse(read(archive)?).expect("a native archive"));
    let picture_dims = picts.all_pict_dims();
    // The chain `startup.rs` runs: a Blorb's `Reso`, else the archive's own
    // picture space (SQ-0838 — 320x200 for MCGA/Amiga, 640x200 for EGA/CGA, and
    // 480x300 for the standard Macintosh's mono plate). The screen is that space
    // times the density below, which is 640x400 for every rendition here.
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
    // SQ-0811: no pinned random seed — this suite measures colour, not chance.
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

/// Zork Zero's full-screen border canvas (window 7), in unit space.
fn frame(session: &GameSession) -> image::RgbaImage {
    (*session.pictures_canvas.get(&7).expect("Zork Zero's border is window 7").img).clone()
}

/// How SPECKLED a frame is: mean per-channel |x − x_right| over horizontally
/// adjacent opaque pairs. A column dither at full contrast scores high; a fused
/// bronze scores low. This is the number the report is about — "salmon-and-olive
/// speckle" is a statement about neighbouring columns, not about any one pixel.
fn speckle(img: &image::RgbaImage) -> f64 {
    let (w, h) = img.dimensions();
    let (mut sum, mut n) = (0u64, 0u64);
    for y in 0..h {
        for x in 0..w.saturating_sub(1) {
            let (a, b) = (*img.get_pixel(x, y), *img.get_pixel(x + 1, y));
            if a.0[3] != 255 || b.0[3] != 255 {
                continue;
            }
            for k in 0..3 {
                sum += u64::from(a.0[k].abs_diff(b.0[k]));
                n += 1;
            }
        }
    }
    if n == 0 { 0.0 } else { sum as f64 / n as f64 }
}

/// Mean per-channel distance between two frames, over pixels opaque in both —
/// the measurement SQ-0797 used to decide the two renditions had converged.
fn distance(a: &image::RgbaImage, b: &image::RgbaImage) -> f64 {
    assert_eq!(a.dimensions(), b.dimensions(), "both renditions land in one unit space");
    let (mut sum, mut n) = (0u64, 0u64);
    for (pa, pb) in a.pixels().zip(b.pixels()) {
        if pa.0[3] == 0 || pb.0[3] == 0 {
            continue;
        }
        for k in 0..3 {
            sum += u64::from(pa.0[k].abs_diff(pb.0[k]));
            n += 1;
        }
    }
    sum as f64 / n as f64
}

/// The SIDE ART's own columns: everything outside the story window's declared
/// native rect, left flank and right. Read from the screen model exactly as
/// `flank_native_box` and `extend_raster_flanks` read it, rather than pinned to
/// Zork Zero's `(86, 78, 468, 320)` — a suite that hard-codes the rect stops
/// measuring the flank the moment the game moves its window.
fn flank_columns(session: &GameSession) -> (u32, u32, u32, u32) {
    let model = session.screen();
    let app::engine::WinNode::Layered(items) = &model.root else { panic!("v6 root is Layered") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let story = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT).story.expect("a story window");
    let left = u32::from(story.x_px).min(u32::from(native.0));
    let right = (u32::from(story.x_px) + u32::from(story.w_px)).min(u32::from(native.0));
    assert!(left > 0 && right < u32::from(native.0), "premise: the frame HAS side art");
    (left, right, u32::from(native.0), u32::from(native.1))
}

/// Columns `[x0, x1)` of `img`, full height.
fn columns(img: &image::RgbaImage, x0: u32, x1: u32) -> image::RgbaImage {
    image::RgbaImage::from_fn(x1 - x0, img.height(), |x, y| *img.get_pixel(x0 + x, y))
}

/// The renderer's own unit-space→pane scale: nearest-neighbour, at every call
/// site in the v6 path (`render/graphics.rs:741`, `render/screen.rs:8548`).
fn to_pane(img: &image::RgbaImage, pane_w: u32) -> image::RgbaImage {
    let h = img.height();
    image::imageops::resize(img, pane_w, h, image::imageops::FilterType::Nearest)
}

/// The reported defect. Zork Zero's EGA arch fuses into bronze instead of
/// arriving as speckle, and in doing so lands next to the MCGA rendition of the
/// same frame rather than 60% further away.
///
/// Falsified by reverting `blend_columns` to `false` in `PictSource::from_native`:
///
/// ```text
/// honor=true: the EGA frame is still a column dither at full contrast —
/// horizontal speckle 49.118, and MCGA's own frame scores 4.331
/// ```
///
/// …which is the salmon-and-olive speckle as reported, at 11.3× the MCGA frame's
/// own neighbour-to-neighbour variation.
#[test]
fn the_ega_column_dither_fuses_instead_of_reaching_the_screen_as_speckle() {
    for honor_game_colours in [true, false] {
        let Some(ega) = boot("zork0.eg1", honor_game_colours) else { return };
        let Some(mcga) = boot("zork0.mg1", honor_game_colours) else { return };
        let (e, m) = (frame(&ega), frame(&mcga));

        // MEASURED: 8.401 fused, against 49.118 raw and 4.331 for MCGA.
        let (se, sm) = (speckle(&e), speckle(&m));
        assert!(
            se < 12.0,
            "honor={honor_game_colours}: the EGA frame is still a column dither at full \
             contrast — horizontal speckle {se:.3}, and MCGA's own frame scores {sm:.3}"
        );
        assert!(
            se < sm * 2.5,
            "honor={honor_game_colours}: EGA speckle {se:.3} must land near the MCGA \
             rendition's {sm:.3}, not a multiple of it"
        );

        // MEASURED: 27.79 fused, 44.51 raw. The residue is the two renditions
        // being separately drawn artwork, which no filter closes.
        let d = distance(&e, &m);
        assert!(
            d < 32.0,
            "honor={honor_game_colours}: mean per-channel distance to the MCGA frame is \
             {d:.4}; unfused it is 44.51"
        );

        // And the fused colour is one the card's palette does not hold, which is
        // the entire point of a dither: the arch's shadow is brown (170,85,0)
        // fused with black, i.e. (85,43,0).
        let seen: BTreeSet<[u8; 3]> =
            e.pixels().filter(|p| p.0[3] == 255).map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
        assert!(
            seen.contains(&[85, 43, 0]),
            "honor={honor_game_colours}: the fused shadow bronze is missing from the frame"
        );
        assert!(
            !blorb::infocom_pics::EGA_PALETTE.contains(&[85, 43, 0]),
            "premise: bronze is not an EGA colour — the artist had to dither for it"
        );
    }
}

/// The design decision, measured. Fusing at the archive boundary makes the answer
/// a property of the ARTWORK; leaving it to the renderer's own scale would make it
/// a property of the player's terminal, because that scale is nearest-neighbour
/// at every call site and never blends at any width.
///
/// Falsified by reverting `blend_columns` to `false`: the EGA frame's speckle runs
/// 22.272 / 40.302 / 49.118 / 39.219 / 24.441 at pane widths 320 / 480 / 640 / 800
/// / 1280 — a 2.2× swing with terminal size, and never once near MCGA's 8.746 /
/// 5.793 / 4.331 / 3.458 / 2.155. The first assertion below fails at every one of
/// the five widths.
#[test]
fn the_fusion_is_not_a_function_of_the_pane_width() {
    for honor_game_colours in [true, false] {
        let Some(ega) = boot("zork0.eg1", honor_game_colours) else { return };
        let Some(mcga) = boot("zork0.mg1", honor_game_colours) else { return };
        let (e, m) = (frame(&ega), frame(&mcga));

        // Panes narrower than the unit screen, equal to it, and wider — the case
        // the renderer's Nearest scale cannot help with at all.
        for pane_w in [320u32, 480, 640, 800, 1280] {
            let (pe, pm) = (to_pane(&e, pane_w), to_pane(&m, pane_w));
            let (se, sm) = (speckle(&pe), speckle(&pm));
            assert!(
                se < sm * 2.5,
                "honor={honor_game_colours}, pane {pane_w}: EGA speckle {se:.3} against MCGA's \
                 {sm:.3} — what the arch reads as still depends on the terminal's width"
            );
            let d = distance(&pe, &pm);
            assert!(
                d < 32.0,
                "honor={honor_game_colours}, pane {pane_w}: distance to MCGA {d:.4}, unfused \
                 44.3–45.1 at these widths"
            );
        }
    }
}

/// CGA is UNTOUCHED, and this is the constraint that shapes the whole fix. A
/// `.CG1` is 640-wide exactly as an `.EG1` is, so a rule keyed on width alone
/// would blur it; the key is `is_monochrome`, off the archive's own `EF_MONO`
/// flags. Blending one-bit line work produces greys, and greys are precisely what
/// SQ-0806 stopped the CGA stencil from having.
///
/// Falsified by dropping `&& !src.is_monochrome()` from the gate: the frame comes
/// back carrying the intermediate values a tent makes of a one-bit edge — a
/// quarter, a half and three quarters of the card's lit state — and the
/// two-colour assertion below fails on them.
#[test]
fn cga_line_art_is_never_blended() {
    for honor_game_colours in [true, false] {
        let Some(cga) = boot("zork0.cg1", honor_game_colours) else { return };
        let c = frame(&cga);
        let seen: BTreeSet<[u8; 3]> =
            c.pixels().filter(|p| p.0[3] != 0).map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
        assert_eq!(
            seen,
            BTreeSet::from([[0, 0, 0], [170, 170, 170]]),
            "honor={honor_game_colours}: a CGA frame stays the card's two states — no third"
        );
        // Its one-bit edges keep their full contrast: measured 38.275, and a tent
        // across them would take it to 10.09. Both numbers are two thirds of what
        // they were before SQ-0956 moved the card's lit state from 255 to 170 —
        // the same edges, a shorter ladder, so the ratio between sharp and tented
        // is what carries the claim and it is unchanged.
        let s = speckle(&c);
        assert!(
            s > 25.0,
            "honor={honor_game_colours}: CGA line work has been softened (speckle {s:.3}, \
             sharp is 38.3 and tented is 10.1)"
        );
    }

    // …and the gate says so directly, off the content rather than the extension.
    for (archive, blends) in [("zork0.cg1", false), ("zork0.eg1", true)] {
        let Some(raw) = read(archive) else { return };
        let src = PictSource::from_native(InfocomPics::parse(raw).expect("parses"));
        assert_eq!(src.art_scale().map(|s| s.0), Some(1), "{archive}: both are 640-wide");
        assert_eq!(src.is_monochrome(), !blends, "{archive}: only the two-colour one opts out");
    }
}

/// The 320-wide renditions are UNTOUCHED, and cannot be otherwise: their art
/// scale is (2, 2), there is no dither at this frequency, and nothing fuses.
/// Measured through the palette rather than the pixels — an MCGA archive stores
/// 4 bits per channel, so every channel on its frame is a multiple of 17, and a
/// blend of two such colours is not.
#[test]
fn the_320_wide_renditions_are_untouched() {
    for archive in ["zork0.mg1", "zork0.pic"] {
        let Some(raw) = read(archive) else { return };
        let src = PictSource::from_native(InfocomPics::parse(raw).expect("parses"));
        assert_eq!(src.art_scale().map(|s| s.0), Some(2), "{archive}: 320-wide picture space");
    }

    for honor_game_colours in [true, false] {
        let Some(mcga) = boot("zork0.mg1", honor_game_colours) else { return };
        let m = frame(&mcga);
        assert!(
            m.pixels().filter(|p| p.0[3] != 0).all(|p| p.0[..3].iter().all(|ch| ch % 17 == 0)),
            "honor={honor_game_colours}: an MCGA frame is 4 bits per channel; a blend is not"
        );
    }
}

/// THE REPORTED DEFECT (SQ-0815): the blend reaches the SIDE ART.
///
/// The flank is not covered by any SQ-0797 case — those all measure the whole
/// frame, where the arch's 22,496 dithered pixels dominate — so "the boot frame
/// fused" was never evidence about the pillars. Measured over the flank columns
/// alone, in unit space:
///
/// | | left flank | right flank |
/// | --- | --- | --- |
/// | raw | 62.86 | 62.17 |
/// | fused | 12.74 | 12.45 |
///
/// The MCGA rendition scores 6.05 and 5.68 over the same columns, but it is
/// 320-wide art doubled onto the unit screen (`art_scale` `(2, 2)`), so every
/// second adjacent pair is two copies of one pixel and its score is halved by
/// construction. Folded back into the 320-wide space both artists drew in — EGA
/// averaged in pairs, MCGA de-doubled — the honest comparison is **17.48 EGA
/// against 12.32 MCGA**, from 25.46 raw. That is the assertion below.
///
/// Falsified by reverting `blend_columns` to `false` in `PictSource::from_native`:
///
/// ```text
/// honor=true: the side art is still a dither at full contrast — left flank
/// horizontal speckle 62.858, right 62.174, and the MCGA flanks score 6.045
/// and 5.680
/// ```
#[test]
fn the_blend_reaches_the_side_art() {
    for honor_game_colours in [true, false] {
        let Some(ega) = boot("zork0.eg1", honor_game_colours) else { return };
        let Some(mcga) = boot("zork0.mg1", honor_game_colours) else { return };
        let (e, m) = (frame(&ega), frame(&mcga));
        let (left, right, w, _) = flank_columns(&ega);

        for (side, x0, x1) in [("left", 0, left), ("right", right, w)] {
            let (fe, fm) = (columns(&e, x0, x1), columns(&m, x0, x1));
            // MEASURED: 12.74 / 12.45 fused, against 62.86 / 62.17 raw.
            let (se, sm) = (speckle(&fe), speckle(&fm));
            assert!(
                se < 20.0,
                "honor={honor_game_colours}: the side art is still a dither at full contrast \
                 — {side} flank horizontal speckle {se:.3}, and the MCGA flank scores {sm:.3}"
            );

            // The like-for-like comparison, in the 320-wide space both artists
            // drew in: EGA averaged in column pairs, MCGA de-doubled. MEASURED:
            // 17.48 against MCGA's 12.32, from 25.46 raw.
            let (fe, fm) = (fold_pairs(&fe), drop_doubles(&fm));
            let (se, sm) = (speckle(&fe), speckle(&fm));
            assert!(
                se < sm * 1.7,
                "honor={honor_game_colours}: in the 320-wide picture space the {side} flank \
                 still carries {se:.3} against the MCGA flank's {sm:.3} — unfused it is 25.46"
            );
        }
    }
}

/// EGA folded into the 320-wide space its half-width columns actually filled:
/// each pair averaged. (Not a filter — a measurement. See
/// [`the_blend_reaches_the_side_art`].)
fn fold_pairs(img: &image::RgbaImage) -> image::RgbaImage {
    image::RgbaImage::from_fn(img.width() / 2, img.height(), |x, y| {
        let (a, b) = (*img.get_pixel(2 * x, y), *img.get_pixel(2 * x + 1, y));
        let mid = |k: usize| ((u16::from(a.0[k]) + u16::from(b.0[k])) / 2) as u8;
        image::Rgba([mid(0), mid(1), mid(2), a.0[3].min(b.0[3])])
    })
}

/// MCGA back out of the unit screen: `art_scale` `(2, 2)` wrote every picture
/// column twice, so every other one is the artist's.
fn drop_doubles(img: &image::RgbaImage) -> image::RgbaImage {
    image::RgbaImage::from_fn(img.width() / 2, img.height(), |x, y| *img.get_pixel(2 * x, y))
}

/// The other half of SQ-0815's question: the flank's pixels come from the CHROME
/// CANVAS, and the tiled border extension copies them rather than re-fetching the
/// picture — so a pane taller than the art gets fused pixels all the way down,
/// and SQ-0808's mirrored alternate tiles cannot re-separate a fused column pair
/// (a vertical mirror does not touch the horizontal axis the tent worked on).
///
/// This is the case that would have caught a genuine bypass: it asks
/// `v6_border::flank_source` for 200 native rows MORE than Zork Zero's pillars
/// occupy, which is the `zork_zero` → `extend_pillars` → `tile_down(flip)` path
/// in full, and measures the manufactured rows against the art's own.
#[test]
fn the_border_extension_tiles_fused_pixels() {
    for honor_game_colours in [true, false] {
        let Some(ega) = boot("zork0.eg1", honor_game_colours) else { return };
        let e = frame(&ega);
        let (left, _, _, native_h) = flank_columns(&ega);
        let art = app::render::v6_border::art_extent(&e, 0, left);
        assert_eq!(art.1, native_h, "premise: Zork Zero's pillars reach the last native row");

        // Ask for a band half again as tall as the art, so every row below
        // `native_h` is manufactured by the tiler.
        let rows = native_h + native_h / 2;
        let src = app::render::v6_border::flank_source(&e, &e, 0, left, art, native_h, 0, rows)
            .expect("Zork Zero's pillars are a recognised border art");
        assert_eq!(src.height(), rows, "the extension paints the whole band");

        let tiled = columns(&crop_rows(&src, native_h, rows), 0, left);
        let s = speckle(&tiled);
        assert!(
            s < 20.0,
            "honor={honor_game_colours}: the tiled extension is speckled ({s:.3}) where the \
             art it repeats is fused — the tiling path is re-fetching unblended pixels"
        );
        // …and it is the SAME art, not merely something smooth: every colour it
        // paints is one the fused flank already holds.
        let art_colours: BTreeSet<[u8; 3]> = columns(&e, 0, left)
            .pixels()
            .filter(|p| p.0[3] == 255)
            .map(|p| [p.0[0], p.0[1], p.0[2]])
            .collect();
        assert!(
            tiled
                .pixels()
                .filter(|p| p.0[3] == 255)
                .all(|p| art_colours.contains(&[p.0[0], p.0[1], p.0[2]])),
            "honor={honor_game_colours}: the extension invented a colour the fused flank \
             does not hold — it did not come from this canvas"
        );
    }
}

/// Rows `[y0, y1)` of `img`.
fn crop_rows(img: &image::RgbaImage, y0: u32, y1: u32) -> image::RgbaImage {
    image::RgbaImage::from_fn(img.width(), y1 - y0, |x, y| *img.get_pixel(x, y0 + y))
}

/// SQ-0816 — keeping the dither is a CHOICE, and the default is to fuse it.
///
/// `fuse_art_dither` defaults to true because that is what the hardware did to
/// the eye and what SQ-0797 measured as correct; false hands back the archive's
/// own pixels, every column distinct. The switch may only ever turn the filter
/// OFF: eligibility stays the archive's business, so no setting can make a `.CG1`
/// blend (the case below), and none can make a 320-wide plate blend either.
#[test]
fn keeping_the_dither_is_a_choice_and_fusing_is_the_default() {
    assert!(
        app::config::Config::default().fuse_art_dither,
        "the shipped default fuses — SQ-0797 measured that as what the card did"
    );

    let Some(raw) = read("zork0.eg1") else { return };
    let mut src = PictSource::from_native(InfocomPics::parse(raw).expect("parses"));
    assert!(src.fuses_dither(), "a 640-wide sixteen-colour archive fuses out of the box");

    // The archive's own pixels: only the EGA sixteen, nothing between them.
    src.set_fuse_dither(false);
    assert!(!src.fuses_dither());
    let unfused = src.image(1).expect("Zork Zero's EGA border").to_rgba8();
    let seen: BTreeSet<[u8; 3]> =
        unfused.pixels().filter(|p| p.0[3] == 255).map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
    assert!(
        seen.iter().all(|c| blorb::infocom_pics::EGA_PALETTE.contains(c)),
        "unfused art is the card's sixteen colours and nothing else, got {} distinct",
        seen.len()
    );

    // …and turning it back on re-decodes rather than serving the cached raw
    // image, which is the whole reason `set_fuse_dither` drops the caches.
    src.set_fuse_dither(true);
    let fused = src.image(1).expect("the same picture again").to_rgba8();
    assert_eq!(fused.dimensions(), unfused.dimensions(), "fusing never resizes");
    assert!(
        fused.pixels().zip(unfused.pixels()).any(|(a, b)| a != b),
        "the setting turned back on and the cache handed back the unfused image"
    );
    let fused_seen: BTreeSet<[u8; 3]> =
        fused.pixels().filter(|p| p.0[3] == 255).map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
    assert!(
        fused_seen.iter().any(|c| !blorb::infocom_pics::EGA_PALETTE.contains(c)),
        "fusing is back on and yet every colour is still one of the card's sixteen — the \
         whole point of a dither is the colour BETWEEN two of them"
    );
}

/// CGA is untouched HOWEVER the setting is set. `fuse_art_dither` is a
/// preference about a filter, not an override of what the filter may run on:
/// blending one-bit line work makes grey, and grey is what SQ-0806 and SQ-0808
/// spent their time removing.
#[test]
fn no_setting_can_make_cga_blend() {
    let Some(raw) = read("zork0.cg1") else { return };
    let mut src = PictSource::from_native(InfocomPics::parse(raw).expect("parses"));
    for fuse in [true, false, true] {
        src.set_fuse_dither(fuse);
        assert!(!src.fuses_dither(), "fuse_art_dither={fuse}: a .CG1 never fuses");
    }
    let img = src.image(1).expect("Zork Zero's CGA border").to_rgba8();
    let seen: BTreeSet<[u8; 3]> =
        img.pixels().filter(|p| p.0[3] == 255).map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
    assert_eq!(
        seen,
        BTreeSet::from([[0, 0, 0], [170, 170, 170]]),
        "a CGA picture stays the card's two states whatever the setting says"
    );
}

/// Alpha is never blended, so a stencil keeps its edges. Every native archive
/// marks nearly every picture transparent, and a filter that averaged alpha would
/// ring every cut-out with half-transparent fringe — invisible on a black ground
/// and wrong everywhere else. `zork0.eg1`'s card body (picture 101) is cut out on
/// colour ONE, which is blue, so it is the case where a fringe would show.
///
/// Falsified by blending alpha along with the colour channels: the card's first
/// cut-out pixel arrives at alpha 64 instead of 0.
#[test]
fn a_cut_out_keeps_its_exact_alpha() {
    let Some(raw) = read("zork0.eg1") else { return };
    let pics = InfocomPics::parse(raw).expect("parses");
    let card = pics.decode(101).expect("Zork Zero's EGA card body");
    assert_eq!(card.transparent, Some(1), "premise: cut out on colour 1, not 0 (SQ-0801)");

    let mut src = PictSource::from_native(InfocomPics::parse(read("zork0.eg1").unwrap()).unwrap());
    let img = src.image(101).expect("the same picture, through the fusing source");
    let img = img.to_rgba8();
    assert_eq!(
        (img.width(), img.height()),
        (u32::from(card.width), u32::from(card.height)),
        "fusing never resizes: 640 columns in, 640 columns out"
    );
    for (i, p) in img.pixels().enumerate() {
        let want = if card.indices[i] == 1 { 0 } else { 255 };
        assert_eq!(
            p.0[3], want,
            "pixel {i} (index {}) came through at alpha {}, not {want}",
            card.indices[i], p.0[3]
        );
    }
}
