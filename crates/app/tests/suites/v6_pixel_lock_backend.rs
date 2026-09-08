//! **The pixel lock is a DEVICE-pixel guarantee, and half-blocks has no device
//! pixels.** SQ-0978.
//!
//! `v6_pixel_lock` (SQ-0936) floors the letterbox magnification to a rung of the
//! artwork's own ladder so that one *art* pixel is always a whole number of device
//! pixels. `v6_layout::locked_scale` delivers that by quantizing `s` against
//! `pane_dev = cells x picker.font_size()`, which is exact for kitty, iTerm2 and
//! sixel — each of them puts the composite on the screen at the pane's real
//! resolution.
//!
//! Half-blocks does not. `Picker::halfblocks()` hardcodes `FontSize::new(10, 20)`
//! whatever the terminal's font is (the crate's own comment calls it "completely
//! arbitrary"), and `Halfblocks::encode` then throws the font size away entirely and
//! resamples to exactly `width x 2·height` SAMPLES — one per column, two per row. So
//! the grid the picture resolves onto is a property of the CELL grid, and the device
//! pixels a rung was counted in are a number the picker invented.
//!
//! ## The decision, and the measurement that settled it
//!
//! The honest analogue is to quantize onto the sample grid instead — one art pixel on
//! a whole number of half-block samples, computable from `cols` and `2·rows` with no
//! font size anywhere. It was measured, and **it buys nothing**, because half-blocks
//! never magnifies at a size anyone runs: a 640x400 canvas has more unit pixels than a
//! terminal has samples until the pane reaches 640x200 CELLS, so the composite is
//! minified at every real size, and `resize_directional` minifies through `Triangle`.
//! Measured on a 640x400 canvas of 2x2 art pixels in hard black/white stripes
//! (`an_exact_rung_on_the_sample_grid_still_blends_two_art_pixels` below):
//!
//! ```text
//!   sample grid   ratio   samples that are a PURE art-pixel colour
//!   640x400        1:1    640 / 640     (Nearest — the target is not below the source)
//!   458x288       1.4:1     50 / 458
//!   320x200         2:1      0 / 320    ← an EXACT rung: one art pixel per sample
//!   160x100         4:1      0 / 160
//! ```
//!
//! The 320x200 row is the finding. It is the honest ladder's own rung — one art pixel
//! onto exactly one sample — and every sample still lands on a 25/75 blend of two art
//! pixels, because a separable Triangle at ratio 2 has support 2 in source space and
//! reaches across the art pixel's edge. And there is nothing below it to reach for
//! either: at 1:1 and above the sample grid is at or over the canvas, `pick` returns
//! `Nearest`, and the art is already pure without any lock — which the lock could only
//! move DOWN off, into Triangle. So there is no pane size at which a rung improves a
//! half-blocks frame, and a measured 17-20% of linear resolution to lose where it acts
//! (at a 120x40 pane a 640x400 canvas free-scales to 120x38 cells; the old device-pixel
//! rung cut it to 96x30).
//!
//! Hence: the lock is **inert** on half-blocks, and `/dump-terminal` reports it as
//! inert rather than as a snap that happened. Not a ceiling — SQ-0964 removed the one
//! half-blocks used to carry, and the free scale still climbs the whole way to the pane.
//!
//! ## What is pinned here
//!
//! Both directions, because "the lock does nothing" is only a fix if it also still does
//! everything on the backends it was written for:
//!
//!   * half-blocks — the locked frame is byte-for-byte the free one, and the state says
//!     INAPPLICABLE rather than FELL BACK (which would send a reader hunting for a
//!     bigger terminal);
//!   * kitty — the locked frame still differs from the free one, at the same panes and
//!     the same stories. Nothing changes for a kitty user.
//!
//! Specimens (release and turn count are part of the fixture — CLAUDE.md):
//!
//! ```text
//!   fixture                      release  turns  role
//!   journey-r83-s890706.z6          83      40   the menu plan
//!   zork0-r393-s890714.z6          393       6   the frame plan
//!   arthur-r74-s890714.z6           74      12   the extend plan
//! ```

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::v6_layout as v6;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui_image::picker::{Picker, ProtocolType};

/// One fixture: file, the key answered to a character read while tapping in, how many
/// taps reach the frame this case is about, and the release it must be holding.
struct Specimen {
    file: &'static str,
    keys: u8,
    taps: usize,
    release: u16,
}

const CORPUS: &[Specimen] = &[
    Specimen { file: "journey-r83-s890706.z6", keys: 13, taps: 40, release: 83 },
    Specimen { file: "zork0-r393-s890714.z6", keys: 13, taps: 6, release: 393 },
    Specimen { file: "arthur-r74-s890714.z6", keys: b'n', taps: 12, release: 74 },
];

/// Panes swept. Chosen so that the LADDER bites at both cells below — the locked rung
/// is strictly under the free scale — because a pane where the two coincide would let
/// the half-blocks case pass whether or not anything was fixed. The cases assert it.
const PANES: [(u16, u16); 3] = [(98, 37), (115, 45), (131, 41)];

/// The kitty control's cell. A real terminal font, not a nominal one.
const KITTY_CELL: (u16, u16) = (8, 18);

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a v6 story the way `startup.rs` does — the profile from the medium the MOUNT
/// returned, and the screen size through the whole `picts.std_window() →
/// native_std_window() → profile.std_window()` chain with `art_scale` beside it — then
/// tap in to the frame. `None` (with a SKIP note) when the gitignored fixture is absent.
fn boot(s: &Specimen) -> Option<(GameSession, (u32, u32))> {
    let path = stories_dir().join(s.file);
    let (bytes, medium) = match app::hints::load_mounted_story(&path) {
        Ok((loaded, medium)) => (loaded.bytes().to_vec(), medium),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            return None;
        }
    };
    let profile = InterpreterProfile::resolve(&path, None, None, medium);
    let mut picts = PictSource::resolve(&path, None);
    let dims = picts.all_pict_dims();
    let release = u16::from_be_bytes([bytes[2], bytes[3]]);
    assert_eq!(
        release, s.release,
        "{}: a disk image is a different BUILD, not the same story on other media — this case is \
         pinned to release {}",
        s.file, s.release
    );
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
    let std_win = boot.screen_px;
    let art_scale = boot.art_scale;
    eprintln!(
        "{}: booted as {profile:?} off {medium:?} · release {release} · screen {std_win:?} · art_scale {art_scale:?}",
        s.file
    );
    let mut session = GameSession::new_for_machine(bytes, true, false, false, dims, None, None, &boot)
    .unwrap_or_else(|e| panic!("{}: should boot without a ZError: {e:?}", s.file));
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..s.taps {
        let t = match session.pending_input() {
            InputKind::Line => session.submit("").transcript,
            InputKind::Char => session.submit_char(s.keys).transcript,
            InputKind::Event => session.submit("").transcript,
        };
        if t.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
    }
    Some((session, art_scale.unwrap_or((2, 2))))
}

/// A hybrid render under a named backend, with `v6_pixel_lock` as asked and the art
/// scale the mount resolved — the pair the ladder is derived from.
#[allow(deprecated)]
fn render(
    model: &app::engine::ScreenModel,
    transcript: &str,
    art_scale: (u32, u32),
    honor: bool,
    lock: bool,
    halfblocks: bool,
    pane: (u16, u16),
) -> (app::state::AppState, Buffer) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    // The REAL constructors: `Picker::halfblocks()` is what `--image-protocol
    // halfblocks` and every failed capability query leave behind, and its nominal
    // 10x20 is the whole crux of this quest.
    state.game_picker = Some(if halfblocks {
        Picker::halfblocks()
    } else {
        let mut p = Picker::from_fontsize(ratatui_image::FontSize::new(KITTY_CELL.0, KITTY_CELL.1));
        p.set_protocol_type(ProtocolType::Kitty);
        p
    });
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    state.config.v6_pixel_lock = lock;
    state.v6_art_scale = art_scale;
    for line in transcript.lines() {
        state.push_transcript(line);
    }
    let area = Rect::new(0, 0, pane.0, pane.1);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    (state, buf)
}

/// The cell size a picker reports, as a pair.
fn font(p: &Picker) -> (u16, u16) {
    let fs = p.font_size();
    (fs.width, fs.height)
}

// ── The crux, upstream ─────────────────────────────────────────────────────────

/// `Picker::halfblocks()` reports a NOMINAL cell whatever the terminal's font is, and
/// the lock's ladder was being counted in it.
///
/// Pinned rather than taken on faith: this is upstream behaviour in a pinned git
/// dependency, and if it ever became a real measurement the reasoning above would need
/// revisiting rather than silently continuing to hold.
#[test]
fn the_half_blocks_picker_reports_a_nominal_cell_and_the_lock_does_not_bind_it() {
    assert_eq!(
        font(&Picker::halfblocks()),
        (10, 20),
        "`Picker::halfblocks()` hardcodes FontSize::new(10, 20) — the crate calls it \
         \"completely arbitrary\", and it is the number `pane_dev` and every rung were \
         derived from"
    );
    assert!(
        !app::render::graphics::v6_pixel_lock_applies(&Picker::halfblocks()),
        "half-blocks resolves into cells, so a device-pixel rung means nothing there"
    );

    // Every backend that ships pixels keeps the lock exactly as SQ-0936 wrote it.
    #[allow(deprecated)]
    for proto in [ProtocolType::Kitty, ProtocolType::Sixel, ProtocolType::Iterm2] {
        let mut p = Picker::from_fontsize(ratatui_image::FontSize::new(KITTY_CELL.0, KITTY_CELL.1));
        p.set_protocol_type(proto);
        assert!(
            app::render::graphics::v6_pixel_lock_applies(&p),
            "{proto:?} puts the composite on the screen at the pane's real device \
             resolution, so the rung is exact there and must keep binding"
        );
    }
}

// ── The measurement the decision rests on ──────────────────────────────────────

/// **Quantizing onto the sample grid instead would buy nothing**, and this is why.
///
/// The honest analogue of the lock on half-blocks is one art pixel onto a whole number
/// of SAMPLES. This drives the canvas through the very resample `v6_halfblocks_protocol`
/// uses and counts how many samples come out a pure art-pixel colour. At the exact rung
/// — 320x200 samples for a 640x400 canvas of 2x2 art pixels, one art pixel per sample —
/// the answer is ZERO, so the rung delivers none of the crispness it exists for.
///
/// It is a fact about `resize_directional`'s filter choice, so it is measured rather
/// than reasoned: if `pick` ever gained an integer-decimation arm, this case fails and
/// the design decision above is genuinely back open.
#[test]
fn an_exact_rung_on_the_sample_grid_still_blends_two_art_pixels() {
    // A 640x400 canvas of 2x2 art pixels: hard black/white vertical stripes, so a
    // blended sample is unmistakable and a pure one is exactly 0 or 255.
    let mut canvas = image::RgbaImage::new(640, 400);
    for y in 0..400u32 {
        for x in 0..640u32 {
            let v = if (x / 2) % 2 == 0 { 0u8 } else { 255 };
            canvas.put_pixel(x, y, image::Rgba([v, v, v, 255]));
        }
    }

    // (sample grid, pure samples expected) — the module doc's table.
    let table = [((640u32, 400u32), 640usize), ((458, 288), 50), ((320, 200), 0), ((160, 100), 0)];
    for ((tw, th), want_pure) in table {
        let out = app::render::graphics::resize_directional(&canvas, tw, th);
        let pure = (0..tw).filter(|&x| matches!(out.get_pixel(x, th / 2)[0], 0 | 255)).count();
        assert_eq!(
            pure, want_pure,
            "{tw}x{th} samples from a 640x400 canvas of 2x2 art pixels: {pure} of {tw} samples \
             are a pure art-pixel colour, expected {want_pure}"
        );
    }

    // Restated as the finding, so a reader of a failure sees the claim and not just
    // two numbers: the rung the honest ladder would land on is 320x200, and it is the
    // row with nothing pure in it.
    let rung = app::render::graphics::resize_directional(&canvas, 320, 200);
    assert!(
        !matches!(rung.get_pixel(0, 100)[0], 0 | 255),
        "one art pixel onto exactly one sample is the honest ladder's own rung, and \
         Triangle still blends across it — which is why the lock is inert here rather \
         than re-derived"
    );
}

// ── The frame: half-blocks ─────────────────────────────────────────────────────

/// **On half-blocks the locked frame IS the free frame**, and the state says so as
/// INAPPLICABLE rather than as a fallback.
///
/// FALSIFY by dropping the `&& lock_applies` gate at either `v6_pixel_lock` site in
/// `render/screen.rs`: `journey-r83-s890706.z6` at 98x37 then renders a different
/// frame with the lock on than with it off — the old rung quantizes `s` from the free
/// 1.531 down to 1.5 in device pixels the backend never sees — and the case fails on
/// the buffer comparison.
#[test]
fn a_locked_half_blocks_frame_is_the_free_one() {
    let mut seen = 0usize;
    let mut any_present = false;
    for spec in CORPUS {
        any_present |= stories_dir().join(spec.file).exists();
        let Some((mut session, art_scale)) = boot(spec) else { continue };
        let transcript = session.take_transcript();
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else {
            panic!("{}: a v6 frame has a Layered root", spec.file)
        };
        let native = v6::native_extent(items.as_slice(), &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));

        for (pw, ph) in PANES {
            // Non-vacuity: the OLD behaviour must actually have moved this frame, or
            // "the lock changed nothing" is true by construction and pins nothing.
            // This is the arithmetic as it was — `pane_dev` off the nominal 10x20.
            let nominal = font(&Picker::halfblocks());
            let pane_dev = (u32::from(pw) * u32::from(nominal.0), u32::from(ph) * u32::from(nominal.1));
            let free = v6::uniform_scale(native, pane_dev).s;
            let old_rung = app::render::v6_layout::FrameGeometry::new(native, art_scale, zvm::screen::V6Cell::DEFAULT).locked_scale(pane_dev)
                .unwrap_or_else(|| panic!("{} {pw}x{ph}: a rung fits this pane", spec.file))
                .s;
            assert!(
                (free - old_rung).abs() > 1e-3,
                "{} r{} {pw}x{ph}: this sweep needs a pane where the old device-pixel rung \
                 DIFFERED from the free scale (free {free:.4} vs rung {old_rung:.4}), or the \
                 comparison below passes whether or not anything was fixed",
                spec.file, spec.release
            );

            for honor in [true, false] {
                let (free_state, free_buf) =
                    render(&model, &transcript, art_scale, honor, false, true, (pw, ph));
                let (lock_state, lock_buf) =
                    render(&model, &transcript, art_scale, honor, true, true, (pw, ph));

                // Non-vacuity: a real v6 pixel frame, not a boot prompt or the cell path.
                assert!(
                    !lock_state.v6_path_log.borrow().is_empty(),
                    "{} r{} {pw}x{ph} honor={honor}: this case measures a v6 pixel frame",
                    spec.file, spec.release
                );
                assert_eq!(
                    free_buf, lock_buf,
                    "{} r{} {pw}x{ph} honor={honor}: half-blocks has no device pixel for an art \
                     pixel to land a whole number of, so the lock must leave the frame exactly \
                     as free scaling drew it",
                    spec.file, spec.release
                );
                assert!(
                    lock_state.v6_scale_lock_inapplicable.get(),
                    "{} r{} {pw}x{ph} honor={honor}: a lock the backend cannot honour at any pane \
                     size must be reported as inapplicable",
                    spec.file, spec.release
                );
                assert!(
                    !lock_state.v6_scale_lock_fallback.get(),
                    "{} r{} {pw}x{ph} honor={honor}: this is NOT the too-small-pane fallback — \
                     reporting it as one would send a reader looking for a bigger terminal",
                    spec.file, spec.release
                );
                assert!(
                    !free_state.v6_scale_lock_inapplicable.get(),
                    "{} r{} {pw}x{ph} honor={honor}: the flag says the lock was ASKED for and \
                     could not apply, so a frame that never asked must not raise it",
                    spec.file, spec.release
                );
                seen += 1;
            }
        }
    }
    assert!(
        !any_present || seen > 0,
        "a v6 fixture is present but no frame was measured — the harness stopped reaching frames"
    );
}

/// The raster arm consults the lock too — `screen.rs` hands its own `locked_scale` to
/// `spawn_v6_encode` — so it gets the same gate, publishes the same flag, and encodes
/// the same composite locked as free.
///
/// FALSIFY by dropping the `&& lock_applies` from the raster arm alone: the composite
/// `v6_halfblocks_grid` lands on changes size and the cell comparison fails, which the
/// state flag on its own would not have caught.
#[test]
fn the_raster_arm_ignores_the_lock_on_a_cell_backend_too() {
    let mut seen = 0usize;
    let mut any_present = false;
    for spec in CORPUS {
        any_present |= stories_dir().join(spec.file).exists();
        let Some((mut session, art_scale)) = boot(spec) else { continue };
        let transcript = session.take_transcript();
        let model = session.screen();

        // The first raster frame after boot encodes SYNCHRONOUSLY (there is no
        // previous composite to redraw), so one render is enough to compare on.
        let raster = |lock: bool| {
            let mut state = app::state::AppState::default();
            state.colors = app::colors::ColorScheme::terminal_default();
            state.game_picker = Some(Picker::halfblocks());
            state.config.v6_render = app::config::V6RenderMode::Raster;
            state.config.v6_pixel_lock = lock;
            state.v6_art_scale = art_scale;
            for line in transcript.lines() {
                state.push_transcript(line);
            }
            let area = Rect::new(0, 0, PANES[0].0, PANES[0].1);
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
            (state, buf)
        };
        let (free_state, free_buf) = raster(false);
        let (lock_state, lock_buf) = raster(true);

        assert!(
            lock_state.v6_scale_lock_inapplicable.get(),
            "{} r{}: the raster arm asks the same picker the ring does",
            spec.file, spec.release
        );
        assert!(
            !free_state.v6_scale_lock_inapplicable.get(),
            "{} r{}: a frame that never asked for the lock must not raise the flag",
            spec.file, spec.release
        );
        assert_eq!(
            free_buf, lock_buf,
            "{} r{}: the raster composite resolves into the same half-block cells whether the \
             lock is on or off, because there is no rung for it to snap to",
            spec.file, spec.release
        );
        seen += 1;
    }
    assert!(!any_present || seen > 0, "a v6 fixture is present but no raster frame was measured");
}

// ── The control: nothing changes for a kitty user ──────────────────────────────

/// **Kitty still snaps.** The same stories at the same panes, and the locked frame is
/// still a different frame from the free one — which is the whole of SQ-0936 and must
/// survive a change made for a different backend.
///
/// FALSIFY by making `v6_pixel_lock_applies` answer `false` unconditionally: every
/// specimen then renders the same frame locked as free and this case fails.
#[test]
fn a_locked_kitty_frame_still_snaps_to_the_ladder() {
    let mut seen = 0usize;
    let mut any_present = false;
    for spec in CORPUS {
        any_present |= stories_dir().join(spec.file).exists();
        let Some((mut session, art_scale)) = boot(spec) else { continue };
        let transcript = session.take_transcript();
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else {
            panic!("{}: a v6 frame has a Layered root", spec.file)
        };
        let native = v6::native_extent(items.as_slice(), &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));

        for (pw, ph) in PANES {
            let pane_dev = (u32::from(pw) * u32::from(KITTY_CELL.0), u32::from(ph) * u32::from(KITTY_CELL.1));
            let (scale, fell_back) = app::render::v6_layout::FrameGeometry::new(native, art_scale, zvm::screen::V6Cell::DEFAULT).fitted_scale(pane_dev, true);
            assert!(
                !fell_back,
                "{} r{} {pw}x{ph}: the control is about the LOCKED fit; a pane too small for the \
                 lowest rung has nothing to say here",
                spec.file, spec.release
            );
            let free = v6::uniform_scale(native, pane_dev).s;
            assert!(
                (free - scale.s).abs() > 1e-3,
                "{} r{} {pw}x{ph}: the control needs a pane where the rung differs from the free \
                 scale (free {free:.4} vs rung {:.4})",
                spec.file, spec.release, scale.s
            );

            for honor in [true, false] {
                let (free_state, free_buf) =
                    render(&model, &transcript, art_scale, honor, false, false, (pw, ph));
                let (lock_state, lock_buf) =
                    render(&model, &transcript, art_scale, honor, true, false, (pw, ph));
                assert_ne!(
                    free_buf, lock_buf,
                    "{} r{} {pw}x{ph} honor={honor}: kitty has real device pixels, so the lock \
                     must still move the frame onto the artwork's ladder",
                    spec.file, spec.release
                );
                assert!(
                    !lock_state.v6_scale_lock_inapplicable.get() && !free_state.v6_scale_lock_inapplicable.get(),
                    "{} r{} {pw}x{ph} honor={honor}: the rung is exact on kitty and nothing about \
                     it is inapplicable",
                    spec.file, spec.release
                );
                seen += 1;
            }
        }
    }
    assert!(
        !any_present || seen > 0,
        "a v6 fixture is present but no frame was measured — the harness stopped reaching frames"
    );
}
