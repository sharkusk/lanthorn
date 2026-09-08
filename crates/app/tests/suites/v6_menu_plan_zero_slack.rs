//! **A command menu is a fact about the FRAME; slack is a fact about the PANE.**
//! SQ-0830.
//!
//! `hybrid_bottom_plan` used to open with `if slack == 0 { return Letterbox }` —
//! taken before `menu_strip_below_story` was ever consulted. So on any pane whose
//! VERTICAL axis is the binding letterbox axis, Journey's command menu stopped
//! being recognised as a menu at all, and everything gated on `plan == Menu` went
//! out in one go:
//!
//!   * `flank_panels` came back empty, so [`menu_flank_panel`] never ran — no panel
//!     fill sampled from the art's own edge (SQ-0547), no vertical centring, and
//!     since SQ-0828 no aspect-correct dest box either;
//!   * the Menu plan's exclusion from `tiled_flanks` lapsed, so the picture column
//!     fell through `v6_border::recognize` to the side-border TILER — the very path
//!     SQ-0819 established is wrong for Journey, whose left flank is a picture
//!     seated in a panel rather than a border to extend;
//!   * `glyph_borders_only` flipped true.
//!
//! **The user's own terminal was one of these panes.** At 166x44 the v6 area is
//! 164x41 cells = 1312x738 device px on an 8x18 kitty cell, so
//! `s = min(1312/640, 738/400) = 1.845` exactly and the vertical slack is zero —
//! which is also why SQ-0824's integer snap was unavailable at precisely the size
//! the defect was reported at.
//!
//! The fix is the ordering, and nothing else: ask the frame first, and let slack
//! gate only the RECLAIM. A Menu plan at zero slack degrades to "menu, no reclaim"
//! for free, because the plan's menu scale is `off_y = slack` and `Letterbox`'s
//! centred scale is `off_y = slack / 2` — both the top-anchored scale when slack is
//! zero. No band moves; only the flank TREATMENT changes.
//!
//! Two halves, both load-bearing:
//!
//!   1. **Journey keeps its Menu plan at zero slack**, on BOTH releases — they are
//!      different builds with differently shaped menus, and one does not stand in
//!      for the other. `Journey - The Quest Begins.adf` is release 30 / serial
//!      890322 (Amiga); `journey-r83-s890706.z6` is release 83 / serial 890706
//!      (IBM PC). Its flanks are drawn as PANELS and not one of them is tiled.
//!   2. **Nothing else moved.** This is a plan-selection change with blast radius,
//!      so the three other v6 titles that reach `hybrid_bottom_plan` are pinned at
//!      the very same panes: Arthur reads no menu at any pane and keeps
//!      `letterbox`; Shogun and Zork Zero are enclosed frames whose story reaches
//!      the native bottom, so `menu_strip_below_story` is false before the hoisted
//!      question can matter, and they keep `letterbox` with their tilers intact.
//!
//! Swept in both `honor_game_colours` modes, per the project's colour-render
//! convention. `stories/` is gitignored, so every case skips vacuously when its
//! fixture is absent.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The kitty-ish font cell the sweep runs at. `Picker::halfblocks()` reports a 1x2
/// cell — a layout regime that reproduces no scale defect at all (the SQ-0548
/// lesson) — so this is a plausible real one, and it is the cell the user's own
/// 166x44 measurement was taken on.
const CELL: (u8, u8) = (8, 18);

/// Panes with **zero vertical letterbox slack**: the vertical axis binds, so
/// `slack == 0` and the old shortcut fired. On a 640x400 native screen at an 8x18
/// cell that is `w * 8 / 640 >= h * 18 / 400`, i.e. `w >= 3.6 · h`. Each case
/// asserts its own slack is zero rather than trusting this arithmetic, so a pane
/// cannot quietly drift into the other regime and pass vacuously.
///
/// **164x41 leads deliberately**: it is the user's own — the v6 area a 166x44
/// terminal leaves — so a falsification run reports the pane the defect was
/// reported at rather than whichever one happens to sort first.
const ZERO_SLACK_PANES: &[(u16, u16)] = &[(164, 41), (120, 30), (144, 40), (180, 50), (216, 60), (236, 65)];

/// The two Journey builds. Release 30 comes off the floppy and resolves to Amiga;
/// release 83 is the bare story file and resolves to the IBM PC.
const JOURNEYS: &[(&str, u16, &str)] =
    &[("Journey - The Quest Begins.adf", 30, "890322"), ("journey-r83-s890706.z6", 83, "890706")];

/// The other three v6 titles that reach `hybrid_bottom_plan`, and the plan each
/// must still get at a zero-slack pane. All three were measured at `letterbox`
/// before the hoist and must be `letterbox` after it: a plan change in a game that
/// is not menu-driven is a regression, not the fix working.
const UNMOVED: &[(&str, u16, &str, &str)] = &[
    ("Arthur - The Quest for Excalibur.adf", 54, "890606", "letterbox"),
    ("James Clavell's Shogun.adf", 295, "890321", "letterbox"),
    ("zork0-r393-s890714.z6", 393, "890714", "letterbox"),
];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot exactly as `startup.rs` does — the profile comes from the MEDIUM — after
/// checking the build is the one this suite claims to measure, then drive `turns`
/// blank turns to a gameplay frame. `None` (with a SKIP note) when the gitignored
/// fixture is absent.
fn boot(file: &str, release: u16, serial: &str, turns: usize) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let loaded = match app::hints::load_mounted_story(&path) {
        Ok((l, _)) => l,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            return None;
        }
    };
    let bytes = loaded.bytes().to_vec();
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]),
        release,
        "{file}: this suite's numbers were measured on release {release}"
    );
    let got: String = bytes[18..24].iter().map(|&b| b as char).collect();
    assert_eq!(got, serial, "{file}: this suite's numbers were measured on serial {serial}");

    let profile = InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut s = GameSession::new_with_trace(
        loaded.bytes().to_vec(),
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .unwrap_or_else(|e| panic!("{file}: should boot without a ZError: {e:?}"));
    // SQ-1393: the machine's own colour table. `new_with_trace` is the
    // no-machine door and presents §8.3.1's own, so a harness that boots a
    // press states the press's table here.
    s.machine.set_palette(profile.palette());
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..turns {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if r.transcript.contains("Praxix") || r.transcript.contains("magical resources") {
            break;
        }
    }
    Some(s)
}

/// How one side-flank band was drawn, read off the render's own band log rather
/// than by re-implementing the pipeline. `stretched` under a Menu plan is
/// [`menu_flank_panel`]'s dest, and since SQ-0898 removed the Frame plan's own
/// stretch arm it is the ONLY way a flank is drawn stretched at all; `tiled` is the
/// side-border tiler this quest is about keeping away from a picture panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Draw {
    Panel,
    Tiled,
    Plain,
}

/// Render one frame at `pane` and report the plan it picked plus how each SIDE
/// flank band (narrower than the pane) was drawn.
#[allow(deprecated)]
fn frame(model: &app::engine::ScreenModel, honor: bool, pane: (u16, u16)) -> (String, Vec<Draw>) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(CELL.0 as u16, CELL.1 as u16)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, pane.0, pane.1);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    let plan = state.v6_ring_plan.get().to_string();
    let flanks = state
        .graphics_render
        .borrow()
        .band_log
        .iter()
        .filter_map(|line| {
            let rest = line.strip_prefix("band ")?;
            let (dims, rest) = rest.split_once('@')?;
            let w: u16 = dims.split_once('x')?.0.trim().parse().ok()?;
            (w < pane.0).then(|| {
                if rest.contains("tiled]") {
                    Draw::Tiled
                } else if rest.contains("stretched]") {
                    Draw::Panel
                } else {
                    Draw::Plain
                }
            })
        })
        .collect();
    (plan, flanks)
}

/// The vertical letterbox slack this pane leaves, in device pixels — the very
/// quantity `render_story_pane` computes and hands to `hybrid_bottom_plan`.
fn slack_of(native: (u16, u16), pane: (u16, u16)) -> u32 {
    let dev = (pane.0 as u32 * CELL.0 as u32, pane.1 as u32 * CELL.1 as u32);
    let s = (dev.0 as f32 / native.0 as f32).min(dev.1 as f32 / native.1 as f32);
    dev.1.saturating_sub((native.1 as f32 * s).round() as u32)
}

fn native_of(model: &app::engine::ScreenModel) -> (u16, u16) {
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT))
}

// ── 1. Journey keeps its menu — and its panels — with no slack to reclaim ────

/// At every zero-slack pane, on both releases and in both colour modes: the plan is
/// `menu`, at least one side flank is drawn, and **not one of them is tiled**.
///
/// FALSIFY by restoring the `if slack == 0 { return BottomPlan::Letterbox; }`
/// shortcut to the top of `hybrid_bottom_plan` in `render/screen.rs`: release 30
/// fails at the user's own pane with
///
/// ```text
/// Journey - The Quest Begins.adf (r30) 164x41 honor=true: the command menu is a
/// property of the FRAME, so the plan is `menu` at any pane aspect — slack (0 device
/// px) gates the reclaim, not the classification. Got `letterbox`
/// ```
#[test]
fn journeys_menu_survives_a_pane_with_no_vertical_slack() {
    for (file, release, serial) in JOURNEYS {
        let Some(session) = boot(file, *release, serial, 40) else { return };
        let model = session.screen();
        let native = native_of(&model);

        for honor in [true, false] {
            for &pane in ZERO_SLACK_PANES {
                let slack = slack_of(native, pane);
                let (w, h) = pane;
                assert_eq!(
                    slack, 0,
                    "{file} (r{release}) {w}x{h}: this sweep is the ZERO-SLACK regime — a pane with slack to \
                     reclaim never took the shortcut and proves nothing here"
                );

                let (plan, flanks) = frame(&model, honor, pane);
                assert_eq!(
                    plan, "menu",
                    "{file} (r{release}) {w}x{h} honor={honor}: the command menu is a property of the FRAME, so \
                     the plan is `menu` at any pane aspect — slack ({slack} device px) gates the reclaim, not the \
                     classification. Got `{plan}`"
                );
                assert!(
                    !flanks.is_empty(),
                    "{file} (r{release}) {w}x{h} honor={honor}: Journey's picture column is a side flank band, so \
                     the frame must draw at least one"
                );
                assert!(
                    !flanks.contains(&Draw::Tiled),
                    "{file} (r{release}) {w}x{h} honor={honor}: Journey's left column is a picture SEATED IN A \
                     PANEL, not a border to extend — the side-border tiler is SQ-0819's wrong path for it and the \
                     Menu plan is what excludes it. Flanks drawn as {flanks:?}"
                );
                assert!(
                    flanks.contains(&Draw::Panel),
                    "{file} (r{release}) {w}x{h} honor={honor}: a Menu-plan flank goes to `menu_flank_panel`'s \
                     dest — the panel fill sampled from the art's own edge, the picture centred in it (SQ-0547) \
                     and its aspect kept (SQ-0828). Flanks drawn as {flanks:?}"
                );
            }
        }
    }
}

// ── 2. Nothing else moved ───────────────────────────────────────────────────

/// Arthur, Shogun and Zork Zero reach the same function and must be untouched by
/// the hoist: none of them prints a text strip below its story window, so the
/// question now asked first is answered `false` for all three and their zero-slack
/// panes stay `letterbox` exactly as they were.
///
/// This is the blast-radius pin. A plan change in a game that is NOT menu-driven is
/// a regression, and it would land here before it landed on a user's screen.
#[test]
fn the_other_v6_titles_keep_their_plans_at_the_same_panes() {
    for (file, release, serial, want) in UNMOVED {
        let Some(session) = boot(file, *release, serial, 12) else { return };
        let model = session.screen();
        let native = native_of(&model);

        for honor in [true, false] {
            for &pane in ZERO_SLACK_PANES {
                let (w, h) = pane;
                assert_eq!(
                    slack_of(native, pane),
                    0,
                    "{file} (r{release}) {w}x{h}: this sweep is the ZERO-SLACK regime"
                );
                let (plan, flanks) = frame(&model, honor, pane);
                assert_eq!(
                    plan, *want,
                    "{file} (r{release}) {w}x{h} honor={honor}: this title prints no menu strip below its story \
                     window, so hoisting the Menu question above the slack shortcut cannot reach it. Got `{plan}` \
                     with flanks {flanks:?}"
                );
            }
        }
    }
}
