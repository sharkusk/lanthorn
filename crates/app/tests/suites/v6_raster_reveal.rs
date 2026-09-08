//! SQ-1138: the momentary word reveal, on the v6 RASTER surface.
//!
//! The reveal (SQ-1107) lights the words on screen the story's own parser really
//! knows. It was built against the CELL path, where the drawn text is terminal
//! cells and the light is a pass over them afterwards, and it was **dark on every
//! graphical v6 title** — which is where Arthur in extended mode lives, and where
//! the player who most wants the help is standing. Raster is a destination and not
//! a fallback, so that is a gap and not a compromise.
//!
//! Two halves broke, and this suite pins both:
//!
//!   1. **The reveal could not READ the screen.** `visible_text` asked the cell
//!      wrap cache, which the raster path never fills, so `arm` answered
//!      `Armed::NoText` — honestly, which is why this was a known gap rather than
//!      a mystery, but honestly is not the same as usefully.
//!   2. **And it could not WRITE to it.** A canvas has no cells to re-style, so
//!      the light has to be applied as each glyph is blitted; and the composite is
//!      behind a generation gate (SQ-0469) that a lit reveal moves nothing else
//!      past, so even a correct draw would never have been rebuilt.
//!
//! # What the light has to keep across the crossing
//!
//! `transcript_reveal` is parented on `accent` and **UNDERLINED**, and the rule is
//! load-bearing rather than decorative: this is host ink laid over the STORY's own
//! prose, and a foreground alone cannot promise legibility over whatever ground
//! the game painted under it. So the canvas draws both — the ink, and a rule in
//! the geometry SQ-1028 gives an emphasised run (the bottom of the TEXT cell, one
//! master row thick, spanning each lit glyph's whole advance so the letters join
//! into one line under the word).
//!
//! **The rule's thickness is stated in the TEXT cell and nowhere else.** On a
//! Macintosh colour press one art pixel is two native pixels while one text pixel
//! stays one (CLAUDE.md's density table, SQ-0917 / SQ-1039), so a thickness
//! resolved from `art_scale` would be double on exactly one press and correct on
//! every other — the hardest kind of wrong to notice. `the_rule_is_stated_in_the_
//! text_cell_not_the_art_scale` measures it against both cells the corpus has.
//!
//! # The specimens
//!
//! ```text
//!   fixture                    release  turns in  role
//!   zork0-r393-s890714.z6        393        6      a prose frame in the raster composite
//! ```
//!
//! Six taps is the frame `v6_extended_frame` measures for the same reason: it is
//! past the boot art and into a screen whose story window holds prose. The turn
//! count is part of the fixture (CLAUDE.md) — a frame is a fixture — and `boot`
//! prints the profile, release, screen size, art scale and v6 cell it resolved, so
//! a harness measuring a screen the app never draws says so out loud.
//!
//! **Both `honor_game_colours` modes are pinned throughout.** A highlight is a
//! colour, and Zork Zero boots `set_colour(fg=2, bg=9)` — so the ground the reveal
//! is read against is the game's in one mode and the host's in the other, which is
//! precisely the difference a single-mode suite would hide.
//!
//! Stories are gitignored (CLAUDE.md), so every real-game case skips cleanly; the
//! cases that measure `draw_story_text` against a bare canvas need no story and
//! run on CI.

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::v6_layout as v6;
use app::reveal::Armed;
use app::session::{GameSession, InputKind};

/// Zork Zero, at the release this suite is pinned to.
const ZORK0: &str = "zork0-r393-s890714.z6";
const ZORK0_RELEASE: u16 = 393;
/// Taps past the boot art, into a frame whose story window holds prose.
const TURNS: usize = 6;

/// A real kitty cell, so the composite is built the way a player's is.
const CELL: (u16, u16) = (10, 20);

fn stories_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Everything one boot settled, travelling together — the shape
/// `v6_extended_frame::Booted` uses, and for the same reason: a harness that
/// remembers the cell separately from the screen is how a Macintosh press gets
/// measured on a grid no Macintosh has (SQ-1020 / SQ-1021).
struct Booted {
    session: GameSession,
    art_scale: (u32, u32),
    face: app::native_font::TextFace,
    honoured: bool,
}

/// Boot the way `startup.rs` boots: the profile from the medium the MOUNT
/// returned, then the screen size through `MachineBoot` — which is what resolves
/// `picts.std_window() → named archive → picts.native_std_window() →
/// profile.std_window()` with `art_scale` alongside. Skipping any link measures a
/// screen the player never sees (SQ-0901).
fn boot() -> Option<Booted> {
    let path = stories_dir().join(ZORK0);
    let (bytes, medium) = match app::hints::load_mounted_story(&path) {
        Ok((loaded, medium)) => (loaded.bytes().to_vec(), medium),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            return None;
        }
    };
    let profile = InterpreterProfile::resolve(&path, None, None, medium);
    let mut picts = PictSource::resolve_with_override(&path, app::graphics::PictureOverride::Unset, None);
    let dims = picts.all_pict_dims();
    let release = u16::from_be_bytes([bytes[2], bytes[3]]);
    assert_eq!(
        release, ZORK0_RELEASE,
        "a disk image is a different BUILD, not the same story on other media — this suite is \
         pinned to release {ZORK0_RELEASE}"
    );
    let honoured = !picts.declines_game_colours(profile.default_colours());
    let faces = app::native_font::resolve(&app::native_font::FaceRequest {
        story_path: &path,
        entry: None,
        profile,
        source: app::interpreter::ProfileSource::Medium,
        art_scale: picts.art_scale(),
        disks: None,
    });
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        honoured.then(|| profile.default_colours()).flatten(),
        true,
        faces.clone(),
        profile.palette(),
        None,
    );
    let art_scale = boot.art_scale;
    let face = app::native_font::TextFace::new(profile, faces, art_scale);
    eprintln!(
        "{ZORK0}: booted as {profile:?} off {medium:?} · release {release} · screen {:?} · \
         art_scale {art_scale:?} · v6 cell {:?} · face scale {:?} · colours {}",
        boot.screen_px,
        face.cell(),
        face.scale(),
        if honoured { "honoured" } else { "declined" },
    );
    let mut session =
        GameSession::new_for_machine(bytes, honoured, false, false, dims, None, None, &boot)
            .unwrap_or_else(|e| panic!("{ZORK0}: should boot without a ZError: {e:?}"));
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..TURNS {
        let t = match session.pending_input() {
            InputKind::Line | InputKind::Event => session.submit("").transcript,
            InputKind::Char => session.submit_char(b' ').transcript,
        };
        if t.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
    }
    Some(Booted { session, art_scale: art_scale.unwrap_or((2, 2)), face, honoured })
}

/// A v6 app state in RASTER mode, at a real kitty cell, carrying the machine's own
/// cell and the archive's own art scale — the two facts a harness that forgets
/// them measures a fabricated screen without.
#[allow(deprecated)]
fn raster_state(b: &Booted, honor: bool, transcript: &[&str]) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    let mut picker =
        ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(CELL.0, CELL.1));
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    state.game_picker = Some(picker);
    state.config.v6_render = app::config::V6RenderMode::Raster;
    state.config.honor_game_colours = honor;
    state.config.guidance = true;
    state.v6_art_scale = b.art_scale;
    state.v6_text = b.face.clone();
    for line in transcript {
        state.push_transcript(line);
    }
    state
}

/// Build the raster composite exactly as `render_story_pane` does — and publish the
/// metrics it returns, which is the line production runs immediately afterwards
/// (`state.v6_raster_metrics.set(...)`) and which is how the reveal knows which
/// rows of the raster wrap are the ones on screen.
fn composite(
    session: &GameSession,
    state: &app::state::AppState,
) -> image::RgbaImage {
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &state.v6_text);
    let layout = v6::classify_windows(items, state.v6_text.cell());
    let (canvas, metrics) = app::render::screen::build_v6_raster_canvas(&layout, native, state);
    state.v6_raster_metrics.set(metrics);
    canvas
}

/// Pixels where two composites of the same frame differ.
fn differing(a: &image::RgbaImage, b: &image::RgbaImage) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for y in 0..a.height().min(b.height()) {
        for x in 0..a.width().min(b.width()) {
            if a.get_pixel(x, y) != b.get_pixel(x, y) {
                out.push((x, y));
            }
        }
    }
    out
}

/// A prose line planted in the transcript, so the words under test are ours rather
/// than whatever the game happened to print. The reveal draws the wrapped
/// transcript, so this is real story text as far as every layer below is concerned.
///
/// `table` is not decoration: at TURNS taps Zork Zero holds three objects in scope
/// and one of them answers to it, so the story's own object tree lights this word
/// and the case can assert `Lit` rather than merely "not `NoText`". The word was
/// found by asking the game (`objects_in_scope` reports 3; `table` and `flathead`
/// are the two of the probed nouns it answers to), not chosen for the sentence.
const PROSE: &str = "A heavy table stands here in the gloom.";
/// A line naming a DIFFERENT word the same frame's scope answers to, planted as
/// the oldest row of a long scrollback — so it is real and in scope and off the
/// screen, which is the only combination that can tell a viewport read from a
/// whole-cache read.
const SCROLLED_AWAY: &str = "The flathead crest is carved above the arch.";

fn lit(words: &[&str]) -> app::reveal::Reveal {
    // No `tier`: SQ-1135 landed in the same wave as this suite and removed
    // `RevealTier`, the reveal now annotating by vocabulary rather than by scope,
    // which leaves one tier and so no field. Neither lane could see the other's
    // change and both compiled alone — the merge was textually clean and did not
    // build, which is the case the full gate exists for.
    app::reveal::Reveal {
        words: words.iter().map(|w| w.to_string()).collect(),
        until: std::time::Instant::now() + app::reveal::REVEAL_HOLD,
    }
}

// ── 1. The reveal can READ the raster surface ────────────────────────────────

/// The words `arm` lit, whatever it lit them by.
fn armed_words(state: &app::state::AppState) -> Vec<String> {
    state.reveal.as_ref().map(|r| r.words.iter().cloned().collect()).unwrap_or_default()
}

/// **The reported defect.** `arm` on a raster frame answered `Armed::NoText` —
/// "there is no drawn text to read" — while a screenful of the story's own prose
/// was in front of the player. `visible_text` asked the CELL wrap cache, and the
/// raster path never fills it.
///
/// The claim asserted is `Lit` and `table` among the words: Zork Zero really does
/// hold something answering to `table` at this frame, so this is the story's own
/// answer about the player's own screen and not merely "the string was non-empty".
/// The TIER is deliberately not asserted — which words qualify, and by what test,
/// belongs to the word-selection half of the feature (SQ-1135) and not to this
/// one.
#[test]
fn a_raster_frame_is_legible_to_the_reveal() {
    let Some(b) = boot() else { return };
    for honor in [true, false] {
        let mut state = raster_state(&b, honor, &[PROSE]);
        let _ = composite(&b.session, &state);

        // Non-vacuity: the frame this case is about really did rasterise prose.
        let metrics = state.v6_raster_metrics.get().unwrap_or_else(|| {
            panic!("honor={honor}: the raster composite reported no story window — this is not a \
                    prose frame and the case would pass vacuously")
        });
        assert!(
            metrics.total_rows > 0,
            "honor={honor}: the raster wrap holds no rows: {metrics:?}"
        );

        let armed = app::reveal::arm(&mut state, &b.session);
        eprintln!("honor={honor}: {metrics:?} → {armed:?} {:?}", armed_words(&state));
        assert_ne!(
            armed,
            Armed::NoText,
            "honor={honor}: the raster composite drew {} rows of prose and the reveal reported \
             there was nothing to read — SQ-1138 exactly",
            metrics.total_rows,
        );
        assert!(
            matches!(armed, Armed::Lit { .. }),
            "honor={honor}: `table` is in this frame's scope, so the reveal must light: {armed:?}",
        );
        assert!(
            armed_words(&state).contains(&"table".to_string()),
            "honor={honor}: the word the story answers to is the one that must light: {:?}",
            armed_words(&state),
        );
    }
}

/// The reveal reads the rows that are ON SCREEN, not the whole scrollback — the
/// same present-tense window the cell path takes, out of the raster wrap instead.
///
/// The oldest row names `flathead`, which this frame's scope really does answer to;
/// the two hundred rows over it name `table`, which it also answers to. Both are in
/// the cache and only one is on screen, so a `visible_text` that returned the whole
/// cache rather than the viewport slice lights both — and the player is shown a
/// highlight for prose that scrolled away two hundred lines ago.
#[test]
fn the_reveal_reads_the_viewport_and_not_the_whole_scrollback() {
    let Some(b) = boot() else { return };
    let mut lines = vec![SCROLLED_AWAY];
    lines.extend(std::iter::repeat_n(PROSE, 200));
    let mut state = raster_state(&b, true, &lines);
    let _ = composite(&b.session, &state);
    let metrics = state.v6_raster_metrics.get().expect("a prose frame");
    let armed = app::reveal::arm(&mut state, &b.session);
    let words = armed_words(&state);
    eprintln!("{metrics:?} → {armed:?} {words:?}");
    // The shape this case depends on, asserted rather than assumed.
    assert!(
        metrics.total_rows as usize > metrics.viewport_rows as usize,
        "the scrollback must outrun the viewport or this case proves nothing: {metrics:?}",
    );
    assert!(
        metrics.first_visible_row > 0,
        "…and the visible slice must start past the oldest row: {metrics:?}",
    );
    assert!(
        words.contains(&"table".to_string()),
        "the word on screen must light: {words:?}",
    );
    assert!(
        !words.contains(&"flathead".to_string()),
        "`flathead` is in scope and {} rows off the top of the screen — lighting it means the \
         reveal read the whole cache instead of the viewport: {words:?}",
        metrics.first_visible_row,
    );
}

// ── 2. The light reaches the canvas ──────────────────────────────────────────

/// The lit words are drawn in the reveal's own ink and ruled under; the words
/// beside them are not touched at all.
///
/// Measured as a DIFF between two composites of the same frame — one with the
/// reveal lit, one without — so every pixel that moved is the reveal's and nothing
/// else's. A dark reveal produces an empty diff, which is the original symptom
/// stated as a number.
#[test]
fn the_lit_words_reach_the_composite_in_the_reveals_own_ink() {
    let Some(b) = boot() else { return };
    for honor in [true, false] {
        let dark = raster_state(&b, honor, &[PROSE]);
        let mut bright = raster_state(&b, honor, &[PROSE]);
        bright.reveal = Some(lit(&["table"]));

        let a = composite(&b.session, &dark);
        let c = composite(&b.session, &bright);
        let moved = differing(&a, &c);
        assert!(
            !moved.is_empty(),
            "honor={honor}: a lit reveal changed NOT ONE PIXEL of the raster composite — this is \
             SQ-1138 exactly: the words are dark on the surface the player is looking at",
        );

        // Every pixel that moved took the reveal's ink, and it is the theme's
        // `transcript_reveal` foreground rather than the story's own.
        let want = app::reveal::raster_reveal(&bright, image::Rgba([0, 0, 0, 0]))
            .expect("a lit reveal resolves an ink")
            .ink;
        for &(x, y) in &moved {
            assert_eq!(
                *c.get_pixel(x, y),
                want,
                "honor={honor}: ({x}, {y}) changed but is not the reveal's ink {want:?}",
            );
        }
        eprintln!("honor={honor}: {} px lit in {want:?}", moved.len());

        // …and it is ONE word's worth. `table` is five characters of a
        // thirty-eight-character line, so a reveal that lit the whole row (or the
        // whole screen) would show up here as a diff many times this size.
        let cell = b.face.cell();
        let row_px = b.face.run_px(PROSE);
        let word_px = b.face.run_px("table");
        let ceiling = (word_px + u32::from(cell.w())) * u32::from(cell.h());
        assert!(
            (moved.len() as u32) <= ceiling,
            "honor={honor}: {} px moved, which is more than the {word_px}x{} cell box one word \
             can occupy (the row is {row_px} px wide)",
            moved.len(),
            cell.h(),
        );
    }
}

/// **The underline is not decoration.** `transcript_reveal` is UNDERLINED because
/// a foreground alone cannot promise legibility over a ground the game chose, and
/// the raster surface has to keep that or the reveal is a colour swap that a
/// game-set page can swallow.
///
/// Pinned as geometry: the lit word's own rows must reach the BOTTOM row of its
/// text cell, which prose glyphs alone do not — no glyph in the face inks the
/// cell's last row (SQ-0932's sampling note), so ink there is the rule and can be
/// nothing else.
#[test]
fn a_lit_word_is_ruled_under_on_the_canvas() {
    let Some(b) = boot() else { return };
    for honor in [true, false] {
        let dark = raster_state(&b, honor, &[PROSE]);
        let mut bright = raster_state(&b, honor, &[PROSE]);
        bright.reveal = Some(lit(&["table"]));
        let a = composite(&b.session, &dark);
        let c = composite(&b.session, &bright);
        let moved = differing(&a, &c);
        assert!(!moved.is_empty(), "honor={honor}: nothing lit — SQ-1138");

        // The lit pixels occupy one text row; its bottom edge is the row's own
        // last scanline, and the rule is `(cell.h / 8).max(1)` deep into it.
        let cell_h = u32::from(b.face.cell().h());
        let bottom = moved.iter().map(|&(_, y)| y).max().expect("lit pixels");
        let top = moved.iter().map(|&(_, y)| y).min().expect("lit pixels");
        assert!(
            bottom - top < cell_h,
            "honor={honor}: the lit word spans {} rows of a {cell_h}-row cell — it must be one \
             text row",
            bottom - top + 1,
        );
        let rule_h = (cell_h / 8).max(1);
        let rule_rows: Vec<u32> = ((bottom + 1 - rule_h)..=bottom).collect();
        // A rule spans the whole advance of every lit glyph, so its rows are the
        // WIDEST of the lit word's rows — wider than any glyph's own ink, which is
        // what tells a rule from a letter with a flat foot.
        let width_at = |y: u32| moved.iter().filter(|&&(_, yy)| yy == y).count();
        let rule_w = rule_rows.iter().map(|&y| width_at(y)).min().unwrap_or(0);
        let glyph_w = (top..(bottom + 1 - rule_h)).map(width_at).max().unwrap_or(0);
        eprintln!(
            "honor={honor}: rule rows {rule_rows:?} are {rule_w} px wide; the widest glyph row is \
             {glyph_w} px",
        );
        assert!(
            rule_w > glyph_w,
            "honor={honor}: the bottom {rule_h} row(s) of the lit word must be an unbroken rule \
             spanning every glyph's advance, wider than any glyph row ({rule_w} vs {glyph_w})",
        );
    }
}

// ── 3. The density trap ──────────────────────────────────────────────────────

/// **The rule is stated in the TEXT cell and never in the art scale.**
///
/// On a Macintosh colour press one art pixel is two native pixels while one text
/// pixel stays one (CLAUDE.md's density table, SQ-0917 / SQ-1039), so a rule sized
/// from `art_scale` is double on exactly that press and correct on every other —
/// which is the hardest kind of wrong to notice, and the reason this case states
/// the answer over both cells the corpus has rather than over one.
///
/// Needs no story: it measures `draw_story_text` against a bare canvas.
#[test]
fn the_rule_is_stated_in_the_text_cell_not_the_art_scale() {
    for cell in [zvm::screen::V6Cell::new(8, 16), zvm::screen::V6Cell::new(7, 15)] {
        let face = app::native_font::TextFace::cell_only(cell);
        let ink = image::Rgba([255u8, 255, 255, 255]);
        let reveal_ink = image::Rgba([0u8, 255, 255, 255]);
        let main = v6::MainText {
            lines: vec!["lantern".into()],
            styles: Vec::new(),
            input: String::new(),
            cursor_col: 0,
            awaiting: false,
            floats: Vec::new(),
        };
        let words: std::collections::BTreeSet<String> = ["lantern".to_string()].into_iter().collect();
        let reveal = app::reveal::RasterReveal { words: &words, ink: reveal_ink, rule: true };
        let w = u32::from(cell.w()) * 8;
        let h = u32::from(cell.h()) * 2;
        let mut canvas = image::RgbaImage::new(w, h);
        v6::draw_story_text(&mut canvas, &main, 0, 0, 8, 2, ink, &[], &face, Some(&reveal));

        // The rule is one MASTER row — `cell.h / 8` — at the bottom of the cell:
        // two native rows on the 16-row cell, one on the Macintosh's fifteen.
        let want = (u32::from(cell.h()) / 8).max(1);
        let full: Vec<u32> = (0..u32::from(cell.h()))
            .filter(|&y| {
                (0..face.run_px("lantern")).all(|x| *canvas.get_pixel(x, y) == reveal_ink)
            })
            .collect();
        eprintln!("cell {cell:?}: unbroken reveal-ink rows across the word: {full:?}");
        assert_eq!(
            full.len() as u32,
            want,
            "cell {cell:?}: the rule must be {want} row(s) — one master row of THIS cell, not a \
             number scaled by the archive's art density",
        );
        assert_eq!(
            full.last().copied(),
            Some(u32::from(cell.h()) - 1),
            "cell {cell:?}: …and it sits on the cell's bottom row, where SQ-1028 puts an \
             emphasised run's",
        );
    }
}

/// An UNLIT row is drawn exactly as it always was — the reveal is a property of
/// the moment, and a `None` reveal must leave the canvas byte-for-byte alone.
///
/// The guard that a highlight applied unconditionally would fail, and it needs no
/// story either.
#[test]
fn no_reveal_leaves_the_canvas_untouched() {
    let cell = zvm::screen::V6Cell::new(8, 16);
    let face = app::native_font::TextFace::cell_only(cell);
    let ink = image::Rgba([255u8, 255, 255, 255]);
    let main = v6::MainText {
        lines: vec!["lantern".into()],
        styles: Vec::new(),
        input: String::new(),
        cursor_col: 0,
        awaiting: false,
        floats: Vec::new(),
    };
    let draw = |reveal: Option<&app::reveal::RasterReveal<'_>>| {
        let mut c = image::RgbaImage::new(u32::from(cell.w()) * 8, u32::from(cell.h()) * 2);
        v6::draw_story_text(&mut c, &main, 0, 0, 8, 2, ink, &[], &face, reveal);
        c
    };
    let plain = draw(None);
    // A lit reveal whose words are not on this row is the same thing from the other
    // side: `lit_spans` finds nothing, so nothing lights.
    let words: std::collections::BTreeSet<String> = ["sceptre".to_string()].into_iter().collect();
    let miss =
        app::reveal::RasterReveal { words: &words, ink: image::Rgba([0, 255, 255, 255]), rule: true };
    assert_eq!(
        plain.as_raw(),
        draw(Some(&miss)).as_raw(),
        "a reveal that lights no word on this row must not move a pixel of it",
    );
}

/// **The rule is read from the theme, not hard-coded** — the same
/// `transcript_reveal` modifier the cell path's `paint_row` patches onto its
/// cells, so a player who restyles the selector gets one reveal on both surfaces
/// rather than two that disagree.
///
/// Its DEFAULT is the load-bearing part and is asserted here too: the shipped
/// theme underlines, because a foreground alone cannot promise legibility over a
/// ground the game chose. That is why the registry sets the modifier — not a
/// reason a theme may not clear it.
#[test]
fn the_rule_follows_the_themes_own_underline() {
    let cell = zvm::screen::V6Cell::new(8, 16);
    let face = app::native_font::TextFace::cell_only(cell);
    let ink = image::Rgba([255u8, 255, 255, 255]);
    let reveal_ink = image::Rgba([0u8, 255, 255, 255]);
    let main = v6::MainText {
        lines: vec!["table".into()],
        styles: Vec::new(),
        input: String::new(),
        cursor_col: 0,
        awaiting: false,
        floats: Vec::new(),
    };
    let words: std::collections::BTreeSet<String> = ["table".to_string()].into_iter().collect();
    let draw = |rule: bool| {
        let r = app::reveal::RasterReveal { words: &words, ink: reveal_ink, rule };
        let mut c = image::RgbaImage::new(u32::from(cell.w()) * 8, u32::from(cell.h()) * 2);
        v6::draw_story_text(&mut c, &main, 0, 0, 8, 2, ink, &[], &face, Some(&r));
        c
    };
    let bottom = u32::from(cell.h()) - 1;
    let ruled_row = |c: &image::RgbaImage| {
        (0..face.run_px("table")).all(|x| *c.get_pixel(x, bottom) == reveal_ink)
    };
    assert!(ruled_row(&draw(true)), "rule=true draws an unbroken rule across the word");
    assert!(!ruled_row(&draw(false)), "rule=false draws none");
    // …and both still light the WORD, so clearing the modifier loses the rule and
    // nothing else.
    for r in [true, false] {
        let c = draw(r);
        assert!(
            (0..bottom).any(|y| (0..face.run_px("table")).any(|x| *c.get_pixel(x, y) == reveal_ink)),
            "rule={r}: the glyphs light either way",
        );
    }

    // The shipped default, from the registry the cell path reads: underlined.
    let colors = app::colors::ColorScheme::terminal_default();
    assert!(
        colors
            .theme
            .get("transcript_reveal")
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::UNDERLINED),
        "`transcript_reveal` ships UNDERLINED — the rule is the promise of legibility over a \
         ground the game chose, not a flourish",
    );
}
