//! SQ-1475: the US S.A.G.A. releases' own **family-C** strip bitmaps (spec
//! §8.3), decoded straight off the release disk beside the database, must
//! reach the SAME picture band the `.blb` releases and the Commodore 64
//! *Mysterious Adventures* vector artwork already draw through — same rows
//! reserved above the room panel, same `GraphicsWindow` shape, same
//! backend-neutral renderer, no protocol special-casing anywhere on the way.
//!
//! `stories/scott-dialects/c64/QUESTPR1.D64` (*The Hulk*, §10.7) and
//! `stories/golden_baton.blb` (a different game, Blorb-carried pictures) are
//! the side-by-side oracle, exactly as `scott_c64_native_pictures.rs` pairs
//! the native and Blorb *Golden Baton*: two picture sources of very different
//! shapes, so agreement on the BAND GEOMETRY — never on pixel content — is
//! what a shared render path actually promises. Both are gitignored commercial
//! fixtures and every case here skips vacuously without them.
//!
//! The Atari case is deliberately the *absence* of pictures; see
//! [`an_atari_saga_side_a_reports_no_pictures_because_the_sides_are_not_paired`].

use app::engine::{Engine, GraphicsWindow, WinNode};
use app::render::graphics::{kitty_picker, GraphicsRender};
use app::scott_session::ScottSession;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui_image::picker::Picker;

use crate::fixture_paths::fixture_path;

/// *The Hulk* off `QUESTPR1.D64`, booted the way `startup.rs` boots it — the
/// mount hands over both the database and the seventy family-C picture files,
/// because the mount does not outlive the load and a hand-assembled pair would
/// be measuring a launch the app never performs.
fn hulk_session() -> Option<ScottSession> {
    let path = fixture_path("scott-dialects/c64/QUESTPR1.D64");
    if !path.exists() {
        return None;
    }
    let mounted = app::hints::load_mounted_story_full(&path, None).ok()?;
    let app::hints::LoadedStory::Scott(bytes) = mounted.story else {
        panic!("QUESTPR1.D64's one story is a Scott Adams database");
    };
    Some(
        ScottSession::new_with_options(
            bytes,
            false,
            None,
            scott::Options::default(),
            app::graphics::ScottPictureSources::none().with_saga_pictures(mounted.saga_pictures),
        )
        .expect("the Hulk boots off its own release disk"),
    )
}

fn golden_baton_blb() -> Option<ScottSession> {
    let raw = std::fs::read(fixture_path("golden_baton.blb")).ok()?;
    let blorb = blorb::Blorb::parse(raw.clone()).expect("golden_baton.blb is a valid blorb");
    let app::hints::LoadedStory::Scott(bytes) = app::hints::extract_story(raw).expect("extract")
    else {
        panic!("golden_baton.blb holds a Scott Adams story");
    };
    Some(ScottSession::new(bytes, Some(blorb)).expect("golden_baton.blb boots"))
}

fn picture_band(model: &app::engine::ScreenModel) -> Option<&GraphicsWindow> {
    match &model.root {
        WinNode::Pair { first, .. } => match &**first {
            WinNode::Graphics(gw) => Some(gw),
            _ => None,
        },
        _ => None,
    }
}

fn reserved_rows(model: &app::engine::ScreenModel) -> Option<u16> {
    match &model.root {
        WinNode::Pair { split, first, .. } if matches!(**first, WinNode::Graphics(_)) => {
            Some(split.fixed)
        }
        _ => None,
    }
}

fn both_sessions() -> Option<(ScottSession, ScottSession)> {
    match (hulk_session(), golden_baton_blb()) {
        (Some(h), Some(b)) => Some((h, b)),
        _ => {
            eprintln!(
                "SKIP: needs both stories/scott-dialects/c64/QUESTPR1.D64 and \
                 stories/golden_baton.blb (gitignored commercial fixtures)"
            );
            None
        }
    }
}

/// The family-C band reserves the same rows, in the same window slot, as the
/// Blorb one — one `PICTURE_ROWS` constant, one layout, three sources.
///
/// `honor_game_colours` plays no part in a room-picture band (it governs
/// TEXT-cell colour resolution; the band is a raw RGBA canvas placed by
/// `GraphicsRender::render`), and CLAUDE.md's "pin both modes" rule exists for
/// regressions in that text path — so this runs both ways to document, rather
/// than assume, that it has no effect here.
#[test]
fn saga_and_blorb_room1_bands_reserve_the_same_rows_and_upscale() {
    for honor_game_colours in [true, false] {
        let Some((hulk, blorbed)) = both_sessions() else { return };
        let _ = honor_game_colours;

        assert_eq!(hulk.current_location().unwrap().number, 1, "Banner starts in room 1");
        assert_eq!(blorbed.current_location().unwrap().number, 1);

        let saga_model = hulk.screen();
        let blorb_model = blorbed.screen();
        let saga_rows = reserved_rows(&saga_model).expect("the Hulk's room 1 shows a band");
        let blorb_rows = reserved_rows(&blorb_model).expect("blorb room 1 shows a band");
        assert_eq!(
            saga_rows, blorb_rows,
            "the picture band reserves the SAME rows regardless of source"
        );

        let saga_gw = picture_band(&saga_model).unwrap();
        let blorb_gw = picture_band(&blorb_model).unwrap();
        assert!(saga_gw.upscale, "the family-C source stretches into the band");
        assert!(blorb_gw.upscale, "the blorb source stretches into the band");
        assert_eq!(saga_gw.win, blorb_gw.win, "both occupy the same window slot");
        assert_eq!(
            (saga_gw.canvas.width(), saga_gw.canvas.height()),
            (
                scott::saga_pictures::CANVAS_WIDTH as u32,
                scott::saga_pictures::CANVAS_HEIGHT as u32
            ),
            "family C's own 280x160 canvas — §8.3 states 158 and the records say 160"
        );
    }
}

/// The bounding rect of every cell in `area` the render actually touched.
fn touched_rect(buf: &Buffer, area: Rect, letterbox_bg: Color) -> Option<Rect> {
    let (mut x0, mut y0, mut x1, mut y1) = (None, None, None, None);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let Some(cell) = buf.cell((x, y)) else { continue };
            if cell.symbol() == " " && cell.bg == letterbox_bg {
                continue;
            }
            x0 = Some(x0.map_or(x, |v: u16| v.min(x)));
            x1 = Some(x1.map_or(x, |v: u16| v.max(x)));
            y0 = Some(y0.map_or(y, |v: u16| v.min(y)));
            y1 = Some(y1.map_or(y, |v: u16| v.max(y)));
        }
    }
    match (x0, x1, y0, y1) {
        (Some(x0), Some(x1), Some(y0), Some(y1)) => {
            Some(Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1))
        }
        _ => None,
    }
}

fn render_band(picker: &Picker, gw: &GraphicsWindow, area: Rect, letterbox: Style) -> Buffer {
    let mut buf = Buffer::empty(area);
    let mut gr = GraphicsRender::default();
    gr.render(picker, gw, area, letterbox, &mut buf);
    buf
}

/// The same placement under half-blocks and under kitty, and the same
/// placement the Blorb source gets — the whole claim that family C goes
/// through the shared, backend-neutral path.
#[test]
fn saga_and_blorb_room1_bands_place_identically_under_halfblocks_and_kitty() {
    let Some((hulk, blorbed)) = both_sessions() else { return };
    let saga_model = hulk.screen();
    let blorb_model = blorbed.screen();
    let saga_gw = picture_band(&saga_model).expect("family-C band");
    let blorb_gw = picture_band(&blorb_model).expect("blorb band");

    // A tall, narrow pane: both canvases are far wider than tall (280x160 and
    // the blorb's 256x96), so the WIDTH binds the aspect-preserving fit for
    // both — the axis the fit does not saturate is what would expose a
    // per-source drift.
    let area = Rect::new(0, 0, 30, 16);
    let letterbox_color = Color::Rgb(0, 0, 0);
    let letterbox = Style::default().bg(letterbox_color);

    let hb = Picker::halfblocks();
    let saga_hb = touched_rect(&render_band(&hb, saga_gw, area, letterbox), area, letterbox_color)
        .expect("the family-C band draws something under half-blocks");
    let blorb_hb = touched_rect(&render_band(&hb, blorb_gw, area, letterbox), area, letterbox_color)
        .expect("the blorb band draws something under half-blocks");
    // Both are full-width and vertically centred, and they differ in HEIGHT
    // by exactly the difference in their own aspects — which is the shared
    // fit doing its job on each canvas's real size, not a family-C branch.
    // 280x160 is 1.75:1, so 30 cells of width is 30/1.75 = 17.1 half-block
    // pixels, 9 cells, centred at row (16 - 9) / 2 = 3. The Blorb's 256x96 is
    // 2.67:1, so 11.25 pixels, 6 cells, centred at row 5. Pinned rather than
    // floored: a decode that produced some other canvas size would land
    // somewhere else, and the point of the pin is to say where.
    assert_eq!(saga_hb, Rect::new(0, 3, 30, 9), "family C's 280x160 fitted into the band");
    assert_eq!(blorb_hb, Rect::new(0, 5, 30, 6), "the Blorb's 256x96 into the same band");
    assert_eq!(saga_hb.x, blorb_hb.x, "both bind on width and start at the left edge");
    assert_eq!(saga_hb.width, blorb_hb.width);
    for rect in [saga_hb, blorb_hb] {
        assert_eq!(
            rect.y,
            (area.height - rect.height) / 2,
            "{rect:?} is vertically centred in the band"
        );
    }

    let kitty = kitty_picker(10, 20);
    let saga_kitty =
        touched_rect(&render_band(&kitty, saga_gw, area, letterbox), area, letterbox_color)
            .expect("the family-C band places under kitty");
    let blorb_kitty =
        touched_rect(&render_band(&kitty, blorb_gw, area, letterbox), area, letterbox_color)
            .expect("the blorb band places under kitty");
    assert_eq!(saga_kitty, blorb_kitty, "same placement rect under kitty");
    assert_eq!(saga_kitty, area, "kitty's explicit r×c grid covers the whole window");
}

/// The non-flat guard: room 1 is a drawing, not a fill.
///
/// This is the case that would catch a decode that produced a
/// correctly-shaped, correctly-placed canvas of nothing — the failure every
/// other number in this suite is blind to, because a blank canvas has exactly
/// the right dimensions and places exactly the same way.
#[test]
fn the_hulks_room_one_is_a_drawing_and_not_a_flat_fill() {
    let Some(hulk) = hulk_session() else {
        eprintln!("SKIP: needs stories/scott-dialects/c64/QUESTPR1.D64");
        return;
    };
    let model = hulk.screen();
    let gw = picture_band(&model).expect("room 1 has a band");
    let mut counts: std::collections::HashMap<(u8, u8, u8), usize> = std::collections::HashMap::new();
    for p in gw.canvas.pixels() {
        *counts.entry((p.0[0], p.0[1], p.0[2])).or_default() += 1;
        assert_eq!(p.0[3], 255, "family C carries no transparent index");
    }
    assert_eq!(counts.len(), 4, "four colours drawn, got {counts:?}");
    let total: usize = counts.values().sum();
    let biggest = *counts.values().max().expect("non-empty");
    assert!(
        biggest * 5 < total * 4,
        "no colour covers four fifths of the canvas — {biggest}/{total} looks like a fill"
    );
    // The palette §8.3's Commodore 64 table resolves for `R01001`: black,
    // orange, purple, white.
    for want in [(0u8, 0u8, 0u8), (186, 134, 32), (177, 89, 185), (255, 255, 255)] {
        assert!(counts.contains_key(&want), "room 1 draws {want:?}, got {counts:?}");
    }
}

/// An Atari US S.A.G.A. side A opens and plays, and reports **no pictures**.
///
/// That is the correct answer today and worth pinning as one: §8.3 puts the
/// Atari's pictures on the companion picture side, reached by hard-coded byte
/// offsets rather than through a catalogue, and §12.10 says those per-title
/// lists "are not recoverable from the database". `cli_host::disk_set` pairs
/// disks by FILENAME for the multi-disk Z-machine releases, and nothing pairs
/// a S.A.G.A. side A with its side B — so the picture side is not even
/// mounted, let alone indexed. A future lane that wires the pairing and the
/// offset lists will change this case; until then the honest report is "the
/// pictures are not on this file", not a blank band that reads as a text-only
/// game.
#[test]
fn an_atari_saga_side_a_reports_no_pictures_because_the_sides_are_not_paired() {
    let path = fixture_path("scott-dialects/atari/SAGA #1 - Adventureland [side A].atr");
    if !path.exists() {
        eprintln!("SKIP: needs stories/scott-dialects/atari/ (gitignored commercial fixtures)");
        return;
    }
    let mounted = app::hints::load_mounted_story_full(&path, None)
        .expect("the Atari side A mounts and holds one Scott database");
    assert!(
        mounted.saga_pictures.is_empty(),
        "an Atari side A carries no family-C picture files in its catalogue"
    );
    let app::hints::LoadedStory::Scott(bytes) = mounted.story else {
        panic!("side A's story is a Scott Adams database");
    };
    assert_eq!(
        scott::detect_saga_us(&bytes),
        Some(scott::SagaPlatform::Atari8Bit),
        "premise: it really is a US S.A.G.A. database on the Atari"
    );
    let session = ScottSession::new_with_options(
        bytes,
        false,
        None,
        scott::Options::default(),
        app::graphics::ScottPictureSources::none().with_saga_pictures(mounted.saga_pictures),
    )
    .expect("Adventureland boots off its own side A");
    assert!(picture_band(&session.screen()).is_none(), "no band without the picture side");
    let dump = session.window_dump().join("\n");
    assert!(
        dump.contains("a S.A.G.A. release with no picture files on this file"),
        "the dump distinguishes this from a text-only game:\n{dump}"
    );
}

/// SQ-1487 (user-reported): *The Hulk*'s own opening — typing `BITE LIP` in
/// room 1 — runs an action that draws several pictures in a row, each
/// waiting for the player to press ENTER (spec §12.11), and only the LAST
/// one used to show (`pending_picture` is one slot; every opcode-90 request
/// in the same turn silently overwrote it).
///
/// This is the render layer's half of the fix, alongside
/// `scott_session::tests::bite_lip_shows_every_picture_in_sequence_each_key_gated`
/// (`crates/app/src/scott_session.rs`, which pins the exact picture numbers
/// measured off this disk — 84, 83, 86, then room 2): mid-sequence, the band
/// must reserve the SAME rows a room's own picture does (one shared
/// `GraphicsWindow` layout, not a special case for a sequence frame), and
/// the keypress hint must actually reach the player. There is no dedicated
/// "press a key" element in the app's main input bar (`render_input_content`
/// just hides it outright whenever `pending_input()` answers `Char`, for
/// every engine) — SQ-1487 rides the existing general-purpose
/// `TurnResult::info` door instead, which the render loop (`turn.rs`) pushes
/// into the transcript with the ordinary transcript style. So "the input bar
/// shows the keypress hint" is checked at that door.
#[test]
fn bite_lip_mid_sequence_reserves_the_same_band_as_a_room_picture_and_shows_the_keypress_hint() {
    let Some(mut hulk) = hulk_session() else {
        eprintln!("SKIP: needs stories/scott-dialects/c64/QUESTPR1.D64 (gitignored commercial fixture)");
        return;
    };
    assert_eq!(hulk.current_location().unwrap().number, 1, "premise: Bruce Banner starts in room 1");
    let room_model = hulk.screen();
    let room_rows = reserved_rows(&room_model).expect("room 1 shows a band");
    let room_gw = picture_band(&room_model).expect("room 1 shows a band");

    let first = hulk.submit("bite lip");
    assert_eq!(
        hulk.pending_input(),
        app::session::InputKind::Char,
        "the sequence's first picture waits for a keypress, not a line"
    );
    assert_eq!(
        first.info.as_deref(),
        Some("[Press RETURN to continue]"),
        "the keypress hint rides TurnResult::info, the app's general-purpose \
         one-line-note door (turn.rs pushes it into the transcript)"
    );
    assert!(
        !first.transcript.contains("Tell me what to do"),
        "no prompt while the sequence is still presenting: {:?}",
        first.transcript
    );

    let mid_model = hulk.screen();
    let mid_rows = reserved_rows(&mid_model).expect("the sequence frame shows a band too");
    let mid_gw = picture_band(&mid_model).expect("the sequence frame shows a band too");
    assert_eq!(mid_rows, room_rows, "the sequence band reserves the SAME rows as a room picture");
    assert_eq!(mid_gw.win, room_gw.win, "same window slot");
    assert!(mid_gw.upscale, "the sequence picture stretches into the band exactly like a room one");
    assert_ne!(
        mid_gw.canvas.as_raw(),
        room_gw.canvas.as_raw(),
        "the band actually switched to the sequence's own picture (84), not room 1's"
    );
}

// ── SQ-1476: the Apple II releases, picture family D ─────────────────────────

/// *Adventureland* on the Apple II (§10.6), booted the way `startup.rs` boots
/// it — from the **boot** side, whose catalogue holds the database and no
/// artwork at all. Everything this fixture proves rides on the mount having
/// reached the companion side for the pictures.
fn adventureland_apple_session() -> Option<ScottSession> {
    let path = fixture_path(
        "scott-dialects/apple/Scott Adams Graphic Adventure 1 - Adventureland v2.1-416 \
         (4am crack) side B - boot.dsk",
    );
    if !path.exists() {
        eprintln!("SKIP: needs stories/scott-dialects/apple/ (gitignored commercial fixtures)");
        return None;
    }
    let mounted = app::hints::load_mounted_story_full(&path, None).ok()?;
    let app::hints::LoadedStory::Scott(bytes) = mounted.story else {
        panic!("the boot side's one story is a Scott Adams database");
    };
    assert_eq!(
        mounted.saga_pictures.len(),
        93,
        "the mount reached the companion side's picture files (§10.6)"
    );
    Some(
        ScottSession::new_with_options(
            bytes,
            false,
            None,
            scott::Options::default(),
            app::graphics::ScottPictureSources::none().with_saga_pictures(mounted.saga_pictures),
        )
        .expect("Adventureland boots off its own release disks"),
    )
}

/// Family D reaches the same band as family C and as a Blorb — same rows
/// reserved, same window slot, same aspect-preserving fit — on its own canvas.
///
/// Both `honor_game_colours` modes, for the reason the family-C case above
/// gives: the flag governs TEXT-cell colour resolution and a room-picture band
/// is a raw RGBA canvas, so this documents rather than assumes that it has no
/// effect here.
#[test]
fn the_apple_ii_band_matches_the_blorb_band_in_both_colour_modes() {
    for honor_game_colours in [true, false] {
        let (Some(apple), Some(blorbed)) = (adventureland_apple_session(), golden_baton_blb())
        else {
            return;
        };
        let _ = honor_game_colours;

        let apple_model = apple.screen();
        let blorb_model = blorbed.screen();
        assert_eq!(
            reserved_rows(&apple_model).expect("the Apple II band"),
            reserved_rows(&blorb_model).expect("the blorb band"),
            "the picture band reserves the SAME rows regardless of source"
        );
        let apple_gw = picture_band(&apple_model).unwrap();
        let blorb_gw = picture_band(&blorb_model).unwrap();
        assert!(apple_gw.upscale, "the family-D source stretches into the band");
        assert_eq!(apple_gw.win, blorb_gw.win, "both occupy the same window slot");
        assert_eq!(
            (apple_gw.canvas.width(), apple_gw.canvas.height()),
            (
                scott::apple_pictures::CANVAS_WIDTH as u32,
                scott::apple_pictures::CANVAS_HEIGHT as u32
            ),
            "family D's own canvas — the Apple II hi-res page, not family C's 280x160"
        );
    }
}

/// The same placement under half-blocks and under kitty, through the shared
/// backend-neutral path — and a canvas of its own aspect, so the numbers are
/// not family C's.
#[test]
fn the_apple_ii_band_places_identically_under_halfblocks_and_kitty() {
    let Some(apple) = adventureland_apple_session() else { return };
    let model = apple.screen();
    let gw = picture_band(&model).expect("family-D band");

    let area = Rect::new(0, 0, 30, 16);
    let letterbox_color = Color::Rgb(0, 0, 0);
    let letterbox = Style::default().bg(letterbox_color);

    let hb = Picker::halfblocks();
    let placed = touched_rect(&render_band(&hb, gw, area, letterbox), area, letterbox_color)
        .expect("the family-D band draws something under half-blocks");
    // 280x192 is 1.458:1, so 30 cells of width is 30/1.458 = 20.6 half-block
    // pixels — about 10 cells — centred in the sixteen the band offers.
    //
    // What is measured is the INKED bounding box, not the placement rect:
    // `touched_rect` ignores a cell left at the letterbox colour, and family D
    // is line art on black, so its outermost rows are background and are not
    // counted. That makes this pin strictly stronger than a placement pin — it
    // fails if the drawing moves OR if it changes — and it is why the number is
    // 8 rows rather than the ~10 the aspect alone would give. Family C's
    // 280x160 lands at `Rect::new(0, 3, 30, 9)`: a different canvas, a
    // different rect, which is exactly what should happen.
    assert_eq!(placed, Rect::new(0, 2, 30, 8), "family D's 280x192 fitted into the band");

    let kitty = kitty_picker(10, 20);
    let placed_kitty =
        touched_rect(&render_band(&kitty, gw, area, letterbox), area, letterbox_color)
            .expect("the family-D band places under kitty");
    assert_eq!(placed_kitty, area, "kitty's explicit r×c grid covers the whole window");
}

/// The non-flat guard: the opening room is a drawing, not a fill — the failure
/// every other number in these cases is blind to.
#[test]
fn the_apple_ii_opening_room_is_a_drawing_and_not_a_flat_fill() {
    let Some(apple) = adventureland_apple_session() else { return };
    let model = apple.screen();
    let gw = picture_band(&model).expect("the start room has a band");
    let mut counts: std::collections::HashMap<(u8, u8, u8), usize> = std::collections::HashMap::new();
    for p in gw.canvas.pixels() {
        *counts.entry((p.0[0], p.0[1], p.0[2])).or_default() += 1;
        assert_eq!(p.0[3], 255, "family D carries no transparent index");
    }
    assert_eq!(counts.len(), 2, "line art: ink and ground, got {counts:?}");
    let ink = counts[&scott::apple_pictures::INK];
    let total: usize = counts.values().sum();
    assert!(ink > 500, "only {ink} inked pixels of {total}, which is not a drawing");
    assert!(ink * 2 < total, "the ink covers half the canvas — {ink}/{total} looks like a fill");
}

/// One of the three **scrambled** Apple II releases (§7.4's string test, §10.6)
/// opens and plays and reports no pictures — because its side A is not a DOS
/// 3.3 disk at all and its room artwork sits at the per-title offsets §12.10
/// says are not recoverable. The honest report, pinned as one, exactly as the
/// Atari case above is.
#[test]
fn a_scrambled_apple_ii_release_reports_no_pictures() {
    let path = fixture_path(
        "scott-dialects/apple/Scott Adams Graphic Adventure 5 - The Count v2.1-115 \
         (4am crack) side B - boot.dsk",
    );
    if !path.exists() {
        eprintln!("SKIP: needs stories/scott-dialects/apple/ (gitignored commercial fixtures)");
        return;
    }
    assert!(
        app::hints::saga_apple_scrambled(&path),
        "premise: §7.4's string test fires on The Count's M2"
    );
    let mounted = app::hints::load_mounted_story_full(&path, None)
        .expect("the boot side mounts and holds one Scott database");
    assert!(
        mounted.saga_pictures.is_empty(),
        "a scrambled release's artwork is not in any catalogue this build can walk"
    );
    let app::hints::LoadedStory::Scott(bytes) = mounted.story else {
        panic!("the boot side's story is a Scott Adams database");
    };
    assert_eq!(
        scott::detect_saga_us(&bytes),
        Some(scott::SagaPlatform::AppleII),
        "premise: it really is a US S.A.G.A. database on the Apple II"
    );
    let session = ScottSession::new_with_options(
        bytes,
        false,
        None,
        scott::Options::default(),
        app::graphics::ScottPictureSources::none().with_saga_pictures(mounted.saga_pictures),
    )
    .expect("The Count boots off its own boot side");
    assert!(picture_band(&session.screen()).is_none(), "no band without readable artwork");
    let dump = session.window_dump().join("\n");
    assert!(
        dump.contains("a S.A.G.A. release with no picture files on this file"),
        "the dump distinguishes this from a text-only game:\n{dump}"
    );
}

// ── SQ-1482: §12.11's object overlays ────────────────────────────────────────

/// An overlaid band is still just a band.
///
/// §12.11 has the objects in a room drawn OVER the room picture, and the
/// composite is done where the picture is decoded — so what reaches the
/// renderer is one canvas of exactly the same shape as any other, placed by
/// exactly the same aspect-preserving fit. The failure this guards is a
/// composite that resized the canvas (an overlay whose own rectangle ran past
/// 280x160, say), which would move the band's rect and letterbox differently
/// from every other frame in the game.
///
/// Both `honor_game_colours` modes, for the reason
/// [`saga_and_blorb_room1_bands_reserve_the_same_rows_and_upscale`] gives:
/// the band is a raw RGBA canvas and the flag governs TEXT-cell colour, so
/// this runs both ways to document rather than assume that it has no effect
/// on an overlaid frame either.
#[test]
fn an_overlaid_band_places_exactly_where_a_plain_room_band_does() {
    for honor_game_colours in [true, false] {
        let Some(mut hulk) = hulk_session() else {
            eprintln!(
                "SKIP: needs stories/scott-dialects/c64/QUESTPR1.D64 (gitignored commercial \
                 fixture)"
            );
            return;
        };
        let _ = honor_game_colours;
        assert_eq!(hulk.current_location().unwrap().number, 1, "premise: Banner starts in room 1");
        let plain_model = hulk.screen();
        let plain_rows = reserved_rows(&plain_model).expect("room 1 shows a band");
        let plain_gw = picture_band(&plain_model).expect("room 1 shows a band");
        let plain_canvas = (plain_gw.canvas.width(), plain_gw.canvas.height());
        let plain_raw = plain_gw.canvas.as_raw().clone();
        let plain_rects: Vec<Rect> = placements(plain_gw);

        // `BITE LIP` and its three ENTER-gated scenes end in the dome, whose
        // two items with artwork (the sign and the iron ring set in the
        // floor) are drawn over room picture 2.
        hulk.submit("bite lip");
        for _ in 0..4 {
            hulk.submit_key(app::engine::KeyInput::Enter);
        }
        assert_eq!(hulk.current_location().unwrap().number, 2, "the opening ends in the dome");
        let dump = hulk.window_dump().join("\n");
        assert!(
            dump.contains("overlays=B01053R,B01033R"),
            "premise: this frame really is an overlaid one:\n{dump}"
        );

        let over_model = hulk.screen();
        let over_rows = reserved_rows(&over_model).expect("the dome shows a band");
        let over_gw = picture_band(&over_model).expect("the dome shows a band");
        assert_eq!(over_rows, plain_rows, "an overlaid band reserves the same rows");
        assert_eq!(over_gw.win, plain_gw.win, "same window slot");
        assert_eq!(over_gw.upscale, plain_gw.upscale, "same fit");
        assert_eq!(
            (over_gw.canvas.width(), over_gw.canvas.height()),
            plain_canvas,
            "the composite writes into the room picture's canvas and never resizes it"
        );
        assert_ne!(over_gw.canvas.as_raw(), &plain_raw, "and it really did change the pixels");
        assert_eq!(placements(over_gw), plain_rects, "same placement rect, overlays or not");
    }
}

/// The band's touched rect under half-blocks and under kitty — the two
/// backends `saga_and_blorb_room1_bands_place_identically_under_halfblocks_and_kitty`
/// pins, measured the same way.
fn placements(gw: &GraphicsWindow) -> Vec<Rect> {
    let area = Rect::new(0, 0, 30, 16);
    let letterbox_color = Color::Rgb(0, 0, 0);
    let letterbox = Style::default().bg(letterbox_color);
    [Picker::halfblocks(), kitty_picker(10, 20)]
        .iter()
        .map(|picker| {
            touched_rect(&render_band(picker, gw, area, letterbox), area, letterbox_color)
                .expect("the band draws something")
        })
        .collect()
}
