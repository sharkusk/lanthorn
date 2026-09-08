//! SQ-0741 — Zork Zero's drop-caps and room icons must float in the transcript
//! whether the art comes from a Blorb or from the original Amiga picture archive.
//!
//! The reported symptom, booting Zork Zero off its Amiga `.adf`: "NO dropcap or
//! map icons … map works, and room icon is shown there". The same pictures
//! rendered fine inside the in-game map and produced nothing inline, so the art
//! decoded — only the inline path failed.
//!
//! MEASURED CAUSE, and it is not the decoder: the game places its own inline art.
//! Zork Zero reads placement picture 478 through `picture_data` and adds it to the
//! text cursor before drawing. That placeholder is `2×1` in the native `Pic.data`
//! where the Blorb's `Rect` says `0×0`, so with native art the drop-cap is drawn
//! at cursor + (4, 2) in unit space instead of exactly on it:
//!
//! ```text
//! blorb   @draw_picture(number=2, window=0, y=17, x=1)  cy=17 cx=1  → at_cursor
//! native  @draw_picture(number=2, window=0, y=19, x=5)  cy=17 cx=1  → NOT at_cursor
//! ```
//!
//! `PictureEvent::at_cursor` is a pixel-exact `y == y_cursor`, so every drop-cap
//! and room icon failed it, fell through to the placed-art canvas path, and left
//! the transcript with nothing — while a window-0 canvas appeared that the Blorb
//! path never creates. The fix reads the game's OTHER declaration of intent: the
//! `set_margins` it issues immediately after the draw (ZMSD §15's margin picture),
//! which is the game reserving the column its prose will flow in.
//!
//! Both fixtures are gitignored, so every case **skips vacuously** when absent.

use std::path::PathBuf;

use app::graphics::PictSource;
use app::inline_image::ImageAlign;
use app::session::{GameSession, InputKind, TranscriptElem};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// The `(width, height, align)` of every inline float Zork Zero anchors into the
/// transcript over its opening turns, plus the set of window canvases it ended up
/// owning.
struct Opening {
    floats: Vec<(u32, u32, ImageAlign)>,
    canvases: Vec<u8>,
    /// The drop-cap's own pixels, for the render pass below.
    dropcap: Option<app::inline_image::InlineImage>,
}

/// Boot Zork Zero against `picts` and play through the opening, collecting the
/// transcript floats. `std_win` stands in for the Blorb `Reso` chunk a native
/// archive does not have — exactly what the Amiga interpreter profile supplies at
/// startup (`startup.rs`: `picts.std_window().or_else(|| profile.std_window())`).
fn opening(mut picts: PictSource, story: Vec<u8>, std_win: Option<(u16, u16)>, honor_game_colours: bool) -> Opening {
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = std_win.or_else(|| picts.std_window());
    let mut session = GameSession::new_with_trace(
        story,
        honor_game_colours,
        false,
        None,
        false,
        picture_dims,
        v6_screen_px,
        None,
        None,
    )
    .expect("Zork Zero (v6) loads and boots without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    let mut floats = Vec::new();
    let mut dropcap = None;
    for _ in 0..6 {
        let turn = match session.pending_input() {
            InputKind::Line => session.submit("look"),
            InputKind::Char => session.submit_char(b' '),
            InputKind::Event => session.submit(""),
        };
        for e in &turn.transcript_elems {
            if let TranscriptElem::Image(img) = e {
                floats.push((img.pixels.width(), img.pixels.height(), img.align));
                if dropcap.is_none() {
                    dropcap = Some(img.clone());
                }
            }
        }
    }
    let mut canvases: Vec<u8> = session.pictures_canvas.keys().copied().collect();
    canvases.sort_unstable();
    Opening { floats, canvases, dropcap }
}

fn story_bytes() -> Option<Vec<u8>> {
    let p = stories_dir().join("zork0-r393-s890714.z6");
    match std::fs::read(&p) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", p.display());
            None
        }
    }
}

/// The original Amiga `Pic.data`, renamed to the story stem by the SQ-0713
/// convention. A story plus a native archive and no Blorb anywhere — the shape
/// the `.adf` mount produces, without needing the disk image.
fn native_pics() -> Option<PictSource> {
    let p = stories_dir().join("zork0.pic");
    let Ok(raw) = std::fs::read(&p) else {
        eprintln!("SKIP: native picture archive missing at {}", p.display());
        return None;
    };
    Some(PictSource::from_native(blorb::infocom_pics::InfocomPics::parse(raw).expect("a native Infocom archive parses")))
}

fn blorb_pics(story_path: &std::path::Path) -> PictSource {
    PictSource::new(blorb::resolve_resource_blorb(story_path).map(|(b, _)| b))
}

/// The floats — the drop-caps and room icons the report is about.
///
/// This used to filter to `ImageSource::Story`, because a large graphics-window
/// draw ALSO echoed into the transcript as a `ContentSplash` band for frameless
/// mode, and the two archives legitimately differ on whether picture 5 is
/// full-screen art. SQ-0895 removed the mode and stopped emitting those bands at
/// the source, so the filter has nothing left to exclude and the projection is
/// what remains of it.
fn story_floats(o: &Opening) -> Vec<(u32, u32, ImageAlign)> {
    o.floats.clone()
}

/// The bug, stated as the user reported it: with native art the transcript gets
/// NO drop-cap and NO room icons at all.
///
/// Falsified by restoring `!ev.at_cursor` in `is_win0_inline_float`: this comes
/// back empty, and `canvases` grows a window 0 the Blorb path never creates.
#[test]
fn a_native_archive_still_floats_the_dropcap_and_room_icons() {
    for honor_game_colours in [true, false] {
        let Some(story) = story_bytes() else { return };
        let Some(picts) = native_pics() else { return };
        let native = opening(picts, story, Some((320, 200)), honor_game_colours);
        let floats = story_floats(&native);

        assert!(
            !floats.is_empty(),
            "honor_game_colours={honor_game_colours}: the native archive must anchor inline floats, not nothing"
        );
        // The opening drop-cap: Zork Zero's 42×35 initial letter, doubled into
        // unit space, floated at the left margin so the prose flows beside it.
        assert_eq!(
            floats[0],
            (84, 70, ImageAlign::MarginLeft),
            "honor_game_colours={honor_game_colours}: the opening drop-cap floats at the left margin"
        );
        // …and the room icons that follow it, 21×21 tiles doubled to 42×42.
        assert!(
            floats[1..].iter().all(|&f| f == (42, 42, ImageAlign::MarginLeft)),
            "honor_game_colours={honor_game_colours}: every room icon floats too, got {floats:?}"
        );
        assert!(
            floats.len() >= 4,
            "honor_game_colours={honor_game_colours}: the opening carries a drop-cap and several room icons, got {}",
            floats.len()
        );

        // A cursor-anchored window-0 picture must never take a window canvas —
        // the same invariant `v6_arthur_intro_plates` pins from the other side.
        assert!(
            !native.canvases.contains(&0),
            "honor_game_colours={honor_game_colours}: an inline float must not create a window-0 canvas, got {:?}",
            native.canvases
        );
    }
}

/// The Blorb path is the pinned corpus and must be untouched: the same story
/// through `Zork0.blb` produces exactly the floats it always did, and the native
/// archive now agrees with it picture for picture.
#[test]
fn the_blorb_path_is_unchanged_and_the_two_archives_agree() {
    for honor_game_colours in [true, false] {
        let Some(story) = story_bytes() else { return };
        let story_path = stories_dir().join("zork0-r393-s890714.z6");
        let blorb = opening(blorb_pics(&story_path), story.clone(), None, honor_game_colours);
        let blorb_floats = story_floats(&blorb);

        assert_eq!(
            blorb_floats[0],
            (84, 70, ImageAlign::MarginLeft),
            "honor_game_colours={honor_game_colours}: the Blorb drop-cap is where it always was"
        );
        assert!(
            blorb_floats[1..].iter().all(|&f| f == (42, 42, ImageAlign::MarginLeft)),
            "honor_game_colours={honor_game_colours}: the Blorb room icons are unchanged, got {blorb_floats:?}"
        );
        assert_eq!(
            blorb.canvases,
            vec![1, 7],
            "honor_game_colours={honor_game_colours}: the Blorb path still paints exactly the banner and frame canvases"
        );

        let Some(picts) = native_pics() else { return };
        let native = opening(picts, story, Some((320, 200)), honor_game_colours);
        assert_eq!(
            story_floats(&native),
            blorb_floats,
            "honor_game_colours={honor_game_colours}: the archive the art came out of must not change what floats"
        );
        assert_eq!(
            native.canvases, blorb.canvases,
            "honor_game_colours={honor_game_colours}: nor which windows own canvases"
        );
    }
}

/// End to end through the shipped renderer: the drop-cap that came out of the
/// native archive really reaches the terminal.
///
/// Halfblocks is the honest oracle — it writes real cell colours rather than an
/// escape sequence the buffer cannot see — so a painted band row is a cell that
/// is no longer the blank the transcript started with.
#[test]
fn the_native_dropcap_reaches_the_rendered_transcript() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    for honor_game_colours in [true, false] {
        let Some(story) = story_bytes() else { return };
        let Some(picts) = native_pics() else { return };
        let native = opening(picts, story, Some((320, 200)), honor_game_colours);
        let dropcap = native.dropcap.expect("the native archive produced a Story float");

        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.honor_game_colours = honor_game_colours;
        state.push_transcript_image(dropcap);
        state.push_transcript("Behind the barred door of the treasury.");

        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        let status = app::engine::StatusModel::HostManaged;
        app::render::transcript::render_transcript(&status, None, &state, area, &mut buf, None);

        let painted = (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| buf.cell((x, y)).is_some_and(|c| c.style().fg.is_some() || c.style().bg.is_some()))
            .count();
        assert!(
            painted > 0,
            "honor_game_colours={honor_game_colours}: the drop-cap band must paint cells in the transcript"
        );

        // RASTER takes the same `transcript_images` sidecar the hybrid transcript
        // does — one representation, both modes — so the float must appear there
        // too, and as a left-margin float with prose beside it rather than a band.
        let (main, _) = app::render::screen::build_main_text(&state, 60, 20);
        assert_eq!(
            main.floats.len(),
            1,
            "honor_game_colours={honor_game_colours}: the raster composite floats the same drop-cap"
        );
        assert!(
            main.floats[0].reserve_cols > 0,
            "honor_game_colours={honor_game_colours}: and reserves the game's own margin for the prose beside it"
        );
    }
}
