//! **The byte-identity oracle for the v6 render modes (SQ-1032).**
//!
//! A deterministic digest of what `hybrid` and `raster` actually draw, so a change
//! to the v6 render path can be shown not to have moved either of them. It exists
//! because that property is pinned by NO suite: the cell-buffer harnesses assert
//! specific facts about specific frames, and a change can slip between all of them
//! while still moving pixels the player sees.
//!
//! ```sh
//! cargo run -p lanthorn --example v6_mode_digest > /tmp/after.txt
//! git stash push            # or: git checkout <base> -- crates/
//! cargo run -p lanthorn --example v6_mode_digest > /tmp/before.txt
//! git stash pop
//! diff /tmp/before.txt /tmp/after.txt        # must be empty
//! ```
//!
//! It is an EXAMPLE and not a test on purpose: its whole use is to be run against
//! two revisions of the tree and diffed, which a test in a group binary cannot do.
//! It reads only APIs that both sides of a change are likely to share — the public
//! `build_v6_raster_canvas`, `render_story_pane`, and `V6ClickMap::map_click` — and
//! touches no field of `V6ClickMap`, so it survives a refactor of that struct.
//!
//! Three digests per fixture:
//!
//!   * the RASTER composite's raw pixels, which is the whole picture;
//!   * the HYBRID pane's cells at four sizes — symbol, colours and modifier;
//!   * and the CLICK MAP the hybrid frame published, swept over every cell of the
//!     pane. `map_click` is the user-visible half of `V6ClickMap`, so hashing its
//!     answers pins mouse routing without pinning the struct's shape.
//!
//! **Why kitty placeholder cells are normalised.** A kitty unicode-placeholder cell
//! encodes the terminal-side IMAGE ID in its symbol and its foreground colour, and
//! that id is allocated per run. Hashing it makes the digest differ between two runs
//! of the SAME build, which is exactly the instability that makes a correct tool
//! look broken and get abandoned. Every such cell is folded to one token; its
//! POSITION still counts, which is what a placement change moves.
//!
//! **`pty_capture` is not an equality oracle** — do not reach for it as one. It runs
//! the real binary under a pty and is the right tool for LOOKING at a frame, but its
//! PNGs are timing-dependent on some titles: measured over this same corpus,
//! `arthur-r74-s890714.z6` in hybrid and `shogun-r322-s890706.z6` in raster each
//! produced two different images from two runs of one unchanged build, while
//! `zork0-r393-s890714.z6` and `journey-r83-s890706.z6` were stable in both modes.
//! This digest is stable on all six.
//!
//! Requires the gitignored `stories/` fixtures; absent ones are reported and skipped.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::v6_layout as v6;
use app::session::{GameSession, InputKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const CELL: (u16, u16) = (8, 18);
const PANES: [(u16, u16); 4] = [(117, 64), (100, 50), (80, 23), (160, 42)];

/// FNV-1a, so the digest needs no dependency and is stable across runs and machines.
fn fnv(bytes: &[u8], seed: u64) -> u64 {
    let mut h = seed;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

#[allow(deprecated)]
fn state_for(mode: app::config::V6RenderMode, transcript: &str, art_scale: (u32, u32)) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    let mut picker = ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(CELL.0, CELL.1));
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    state.game_picker = Some(picker);
    state.config.v6_render = mode;
    state.config.honor_game_colours = true;
    state.v6_art_scale = art_scale;
    for line in transcript.lines() {
        state.push_transcript(line);
    }
    state
}

fn main() {
    let _g = app::v6_palette_at_boot();
    let corpus: &[(&str, u8, usize)] = &[
        ("zork0-r393-s890714.z6", 13, 6),
        ("arthur-r74-s890714.z6", b'n', 12),
        ("journey-r83-s890706.z6", 13, 40),
        ("shogun-r322-s890706.z6", 13, 2),
        ("Journey - The Quest Begins.adf", 13, 40),
        ("Arthur - The Quest for Excalibur.adf", b'n', 12),
    ];
    for (file, key, taps) in corpus {
        let path = stories_dir().join(file);
        let Ok((loaded, medium)) = app::hints::load_mounted_story(&path) else {
            println!("{file}: SKIP (absent)");
            continue;
        };
        let bytes = loaded.bytes().to_vec();
        let profile = InterpreterProfile::resolve(&path, None, None, medium);
        app::v6_set_palette(profile.palette());
        let mut picts = PictSource::resolve(&path, None);
        let dims = picts.all_pict_dims();
        let release = u16::from_be_bytes([bytes[2], bytes[3]]);
        let boot = app::machine_boot::MachineBoot::resolve(
            profile,
            &picts,
            None,
            profile.interpreter_number(),
            profile.default_colours(),
            true,
            app::native_font::FaceSet::none(),
        );
        let art_scale = boot.art_scale.unwrap_or((2, 2));
        let mut session = match GameSession::new_for_machine(bytes, true, false, false, dims, None, None, &boot) {
            Ok(s) => s,
            Err(e) => {
                println!("{file}: BOOT FAILED {e:?}");
                continue;
            }
        };
        session.set_pict_source(Some(picts));
        session.flush_boot_pictures();
        let _ = session.take_transcript();
        for _ in 0..*taps {
            let t = match session.pending_input() {
                InputKind::Line => session.submit("").transcript,
                InputKind::Char => session.submit_char(*key).transcript,
                InputKind::Event => session.submit("").transcript,
            };
            if t.to_lowercase().contains("y or n") {
                let _ = session.submit_char(b'n');
            }
        }
        let transcript = session.take_transcript();
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else {
            println!("{file}: not a v6 frame");
            continue;
        };

        // The RASTER composite: the canvas's own pixels, which is the whole picture.
        let rstate = state_for(app::config::V6RenderMode::Raster, &transcript, art_scale);
        let native = v6::native_extent(items, &rstate.v6_text);
        let layout = v6::classify_windows(items, rstate.v6_text.cell());
        let (canvas, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &rstate);
        let raster = fnv(canvas.as_raw(), 0xcbf2_9ce4_8422_2325);
        println!(
            "{file} r{release} native {native:?} art_scale {art_scale:?} :: RASTER canvas {}x{} digest {raster:016x}",
            canvas.width(),
            canvas.height()
        );

        // HYBRID: the pane's cells, and the click map, at four sizes.
        for pane in PANES {
            let hstate = state_for(app::config::V6RenderMode::Hybrid, &transcript, art_scale);
            let area = Rect::new(0, 0, pane.0, pane.1);
            let mut buf = Buffer::empty(area);
            let m = app::render::screen::render_story_pane(&model, false, None, &hstate, area, &mut buf);
            let mut h = 0xcbf2_9ce4_8422_2325u64;
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    if let Some(c) = buf.cell((x, y)) {
                        // See the module doc: a kitty placeholder's symbol and fg
                        // carry a per-run image id, so only its POSITION is hashed.
                        if c.symbol().chars().any(|ch| ch as u32 >= 0x0300) {
                            h = fnv(b"IMG", h);
                            continue;
                        }
                        h = fnv(c.symbol().as_bytes(), h);
                        h = fnv(format!("{:?}|{:?}|{:?}", c.fg, c.bg, c.modifier).as_bytes(), h);
                    }
                }
            }
            let mut ch = 0xcbf2_9ce4_8422_2325u64;
            if let Some(cm) = hstate.graphics_render.borrow().last_v6_map.clone() {
                for y in area.y..area.bottom() {
                    for x in area.x..area.right() {
                        ch = fnv(format!("{:?}", cm.map_click(x, y)).as_bytes(), ch);
                    }
                }
            }
            println!(
                "{file} r{release} :: HYBRID pane {pane:?} viewport {} max_scroll {} digest {h:016x} clicks {ch:016x}",
                m.viewport_rows, m.max_scroll
            );
        }
    }
}
