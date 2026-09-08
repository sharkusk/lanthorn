//! **The letterbox margin is not the game's to paint.** SQ-0946.
//!
//! `v6_pixel_lock` (SQ-0936) floors the letterbox magnification to a rung of the
//! artwork's own ladder, so the game's screen usually stops SHORT of the pane and a
//! horizontal margin opens on either side of it. `uniform_scale`'s `centred` splits
//! that slack evenly and the ART lands centred to the device pixel — measured
//! symmetric to the cell on `Journey - The Quest Begins.adf` at every pane width from
//! 80 to 160 columns, lock on, before anything here was changed.
//!
//! The GROUND around the art was not. Two of the hybrid ring's cell fills ran to the
//! BAND's edge, and a band runs to the pane's:
//!
//!   * `draw_chrome_text_strip`'s strip flood and its per-row bar flood, which is
//!     Journey's bottom command strip; and
//!   * `menu_flank_panel`'s panel fill, whose `(lo, hi)` is bounded by the flank's two
//!     BORDER columns and falls back to the band when a border is not found — which
//!     is exactly `journey-r83-s890706.z6`, whose outer rule is a reverse-video SPACE
//!     that the border probe does not return.
//!
//! Measured on release 83 at a 98x37 pane, kitty cell 8x18, lock on (`s = 1.0`,
//! `off_x = 72`, so the game's screen covers columns 9..89): **333 painted cells in
//! the left margin against 54 in the right** — nine columns of the picture panel's own
//! dark ground down the left of the pane, and bare backdrop down the right. The frame
//! read as shoved left. That is the user's *"Journey's art is not centred
//! horizontally"*, and the art was never the thing that moved.
//!
//! The oracle is margin AGREEMENT rather than "the margin is empty", because an empty
//! margin is not the rule: a game that sets its story page floods the WHOLE pane with
//! it deliberately (SQ-0532 wave-5), letterbox included — Zork Zero's white and
//! Journey r30's grey both do. What may never happen is one margin carrying a ground
//! the other does not, and that single sentence is true of a page flood, of a bare
//! backdrop, and of nothing this quest found.
//!
//! Both `honor_game_colours` modes, per the project's colour-render convention: the
//! panel colour that spilled is only resolved when colours are honoured, and the
//! reverse-video menu ground spilled in BOTH — release 83 measured 333/54 either way.
//!
//! Specimens (release and turn count are part of the fixture — CLAUDE.md):
//!
//! ```text
//!   fixture                                release  turns  plan       role
//!   journey-r83-s890706.z6                    83      40    menu       the report
//!   Journey - The Quest Begins.adf            30      40    menu       the other build
//!   zork0-r393-s890714.z6                    393       6    frame      control
//!   arthur-r74-s890714.z6                     74      12    frame      control
//!   shogun-r322-s890706.z6                   322       2    frame      control
//! ```
//!
//! The controls are not decoration. A fix that centres Journey by moving Zork Zero or
//! Arthur is not a fix, and all three were already margin-symmetric before it.
//!
//! Arthur's plan reads `frame` and used to read `extend`, and the change is the guard
//! working rather than drift: at twelve taps the harness has fed the parser blank
//! lines, and Arthur answers those in the boxed window 3 across the bottom of the
//! screen — *"I beg your pardon?"*. That box arrives by `print_form`, which was a
//! no-op stub until SQ-1006, so the frame genuinely had no bottom strip before and
//! genuinely has one now. The margins agree either way, which is what this case is
//! about.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::v6_layout as v6;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// One fixture: file, the key answered to a character read while tapping in, how many
/// taps reach the frame this case is about, and the release it must be holding.
struct Specimen {
    file: &'static str,
    keys: u8,
    taps: usize,
    release: u16,
    /// Does this frame's ring PAINT ground with cells — a text strip, a bottom menu
    /// strip, or a flank panel? Only such a fill can spill into the letterbox, so only
    /// these frames get the shape guard. Zork Zero's is art the whole way round at the
    /// frame this case drives, and is a control whose margins agree for a different
    /// reason and must keep agreeing.
    fills: bool,
    /// The ring plan this frame lays out under, from the specimen table in the module
    /// doc — asserted, so the table is a measurement rather than a caption, and so a
    /// frame that drifts onto another plan cannot quietly pass as this one.
    plan: &'static str,
}

const CORPUS: &[Specimen] = &[
    Specimen { file: "journey-r83-s890706.z6", keys: 13, taps: 40, release: 83, fills: true, plan: "menu" },
    Specimen { file: "Journey - The Quest Begins.adf", keys: 13, taps: 40, release: 30, fills: true, plan: "menu" },
    Specimen { file: "zork0-r393-s890714.z6", keys: 13, taps: 6, release: 393, fills: false, plan: "frame" },
    Specimen { file: "arthur-r74-s890714.z6", keys: b'n', taps: 12, release: 74, fills: true, plan: "frame" },
    Specimen { file: "shogun-r322-s890706.z6", keys: 13, taps: 2, release: 322, fills: true, plan: "frame" },
];

/// Panes swept. Every one of them leaves the locked fit a horizontal margin at the
/// 8x18 kitty cell these cases render at (the case asserts that, so the sweep cannot
/// drift into a width-bound fit where there is nothing to be off-centre about).
const PANES: [(u16, u16); 4] = [(98, 37), (110, 37), (115, 45), (131, 41)];

/// The terminal cell the sweep renders at. `Picker::halfblocks()` reports a 1x2 cell,
/// a regime with no sub-cell boundary for anything here to fall on.
const CELL: (u16, u16) = (8, 18);

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a v6 story the way `startup.rs` does — the profile from the medium the MOUNT
/// returned, and the screen size through the whole `picts.std_window() →
/// native_std_window → profile.std_window()` chain with `art_scale` beside it — then
/// tap in to the frame. `None` (with a SKIP note) when the gitignored fixture is absent.
///
/// Skipping `native_std_window` is what booted two 560x384 presses at 640x400 and
/// fabricated a frame a whole quest was fixed against (CLAUDE.md); the art scale is
/// the other half of the same chain here, because it is what the magnification ladder
/// is DERIVED from and a wrong one silently changes every rung.
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

/// A hybrid render at a real kitty cell, with `v6_pixel_lock` as asked and the art
/// scale the mount resolved — the pair the ladder is derived from.
#[allow(deprecated)]
fn render(
    model: &app::engine::ScreenModel,
    transcript: &str,
    art_scale: (u32, u32),
    honor: bool,
    lock: bool,
    pane: (u16, u16),
) -> (app::state::AppState, Rect, Buffer) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    let mut picker = ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(CELL.0, CELL.1));
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    state.game_picker = Some(picker);
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
    (state, area, buf)
}

/// The distinct cell backgrounds standing in `cols` of `buf`, as a sorted set of
/// debug strings — the GROUND a margin carries, with the game's own text ignored
/// because a margin has none to ignore.
fn grounds(buf: &Buffer, rows: std::ops::Range<u16>, cols: std::ops::Range<u16>) -> Vec<String> {
    let mut v: Vec<String> = rows
        .flat_map(|y| cols.clone().map(move |x| (x, y)))
        .filter_map(|(x, y)| buf.cell((x, y)).map(|c| format!("{:?}", c.bg)))
        .collect();
    v.sort();
    v.dedup();
    v
}

// ── The rule ───────────────────────────────────────────────────────────────────

/// With the pixel lock on, the letterbox margins on either side of the game's screen
/// carry the same ground — for every v6 title in the corpus, at every pane in the
/// sweep, in both colour modes.
///
/// FALSIFY by restoring either fill's old bound in `render/screen.rs` — the
/// `rect.x..rect.right()` loops in `draw_chrome_text_strip`, or the unclamped `fill`
/// out of `menu_flank_panel` at the `flank_panels` call site. `journey-r83-s890706.z6`
/// then fails at 98x37 with the left margin carrying `Rgb(34, 34, 34)` (the picture
/// panel's own ground) and `Rgb(0, 0, 0)` where the right carries only `Reset` — 333
/// painted cells against 54.
#[test]
fn the_locked_letterbox_margins_carry_the_same_ground_on_both_sides() {
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
            let pane_dev = (pw as u32 * CELL.0 as u32, ph as u32 * CELL.1 as u32);
            let (scale, fell_back) = app::render::v6_layout::FrameGeometry::new(native, art_scale, zvm::screen::V6Cell::DEFAULT).fitted_scale(pane_dev, true);
            assert!(
                !fell_back,
                "{} r{} {pw}x{ph}: this sweep is about the LOCKED fit; a pane too small for the \
                 lowest rung free-scales and has nothing to say here",
                spec.file, spec.release
            );
            let pane = Rect::new(0, 0, pw, ph);
            let (lo, hi) = v6::screen_cols(&scale, native.0, CELL, pane);
            // Non-vacuity, half one: there IS a horizontal letterbox to be off-centre
            // in. Without this the whole case passes on a width-bound fit where the
            // margins are both empty by construction.
            assert!(
                lo > pane.x && hi < pane.right(),
                "{} r{} {pw}x{ph}: the locked fit must leave a margin on both sides to be \
                 measuring anything — s={:.4} off_x={} gave columns {lo}..{hi} of {pw}",
                spec.file, spec.release, scale.s, scale.off_x
            );

            for honor in [true, false] {
                let (state, area, buf) = render(&model, &transcript, art_scale, honor, true, (pw, ph));
                // Non-vacuity, half two: this is a RING frame with a story viewport —
                // not a boot prompt, not a raster takeover, not the cell path.
                assert_eq!(
                    state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).as_deref(),
                    Some("hybrid-ring"),
                    "{} r{} {pw}x{ph} honor={honor}: this case measures the hybrid RING",
                    spec.file, spec.release
                );
                assert_eq!(
                    state.v6_ring_plan.get(), spec.plan,
                    "{} r{} {pw}x{ph} honor={honor}: the specimen table names this frame's ring \
                     plan, and a frame on another plan is another frame",
                    spec.file, spec.release
                );
                let vp = state
                    .v6_cell_map
                    .borrow()
                    .iter()
                    .find(|e| e.label == "viewport")
                    .map(|e| e.cells)
                    .unwrap_or_else(|| panic!("{}: a ring frame records its story viewport", spec.file));
                assert!(
                    vp.2 > 0 && vp.3 > 0,
                    "{} r{} {pw}x{ph} honor={honor}: a degenerate viewport means the frame this \
                     case describes is not on screen. Got {vp:?}",
                    spec.file, spec.release
                );
                // Non-vacuity, half three: the frame really has the SHAPE the defect
                // lives in — a cell fill that runs the width of a band. Without this a
                // frame that quietly stopped drawing its menu strip would pass with two
                // empty margins and say nothing at all.
                if spec.fills {
                    let map = state.v6_cell_map.borrow();
                    let has_fill = map.iter().any(|e| {
                        e.label.starts_with("strip:text")
                            || e.label.starts_with("menu:text")
                            || e.label == "flank-panel"
                    });
                    assert!(
                        has_fill,
                        "{} r{} {pw}x{ph} honor={honor}: this frame is supposed to carry a cell \
                         fill — a text strip, a menu strip or a flank panel — and carries none, so \
                         agreeing margins prove nothing. Records: {:?}",
                        spec.file,
                        spec.release,
                        map.iter().map(|e| e.label.as_str()).collect::<Vec<_>>()
                    );
                }

                let left = grounds(&buf, area.y..area.bottom(), area.x..lo);
                let right = grounds(&buf, area.y..area.bottom(), hi..area.right());
                assert_eq!(
                    left, right,
                    "{} r{} {pw}x{ph} honor={honor}: the game's screen covers columns {lo}..{hi} \
                     of {pw}, and the letterbox margins either side of it must carry the same \
                     ground — one of them holding a colour the other does not is the frame sitting \
                     off-centre. s={:.4} off_x={}",
                    spec.file, spec.release, scale.s, scale.off_x
                );
                seen += 1;
            }
        }
    }
    assert!(
        !any_present || seen > 0,
        "the corpus is on disk but no frame was measured — a vacuous pass"
    );
}

/// …and turning the lock ON never moves the game's screen off centre: the slack it
/// opens is split evenly, so the margin the art leaves on the left is the margin it
/// leaves on the right, to within the one cell a half-cell offset cannot split.
///
/// This is the ART half of the report, asserted separately from the GROUND half above
/// because they are different mechanisms and were in different states: the art was
/// centred throughout and the ground was not, and a single case covering both could
/// not have said so.
#[test]
fn the_locked_fit_centres_the_game_screen_to_within_one_cell() {
    let mut seen = 0usize;
    let mut any_present = false;
    for spec in CORPUS {
        any_present |= stories_dir().join(spec.file).exists();
        let Some((mut session, art_scale)) = boot(spec) else { continue };
        let _ = session.take_transcript();
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else {
            panic!("{}: a v6 frame has a Layered root", spec.file)
        };
        let native = v6::native_extent(items.as_slice(), &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        // Every pane width a terminal plausibly has, not the four the ring sweep uses:
        // the offset is a function of the width alone and the sub-cell remainders only
        // show up across a run of them.
        for pw in 80u16..=160 {
            for ph in [37u16, 45] {
                let pane_dev = (pw as u32 * CELL.0 as u32, ph as u32 * CELL.1 as u32);
                let (scale, fell_back) = app::render::v6_layout::FrameGeometry::new(native, art_scale, zvm::screen::V6Cell::DEFAULT).fitted_scale(pane_dev, true);
                if fell_back {
                    continue;
                }
                let art_w = (native.0 as f32 * scale.s).round() as u32;
                let left = scale.off_x;
                let right = pane_dev.0.saturating_sub(scale.off_x + art_w);
                assert!(
                    left.abs_diff(right) <= 1,
                    "{} r{} {pw}x{ph}: the locked letterbox must be centred — {left} device px of \
                     margin on the left against {right} on the right (s={:.4}, art {art_w} of {} \
                     device px)",
                    spec.file, spec.release, scale.s, pane_dev.0
                );
                seen += 1;
            }
        }
    }
    assert!(!any_present || seen > 0, "the corpus is on disk but nothing was measured");
}
