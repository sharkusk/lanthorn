//! SQ-1424 — a Glk 0.7.6 `imagerule_WidthRatio` inline picture is STANDING:
//! it re-resolves against the transcript band's CURRENT width, so a terminal
//! resize resizes the picture with the text.
//!
//! This is the half of `glk_image_draw_scaled_ext` that made SQ-1416 decline
//! the whole call and report Glk 0.7.5 instead. The spec
//! (`Glk-Spec-076.html`, §"Graphics in Text Buffer Windows") is explicit that a
//! text buffer differs from a graphics window here:
//!
//! > In a text buffer window, imagerule_WidthRatio is dynamically computed; the
//! > image width will always be relative to the *current* window width. If the
//! > text buffer window is resized (by the user or a window arrangement call),
//! > the image will resize too.
//!
//! …against §"Graphics in Graphics Windows", where the same rule is one-shot:
//!
//! > The imagerule_WidthRatio option does *not* dynamically resize in a
//! > graphics window. The image size is computed when
//! > glk_image_draw_scaled_ext() is called, and then the image is painted to
//! > the canvas. The maxwidth argument is ignored in graphics windows.
//!
//! So the whole chain is driven here, not just the arithmetic: a real `AppGlk`
//! with a real Blorb `Pict`, the `GlkBackend` seam gvm dispatches `0x00EC`
//! through, the transcript element the app actually anchors, and finally
//! `InlineImage::fitted_cells` — the one function the transcript wrapper calls
//! with the live band width on every layout, and therefore the place a resize
//! re-enters.
//!
//! **Perturb before asserting.** Measuring the picture at the width it was
//! drawn at proves nothing — a frozen size looks right there, which is exactly
//! how a one-shot implementation passes a naive test. Every case below asks for
//! the size at a DIFFERENT width than the draw happened at.
//!
//! FALSIFY: make `AppGlk::buffer_draw_image_ext` resolve the rule itself and
//! store the result in `scaled` (with `rule: None`) — i.e. treat a buffer
//! window like a graphics window. `ratio_image_follows_the_band_width` then
//! fails with the picture stuck at its original width.

use app::inline_image::{ImageAlign, InlineImage};
use app::session::TranscriptElem;
use gvm::glk::{imagerule, GlkBackend, ImageRule, WinType};

/// A Blorb carrying one `Pict` resource, built by hand: the app's own
/// `test_blorb_with_pict` is `pub(crate)` and this suite is out of crate.
fn blorb_with_pict(resnum: u32, png: &[u8]) -> blorb::Blorb {
    fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(ty);
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }
    let ridx_data_len = 4 + 12; // resource count + one 12-byte entry
    let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
    let mut ridx = Vec::new();
    ridx.extend_from_slice(&1u32.to_be_bytes());
    ridx.extend_from_slice(b"Pict");
    ridx.extend_from_slice(&resnum.to_be_bytes());
    ridx.extend_from_slice(&(first_res_off as u32).to_be_bytes());
    let mut inner = Vec::new();
    inner.extend_from_slice(b"IFRS");
    inner.extend_from_slice(&chunk(b"RIdx", &ridx));
    inner.extend_from_slice(&chunk(b"PNG ", png));
    let mut form = Vec::new();
    form.extend_from_slice(b"FORM");
    form.extend_from_slice(&(inner.len() as u32).to_be_bytes());
    form.extend_from_slice(&inner);
    blorb::Blorb::parse(form).expect("a hand-built Blorb with one Pict parses")
}

/// A `w × h` opaque PNG — the picture whose proportions the rules act on.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30]));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .expect("PNG encodes");
    bytes
}

/// Drive one `glk_image_draw_scaled_ext` into a text-buffer window through the
/// real backend and hand back the transcript picture it anchored.
///
/// `draw_band_px` is the buffer window's width in pixels AT THE MOMENT OF THE
/// DRAW — the number gvm passes down. A correct implementation must not let it
/// influence anything the caller measures later, which is what the cases check.
fn drawn(pic: (u32, u32), rule: ImageRule, draw_band_px: u32) -> InlineImage {
    let blorb = blorb_with_pict(1, &png(pic.0, pic.1));
    let mut glk = app::glk_backend::AppGlk::with_graphics(
        80,
        24,
        (1, 1),
        app::graphics::PictSource::new(Some(blorb)),
    );
    glk.window_open(1, WinType::TextBuffer);
    assert!(
        glk.buffer_draw_image_ext(1, /*resnum*/ 1, /*imagealign_InlineUp*/ 1, rule, draw_band_px),
        "the picture resolves and is anchored"
    );
    let elems = glk.take_transcript_elems();
    let mut pics = elems.into_iter().filter_map(|e| match e {
        TranscriptElem::Image(img) => Some(img),
        _ => None,
    });
    let img = pics.next().expect("exactly one inline picture was anchored");
    assert!(pics.next().is_none(), "and only one");
    img
}

/// The rule reaches the transcript UNRESOLVED. This is the structural claim the
/// rest of the file rests on: a picture that arrived carrying a resolved size
/// could not follow a later resize no matter how the renderer behaved.
#[test]
fn a_buffer_draw_stores_the_rule_rather_than_a_size() {
    let rule = ImageRule {
        rule: imagerule::WIDTH_RATIO | imagerule::ASPECT_RATIO,
        width: 0x8000,
        height: 0x1_0000,
        maxwidth: 0,
    };
    let img = drawn((200, 100), rule, /*drawn at*/ 320);
    assert_eq!(img.rule, Some(rule), "the standing rule is kept verbatim");
    assert_eq!(img.scaled, None, "and NOT collapsed into a frozen pixel size");
    assert_eq!(img.align, ImageAlign::InlineUp, "imagealign_InlineUp (1) decoded");
    assert_eq!((img.pixels.width(), img.pixels.height()), (200, 100), "natural pixels intact");
}

/// The headline behaviour: `imagerule_WidthRatio` at 50% occupies half the band,
/// and keeps occupying half of it when the band changes size. The picture is
/// DRAWN at one width and MEASURED at three others — none of them the draw
/// width — so a frozen size cannot pass by coincidence.
#[test]
fn ratio_image_follows_the_band_width() {
    // 200×100 (2:1) drawn at a 320px band, asking for half the window width and
    // its own aspect ratio.
    let img = drawn(
        (200, 100),
        ImageRule {
            rule: imagerule::WIDTH_RATIO | imagerule::ASPECT_RATIO,
            width: 0x8000,
            height: 0x1_0000,
            maxwidth: 0,
        },
        320,
    );
    // 1px cells keep the cell count equal to the pixel count, so these numbers
    // are the resolved picture size directly.
    let cell = (1u16, 1u16);
    // A 40-column band → 20 columns of picture, 10 rows (2:1 kept).
    assert_eq!(img.fitted_cells(40, cell), (20, 10), "half of 40");
    // THE RESIZE: the pane doubles. A one-shot implementation would still say 20.
    assert_eq!(img.fitted_cells(80, cell), (40, 20), "half of 80 — it followed the resize");
    // And shrinks again just as readily.
    assert_eq!(img.fitted_cells(20, cell), (10, 5), "half of 20");
    // The ratio itself is what is constant, at every width.
    for w in [16u16, 30, 64, 100, 250] {
        let (cols, _) = img.fitted_cells(w, cell);
        assert_eq!(cols, w / 2, "always half the band, at width {w}");
    }
}

/// A picture with no rule is the pre-0.7.6 behaviour and must not have moved:
/// `glk_image_draw` / `glk_image_draw_scaled` still freeze their size, and are
/// only ever reduced by the band's own width cap.
#[test]
fn a_ruleless_picture_keeps_its_fixed_size_across_a_resize() {
    let mut img = drawn(
        (200, 100),
        ImageRule { rule: imagerule::WIDTH_ORIG | imagerule::HEIGHT_ORIG, width: 0, height: 0, maxwidth: 0 },
        320,
    );
    img.rule = None;
    img.scaled = Some((60, 30)); // as glk_image_draw_scaled leaves it
    let cell = (1u16, 1u16);
    assert_eq!(img.fitted_cells(100, cell), (60, 30), "fixed at 60×30");
    assert_eq!(img.fitted_cells(200, cell), (60, 30), "…and a wider pane does not stretch it");
    // Only the band cap bites, and it preserves the aspect (the long-standing rule).
    assert_eq!(img.fitted_cells(30, cell), (30, 15), "narrower than the picture → reduced proportionally");
}

/// `maxwidth` is the buffer window's own bound and applies here (it is ignored
/// only in graphics windows). It is a fraction of the CURRENT band too, so it
/// moves with the resize rather than pinning a pixel count.
#[test]
fn maxwidth_bounds_against_the_current_band_not_the_drawing_one() {
    // A fixed 60px-wide picture, bounded to a quarter of the window.
    let img = drawn(
        (200, 100),
        ImageRule {
            rule: imagerule::WIDTH_FIXED | imagerule::HEIGHT_FIXED,
            width: 60,
            height: 30,
            maxwidth: 0x4000, // 25%
        },
        320,
    );
    let cell = (1u16, 1u16);
    // 25% of 400 = 100 > 60 → the picture keeps its requested size.
    assert_eq!(img.fitted_cells(400, cell), (60, 30), "the bound does not bite in a wide pane");
    // 25% of 160 = 40 < 60 → reduced to 40, and the height with it (30·40/60).
    assert_eq!(img.fitted_cells(160, cell), (40, 20), "…and does bite in a narrow one");
    // 25% of 80 = 20 → 20×10.
    assert_eq!(img.fitted_cells(80, cell), (20, 10), "proportional all the way down");
}

/// The picture is measured in CELLS, so the cell size participates: the same
/// standing rule against the same column count answers differently on a
/// different font. Half of a 40-column band is 20 columns whatever the cell is,
/// but the ROW count follows the cell's aspect — which is why the rule is
/// resolved in pixels and only then divided into cells.
#[test]
fn the_rule_resolves_in_pixels_and_the_cell_size_still_applies() {
    let img = drawn(
        (200, 100),
        ImageRule {
            rule: imagerule::WIDTH_RATIO | imagerule::ASPECT_RATIO,
            width: 0x8000,
            height: 0x1_0000,
            maxwidth: 0,
        },
        320,
    );
    // 40 cols at 8×16 → a 320px band; half is 160px wide, 80px tall (2:1).
    // 160/8 = 20 columns, 80/16 = 5 rows.
    assert_eq!(img.fitted_cells(40, (8, 16)), (20, 5));
    // The same band in square 8×8 cells is the same 160×80 picture: 20 × 10.
    assert_eq!(img.fitted_cells(40, (8, 8)), (20, 10));
    // And a font-size change is a relayout like any other — the ratio holds.
    assert_eq!(img.fitted_cells(80, (8, 16)), (40, 10), "wider pane, same half");
}

// ── Real-game smoke: Andrew Plotkin's own `imagetest` exerciser ──────────────

/// `unit_tests/imagetest.gblorb` is the glk-dev exerciser written FOR this
/// feature: its own help text offers `wfix`, `worig`, `wratio`, `hfix`,
/// `horig`, `hratio`, `maxwidth` and `halfwidth`, and a `scales` command that
/// sweeps them. It is gitignored (see `unit_tests/README.md` for the download),
/// so this skips vacuously on CI exactly as the `stories/` suites do — and, as
/// always, a vacuous skip reads like a pass, which is why the synthetic cases
/// above carry the actual proof.
#[test]
fn imagetest_drives_the_new_call_and_anchors_standing_rules() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../unit_tests/imagetest.gblorb");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored fixture missing at {}", path.display());
        return;
    };
    let blorb = blorb::Blorb::parse(bytes.clone()).expect("imagetest.gblorb is a Blorb");
    let (kind, exec) = blorb.executable().expect("…carrying an executable");
    assert_eq!(kind, blorb::ExecKind::Glulx, "imagetest is a Glulx story");
    let story = exec.to_vec();
    let blorb = blorb::Blorb::parse(bytes).expect("re-parsed for the resource source");
    let mut sess = app::glulx_session::GlulxSession::new(
        story,
        80,
        30,
        /*honor_game_colours*/ true,
        /*graphics*/ true,
        /*sound*/ false,
        (8, 16),
        Some(blorb),
        &[],
    )
    .expect("imagetest boots");
    use app::engine::Engine;
    let _ = Engine::take_transcript(&mut sess);

    // `scales` is the command that sweeps the imagerule options.
    let turn = Engine::submit(&mut sess, "scales");
    let banner = Engine::take_transcript(&mut sess);
    let text: String = turn
        .transcript_elems
        .iter()
        .filter_map(|e| match e {
            app::session::TranscriptElem::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    // With graphics ON the exerciser must not fall back to its no-graphics
    // apology — that line is what gvm-cli (which supports no graphics at all,
    // truthfully) still prints, and it is the tell that this smoke measured
    // nothing.
    assert!(
        !text.contains("does not support graphics") && !banner.contains("does not support graphics"),
        "graphics are enabled here, so imagetest must not take its no-graphics path; got:\n{text}"
    );

    let pics: Vec<_> = turn
        .transcript_elems
        .iter()
        .filter_map(|e| match e {
            app::session::TranscriptElem::Image(img) => Some(img),
            _ => None,
        })
        .collect();
    assert!(!pics.is_empty(), "`scales` anchors pictures; transcript was:\n{text}");

    // At least one of them must carry a STANDING rule — i.e. the game reached
    // `glk_image_draw_scaled_ext` rather than only the two older calls. That is
    // the whole point of reporting 0.7.6: before SQ-1424 the selector was
    // unhandled and every one of these would have been a silent no-draw.
    let standing: Vec<_> = pics.iter().filter_map(|p| p.rule).collect();
    assert!(
        !standing.is_empty(),
        "imagetest's `scales` must reach glk_image_draw_scaled_ext; {} pictures, none with a rule",
        pics.len()
    );

    // And a standing rule must actually be standing: at least one of them
    // answers differently at two band widths. (A `WidthFixed`/`HeightFixed`
    // rule legitimately does not, so this asks for one that does rather than
    // requiring it of all of them.)
    let follows_the_pane = pics.iter().any(|p| {
        p.rule.is_some() && p.fitted_cells(40, (8, 16)) != p.fitted_cells(80, (8, 16))
    });
    assert!(
        follows_the_pane,
        "at least one of imagetest's scaled pictures must resize with the pane; rules seen: {standing:?}"
    );
}

/// A rule word naming no width rule or no height rule is invalid (the spec:
/// "You must supply one of each"), and the reference library answers false.
/// Nothing may be anchored for one.
#[test]
fn an_invalid_rule_word_draws_nothing() {
    let blorb = blorb_with_pict(1, &png(200, 100));
    let mut glk = app::glk_backend::AppGlk::with_graphics(
        80,
        24,
        (1, 1),
        app::graphics::PictSource::new(Some(blorb)),
    );
    glk.window_open(1, WinType::TextBuffer);
    // A missing Pict fails even with a valid rule…
    let good = ImageRule {
        rule: imagerule::WIDTH_ORIG | imagerule::HEIGHT_ORIG,
        width: 0,
        height: 0,
        maxwidth: 0,
    };
    assert!(!glk.buffer_draw_image_ext(1, /*resnum*/ 99, 1, good, 320), "no such picture");
    // …and a window that is not a text buffer has no inline flow to draw into.
    glk.window_open(5, WinType::Graphics);
    assert!(!glk.buffer_draw_image_ext(5, 1, 1, good, 320), "graphics windows go the other route");
    assert!(
        glk.take_transcript_elems().is_empty(),
        "neither failure anchored a picture in the transcript"
    );
}
