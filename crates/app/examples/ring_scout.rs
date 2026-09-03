//! Scout what the v6 hybrid ring is working from, for a real frame (SQ-0894/0892).
//!
//! It draws no pixels and negotiates no terminal: it boots the story headlessly,
//! takes the same `ScreenModel` the renderer gets, and runs the same layout
//! primitives. That makes it the cheapest of the three render-testing layers — reach
//! for `examples/pty_capture` when the model looks right and the screen does not.
//!
//! It boots through the same profile chain `startup.rs` does — the medium the mount
//! resolved picks the machine, which picks the palette, the interpreter number, the
//! standard window and the default colours (SQ-0901). It prints the profile and the
//! RELEASE it is holding on every line of output, because a disk image is a different
//! build and not the same story on other media (CLAUDE.md), and a disagreement
//! between this and a `/dump-windows` capture should be visible rather than deduced.
//!
//! What it reports:
//!
//! * the story viewport, by the declared window BOX and by what the art leaves
//!   CLEAR (SQ-0894 step (b)) — the two agreed on every corpus frame, and this is
//!   how that is re-measured rather than taken on trust;
//! * `--runs`, every chrome text run with its native origin, the sub-cell remainder
//!   of its mapped device column, and the column per-run rounding puts it in. This
//!   is the SQ-0892 view: a run is POSITIONED through the scale but ADVANCES one
//!   terminal column per character, and these are the numbers that argument turns
//!   on;
//! * `--bands`, the ring AS DRAWN: the strips the renderer classified, the band
//!   log the graphics layer wrote, and — the reason this exists — each band's
//!   MAGNIFICATION beside the frame's own letterbox scale (SQ-0898). One frame,
//!   one magnification: a column drawn in two pieces at two factors is a seam, and
//!   it is invisible in the rects. Do not read the band log's `native` field for
//!   this; on a crop it is a hash footprint carrying the area filter's halo, so
//!   below scale 1 it reads several pixels wide and neighbouring bands look like
//!   they overlap where they partition the canvas exactly;
//! * the GEOMETRIC ring, `pane − viewport`, which is the baseline the shipped
//!   content-carved ring is read against — not the ring itself.
//!
//! ```sh
//! cargo run -q -p lanthorn --example ring_scout -- --story stories/zork0-r393-s890714.z6
//! cargo run -q -p lanthorn --example ring_scout -- --all --size 100x40
//! cargo run -q -p lanthorn --example ring_scout -- --story "stories/James Clavell's Shogun.adf" --taps 1 --runs
//! cargo run -q -p lanthorn --example ring_scout -- --story stories/arthur-r74-s890714.z6 \
//!     --keys n --taps 12 --bands --size 70x19
//! cargo run -q -p lanthorn --example ring_scout -- --story stories/InfocomMasterpieces.img \
//!     --entry arthur --keys n
//! ```
//!
//! `--entry <n|name>` says WHICH story, on a volume holding several: a 1-based
//! position in the browser's list or enough of a name, matched exactly as
//! `lanthorn --story` matches it (SQ-1078). It took the stored name literally
//! until then, so reaching a game on a compilation disc meant mounting the disc
//! yourself to learn what it was called.
//!
//! `--keys` is the byte answered to a CHARACTER read while tapping through the
//! intro; `--all` takes each title's own from the corpus table unless this
//! overrides it. Arthur needs `n` for his "restore a saved position?" question.
//!
//! `--size` is the PANE in cells (default 98x37, what a 100x40 terminal leaves
//! after the app frame — the size §5's captures were measured at); `--cell` is the
//! terminal cell in pixels (default 8x18, what the capture harness negotiates).
//! `--taps N` presses exactly N keys, which is how a screen BETWEEN the splash and
//! gameplay is reached (Shogun's credits/menu is one tap in); `--no-tap` reports the
//! boot frame; `--turns N` then plays N turns, reporting the viewport at each.
//!
//! `--cmd <text>` is the LINE submitted by each `--turns` step, default `look`. A
//! frame is a fixture and some of them are only reachable by a specific command —
//! Zork Zero's full-screen map is `--turns 1 --cmd map`, and driving blank lines
//! reaches an intro card and often nothing else (CLAUDE.md).
//!
//! `--game-colours off` renders `--bands` with `honor_game_colours` off. The two modes
//! are separate baselines and always have been (CLAUDE.md), and the ring is not the
//! same picture in both: honouring the game's colours floods every window's PAGE, and
//! SQ-0883 lived entirely in the flooded one — the shipped default — while the other
//! mode was correct throughout. An instrument that can only see one of them will
//! report a frame as clean at the very moment it is broken for the user.

use app::engine::Engine;
use app::render::v6_layout as v6;
use app::session::{GameSession, InputKind};
use ratatui::layout::Rect;

/// The corpus `--all` sweeps: every v6 title in `stories/` that draws a ring,
/// with the keys needed to get past its intro to a frame worth measuring.
const CORPUS: &[(&str, &str)] = &[
    ("stories/zork0-r393-s890714.z6", ""),
    ("stories/arthur-r74-s890714.z6", "n"),
    ("stories/shogun-r322-s890706.z6", ""),
    ("stories/journey-r83-s890706.z6", ""),
    ("stories/mysterious01.z6", "n"),
    ("stories/fmvpoker.z6", ""),
    ("stories/scopa.z6", ""),
    ("stories/advent.z6", ""),
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // SQ-1020: which story on a compilation, as `HfsEntry::path` spells it. Without
    // one the volume's tiebreak chooses, and on the Masterpieces CD that is Zork
    // Zero — so every Macintosh Arthur measurement was of a different game.
    let mut entry: Option<String> = None;
    let mut story: Option<String> = None;
    let mut all = false;
    let mut pane_cells = (98u16, 37u16);
    let mut cell_px = (8u16, 18u16);
    let mut turns = 0usize;
    let mut no_tap = false;
    let mut honor_colours = true;
    let mut runs = false;
    let mut bands = false;
    let mut keys_override: Option<String> = None;
    let mut taps: Option<usize> = None;
    let mut cmd: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--story" => {
                story = args.get(i + 1).cloned();
                i += 1;
            }
            "--all" => all = true,
            "--keys" => {
                keys_override = args.get(i + 1).cloned();
                i += 1;
            }
            "--entry" => {
                entry = args.get(i + 1).cloned();
                i += 1;
            }
            "--no-tap" => no_tap = true,
            // SQ-1082: `--<noun> on|off`, the spelling every front-end uses. This
            // is a debug instrument rather than a front-end, but a third spelling
            // of one concept is exactly what that quest removed.
            "--game-colours" => {
                honor_colours = args.get(i + 1).map(String::as_str) != Some("off");
                i += 1;
            }
            "--runs" => runs = true,
            "--bands" => bands = true,
            "--taps" => {
                if let Some(v) = args.get(i + 1) {
                    taps = v.parse().ok();
                }
                i += 1;
            }
            "--cmd" => {
                cmd = args.get(i + 1).cloned();
                i += 1;
            }
            "--turns" => {
                if let Some(v) = args.get(i + 1) {
                    turns = v.parse().unwrap_or(0);
                }
                i += 1;
            }
            "--size" => {
                if let Some(v) = args.get(i + 1) {
                    pane_cells = parse_pair(v).unwrap_or(pane_cells);
                }
                i += 1;
            }
            "--cell" => {
                if let Some(v) = args.get(i + 1) {
                    cell_px = parse_pair(v).unwrap_or(cell_px);
                }
                i += 1;
            }
            other => eprintln!("ring_scout: ignoring `{other}`"),
        }
        i += 1;
    }

    let targets: Vec<(String, String)> = if all {
        CORPUS
            .iter()
            .map(|(s, k)| (s.to_string(), keys_override.clone().unwrap_or_else(|| k.to_string())))
            .collect()
    } else if let Some(s) = story {
        vec![(s, keys_override.clone().unwrap_or_default())]
    } else {
        eprintln!("ring_scout: pass --story <file> or --all");
        std::process::exit(2);
    };

    println!(
        "pane {}x{} cells, cell {}x{} px  →  {}x{} device px\n",
        pane_cells.0, pane_cells.1, cell_px.0, cell_px.1,
        pane_cells.0 as u32 * cell_px.0 as u32,
        pane_cells.1 as u32 * cell_px.1 as u32,
    );

    for (path, keys) in targets {
        println!("═══ {path}");
        match scout(&path, entry.as_deref(), &keys, pane_cells, cell_px, turns, no_tap, runs, bands, taps, honor_colours, cmd.as_deref()) {
            Ok(()) => {}
            Err(e) => println!("  SKIP: {e}\n"),
        }
    }
}

/// The cell rect the CONTENT leaves for story text — SQ-0894 step (b).
///
/// `v6_layout::story_viewport` used to be this, and SQ-0894 deleted it after
/// measuring the answer to be identical to the declared window box on every corpus
/// frame. Kept here, against the surviving native-space `story_clear_native`, so
/// the measurement can be re-run rather than taken on trust.
fn clear_viewport_cells(
    story: Option<&app::engine::PositionedWindow>,
    gfx: &image::RgbaImage,
    scale: &v6::Scale,
    pane_cells: (u16, u16),
    cell_px: (u16, u16),
) -> Rect {
    let Some((left, top, w, h)) = v6::story_clear_native(story, gfx) else {
        return Rect { x: 0, y: 0, width: pane_cells.0, height: pane_cells.1 };
    };
    let (cw, ch) = (cell_px.0.max(1) as f32, cell_px.1.max(1) as f32);
    let dev = |v: u32, off: u32, per: f32| (off as f32 + v as f32 * scale.s) / per;
    let cell_left = dev(left, scale.off_x, cw).ceil() as u16;
    let cell_top = dev(top, scale.off_y, ch).ceil() as u16;
    let cell_right = dev(left + w, scale.off_x, cw).floor() as u16;
    let cell_bottom = dev(top + h, scale.off_y, ch).floor() as u16;
    let width = cell_right.saturating_sub(cell_left).max(1).min(pane_cells.0.saturating_sub(cell_left));
    let height = cell_bottom.saturating_sub(cell_top).max(1).min(pane_cells.1.saturating_sub(cell_top));
    Rect { x: cell_left, y: cell_top, width, height }
}

fn parse_pair(s: &str) -> Option<(u16, u16)> {
    let (a, b) = s.split_once(['x', 'X'])?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

#[allow(clippy::too_many_arguments)]
fn scout(
    path: &str,
    entry: Option<&str>,
    keys: &str,
    pane_cells: (u16, u16),
    cell_px: (u16, u16),
    turns: usize,
    no_tap: bool,
    want_runs: bool,
    want_bands: bool,
    taps: Option<usize>,
    honor_colours: bool,
    cmd: Option<&str>,
) -> Result<(), String> {
    // Disk images (.adf/.po/.2mg/.dsk) are mounted, not read — a medium carries a
    // different RELEASE, not the same story on other media (CLAUDE.md). The mount's
    // second answer is the medium THIS story came off, which on a hybrid disc is not
    // the image's own format (SQ-0876) — so it is what the profile resolves from.
    let p = std::path::Path::new(path);
    // `--entry` is a 1-based position or enough of a name to pick out one story —
    // the rule `lanthorn --story` and `zvm-cli --story` both match by (SQ-1078).
    // It used to be the stored name LITERALLY, so measuring anything on a
    // compilation disc meant mounting the disc yourself first to learn it.
    let entry = match entry.map(|w| app::story_pick::entry_on(p, w)) {
        Some(Ok(e)) => e,
        Some(Err(msg)) => return Err(msg),
        None => None,
    };
    let entry = entry.as_deref();
    let (bytes, disk_image) = match app::hints::load_mounted_story_from(p, entry) {
        Ok((loaded, medium)) => (loaded.bytes().to_vec(), medium),
        Err(e) => return Err(format!("{path}: {e:?}")),
    };
    if bytes.first() != Some(&6) {
        return Err("not a v6 story".into());
    }
    // SQ-0901: boot the way `startup.rs` does. Passing `interpreter_number: None`
    // and skipping `set_palette` measured a frame the renderer never draws — a
    // 172-native-px flank on the Amiga Arthur where the app renders a 30px pole —
    // and an instrument that silently disagrees with the app on a whole class of
    // media is worse than no instrument at all.
    let mut picts = app::graphics::PictSource::resolve(p, entry);
    // No tier-3 archive is named here, so the machine comes from the medium alone.
    let (profile, profile_source) =
        app::interpreter::InterpreterProfile::resolve_with_source(p, None, None, disk_image);
    zvm::screen::set_palette(profile.palette());
    let dims = picts.all_pict_dims();
    // The screen size the game is TOLD it has, by `startup.rs`'s own chain. The
    // `native_std_window` step is not optional decoration: it is the archive's own
    // picture space, and it is the only step that answers for a press whose art is
    // not 640x400. Arthur's ProDOS release 63 is such a press — the app gives it a
    // 560x384 screen, and a boot that skips this step hands it 640x400 instead, so
    // the GAME lays its own windows out differently and every rect measured
    // afterwards describes a screen the player never sees. `art_scale` rides along
    // for the same reason (SQ-0790): a 320-wide plate is drawn at (2,2).
    // SQ-1022: one call, so this instrument cannot drift from the app again. It
    // had drifted twice — SQ-0901 caught it omitting `native_std_window`, SQ-1020
    // caught it omitting the CELL — and both times the numbers it printed were
    // perfectly self-consistent descriptions of a screen the app never draws.
    // Note it also never had the NAMED-ARCHIVE link; it has it now, for free.
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        // SQ-1009: and the release's own typeface, which the DECLARED cell now
        // follows — omit it and Arthur's Amiga floppy is measured on a 16-row line
        // where the app gives it 20. The third omission this instrument would have
        // made, after `native_std_window` and the cell itself.
        // SQ-1037: the WHOLE cascade, including the player's own boot disks, so this
        // instrument reports the face the app would actually draw with rather than
        // the release rung alone.
        app::native_font::resolve(&app::native_font::FaceRequest {
            story_path: p,
            entry,
            profile,
            source: profile_source,
            art_scale: picts.art_scale(),
            disks: Some(&app::system_fonts::UserDisks::new("")),
        }),
    );
    let (std_win, art_scale) = (boot.screen_px, boot.art_scale);
    println!(
        "  booted as {:?}{}  ·  release {} serial {}  ·  screen {}  art scale {:?}",
        profile,
        disk_image.map(|m| format!(" off {m:?}")).unwrap_or_default(),
        u16::from_be_bytes([bytes[2], bytes[3]]),
        String::from_utf8_lossy(&bytes[0x12..0x18]),
        std_win.map_or("(none)".into(), |(w, h)| format!("{w}x{h}")),
        art_scale,
    );
    println!(
        "  cell {}x{}  ·  face {}",
        boot.cell.w(),
        boot.cell.h(),
        boot.faces.body().map_or("(none)".to_string(), |f| format!(
            "{}x{}{}",
            f.width,
            f.height,
            if f.proportional { " proportional" } else { "" }
        )),
    );
    let mut s = GameSession::new_for_machine(bytes, true, false, false, dims, None, None, &boot)
    .map_err(|e| format!("boot: {e:?}"))?;
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();

    // Tap through the intro to a frame that has a ring on it — unless asked to
    // report the BOOT frame as it stands, which is the only way to see a screen the
    // router sends to the composite before a viewport is ever computed.
    let tap_count = taps.unwrap_or(if no_tap { 0 } else { 6 });
    let stop_at_line = taps.is_none();
    for _ in 0..tap_count {
        let r = match s.pending_input() {
            InputKind::Line => {
                let r = s.submit("");
                if stop_at_line {
                    break;
                }
                Some(r.transcript)
            }
            InputKind::Char => {
                let b = keys.bytes().next().unwrap_or(13);
                Some(s.submit_char(b).transcript)
            }
            InputKind::Event => Some(s.submit("").transcript),
        };
        // Arthur and Journey both open on "restore a saved position?", and a story
        // still sitting on that question is not a frame worth measuring — the intro
        // is what gets reported, at every tap count, forever. The same answer the
        // suites' `drive()` gives (`v6_side_border_tiling.rs`), so the instrument and
        // the tests walk the same path to the same screen.
        if r.is_some_and(|t| t.to_lowercase().contains("y or n")) {
            let _ = s.submit_char(b'n');
        }
    }

    // …then as many further turns as asked, reporting (b) at each: a declared box
    // that avoids the art at BOOT may stop doing so once the game draws into its
    // own story window.
    for t in 0..=turns {
        if t > 0 {
            match s.pending_input() {
                InputKind::Line => { let _ = s.submit(cmd.unwrap_or("look")); }
                InputKind::Char => { let _ = s.submit_char(13); }
                InputKind::Event => { let _ = s.submit(""); }
            }
            let _ = s.take_transcript();
        }
        let m = s.screen();
        let app::engine::WinNode::Layered(it) = &m.root else { continue };
        let nat = v6::native_extent(it.as_slice(), &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let lay = v6::classify_windows(it.as_slice(), zvm::screen::V6Cell::DEFAULT);
        let pd = (pane_cells.0 as u32 * cell_px.0 as u32, pane_cells.1 as u32 * cell_px.1 as u32);
        let sc = v6::uniform_scale(nat, pd);
        let vp_box = v6::story_viewport_box(lay.story, &sc, pane_cells, cell_px);
        let g = v6::build_graphics_canvas(&lay.chrome, nat);
        let vp_clear = clear_viewport_cells(lay.story, &g, &sc, pane_cells, cell_px);
        {
            // SQ-0897: the routing arm per FRAME, not just for the final one — a
            // hatch's reachability is a property of the frames a title passes
            // through, and a title is on the ring at one turn and off it at the
            // next (Arthur alternates plate/prose screens through his whole intro).
            let arm = lay.story.and_then(|w| {
                app::render::screen::picture_takeover_reason(w, &lay.chrome, lay.story_gfx, nat)
            });
            let prose = v6::story_clear_native(lay.story, &g)
                .and_then(|c| v6::story_prose_box(c, lay.story_gfx, zvm::screen::V6Cell::DEFAULT));
            println!(
                "  turn {t}: {} box {}x{}@({},{}) content {}x{}@({},{}); win0 {:?} -> clear {:?} prose {:?} [{}]",
                if vp_box == vp_clear { "same  " } else { "DIFFERS" },
                vp_box.width, vp_box.height, vp_box.x, vp_box.y,
                vp_clear.width, vp_clear.height, vp_clear.x, vp_clear.y,
                lay.story.map(|w| (w.x_px, w.y_px, w.w_px, w.h_px)),
                v6::story_clear_native(lay.story, &g),
                prose,
                arm.unwrap_or("ring"),
            );
        }
    }

    let model = s.screen();
    let app::engine::WinNode::Layered(items) = &model.root else {
        return Err("not a Layered v6 frame".into());
    };

    let native = v6::native_extent(items.as_slice(), &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = v6::classify_windows(items.as_slice(), zvm::screen::V6Cell::DEFAULT);
    let pane_dev = (pane_cells.0 as u32 * cell_px.0 as u32, pane_cells.1 as u32 * cell_px.1 as u32);
    let scale = v6::uniform_scale(native, pane_dev);
    let pane = Rect::new(0, 0, pane_cells.0, pane_cells.1);
    let viewport = v6::story_viewport_box(layout.story, &scale, pane_cells, cell_px);

    println!(
        "  native {}x{}  scale {:.4} off ({},{})  viewport {}x{} at ({},{})  chrome windows {}",
        native.0, native.1, scale.s, scale.off_x, scale.off_y,
        viewport.width, viewport.height, viewport.x, viewport.y,
        layout.chrome.len(),
    );

    // SQ-0894 step (b): the text region the PANELS leave, versus the raw window box
    // hybrid takes today. `story_viewport` is the shrink-until-clear-then-quantize
    // wrapper that has never had a production caller; measured here against the
    // ART-ONLY canvas, which is the oracle raster uses and the one §3(b) says it
    // needs (against the full chrome canvas — which carries rasterised TEXT as
    // opaque pixels — Shogun's declared 548x64 box comes back 548x16).
    let gfx = v6::build_graphics_canvas(&layout.chrome, native);
    let clear = clear_viewport_cells(layout.story, &gfx, &scale, pane_cells, cell_px);
    let native_clear = v6::story_clear_native(layout.story, &gfx);
    println!(
        "  (b) viewport by BOX   {}x{} at ({},{})\n      viewport by CONTENT {}x{} at ({},{}){}",
        viewport.width, viewport.height, viewport.x, viewport.y,
        clear.width, clear.height, clear.x, clear.y,
        if clear == viewport { "   [no change]" } else { "   <-- DIFFERS" },
    );
    if let Some((l, t, w, h)) = native_clear {
        let sw = layout.story.map(|s| (s.x_px, s.y_px, s.w_px, s.h_px));
        println!("      native: declared {sw:?} -> clear ({l},{t},{w},{h})");
    }

    // SQ-0896: the same question asked of an oracle that ALSO carries the story
    // window's own plate. `build_graphics_canvas` is chrome-only by construction —
    // `classify_windows` sets a `win == 0` Graphics aside as `story_gfx` so the ring
    // does not carry it — so art the game painted INSIDE window 0 is invisible to
    // the clear probe above, and that is the capability gap: hybrid opens its
    // transcript viewport straight over the plate and never draws a pixel of it.
    //
    // The plate is NOT asked for by insetting the edges — `story_clear_native`
    // cannot see a picture that touches none of them, and cannot see one that
    // touches all four either (fmvpoker's hollow 640x400 table insets to width 0).
    // Raster's own composition is the right one and is reused rather than restated:
    // inset past the FRAME art with the chrome-only oracle above, then ask
    // `story_prose_box` for the largest rectangle of what is left that the PLATE
    // painted no pixel of — `None` when the plate owns the screen (SQ-0707), which
    // for the ring means the whole pane is chrome and no transcript belongs here.
    let plate = layout.story_gfx.map(|pw| (pw.x_px, pw.y_px, pw.w_px, pw.h_px));
    let prose = native_clear.and_then(|c| v6::story_prose_box(c, layout.story_gfx, zvm::screen::V6Cell::DEFAULT));
    println!(
        "      plate {plate:?}  ->  prose {prose:?}{}",
        match (native_clear, prose) {
            (Some(a), Some(b)) if a == b => "   [no change]",
            (None, None) => "   [no story window]",
            _ => "   <-- DIFFERS",
        }
    );

    // SQ-0897: and WHICH `picture_takeover` hatch, if any, keeps this frame off the
    // ring. Retiring them one at a time needs the arm named, not a boolean — the
    // four are OR'd over one frame and `art_paints_anything` subsumes the two shapes
    // below it, so "the gate is closed" says nothing about which arm shut it.
    let takeover = layout.story.and_then(|s| {
        app::render::screen::picture_takeover_reason(s, &layout.chrome, layout.story_gfx, native)
    });
    println!(
        "      picture_takeover: {}",
        match takeover {
            Some(arm) => format!("{arm}  -->  RASTER (the ring never runs on this frame)"),
            None => "none  -->  the ring".to_string(),
        }
    );

    // SQ-0892: every chrome run with the numbers the quantization argument turns on
    // — native origin, the sub-cell remainder of its mapped device x, and the cell
    // per-run rounding puts it in. Grouped by native text row, which is the key the
    // grouping rule is a refinement of.
    if want_runs {
        use std::collections::BTreeMap;
        let cw = cell_px.0.max(1) as f32;
        let mut rows: BTreeMap<u16, Vec<(&app::engine::PxText, u16)>> = BTreeMap::new();
        for it in &layout.chrome {
            if let app::engine::WinNode::Grid(g) = &it.node {
                for t in &g.px_texts {
                    rows.entry(t.y).or_default().push((t, it.w_px));
                }
            }
        }
        for it in items.iter() {
            let kind = match &it.node {
                app::engine::WinNode::Grid(g) => format!("Grid({} runs)", g.px_texts.len()),
                app::engine::WinNode::Buffer(b) => format!("Buffer(primary={})", b.primary),
                app::engine::WinNode::Graphics(_) => "Graphics".into(),
                _ => "other".into(),
            };
            println!("    win {}x{}@({},{}) {kind}", it.w_px, it.h_px, it.x_px, it.y_px);
        }
        println!("  RUNS ({} native rows):", rows.len());
        for (y, mut rr) in rows {
            rr.sort_by_key(|(t, _)| t.x);
            println!("    native y={y}  (y-1)%16={}", (y.max(1) - 1) % 16);
            for (t, win_w) in rr {
                let px = t.x.max(1) as f32 - 1.0;
                let dev = (scale.off_x as f32 + px * scale.s) / cw;
                println!(
                    "      x={:<4} n={:<3} win_w={:<4}({:>3}%) dev={:<8.3} cell={:<4} \
                     style={:#06b} fg={:?} bg={:?} {:?}",
                    t.x,
                    t.text.chars().count(),
                    win_w,
                    win_w as u32 * 100 / native.0.max(1) as u32,
                    dev,
                    dev.round() as i32,
                    t.style,
                    t.fg,
                    t.bg,
                    t.text,
                );
            }
        }
    }

    // SQ-0898: the ring as it is actually DRAWN — the strips the renderer classified
    // and the band log the graphics layer wrote, at this pane. The band log is the
    // only place a piece's native source and its device destination are reported
    // side by side, which is the pair the two extents must agree on: `native WxH`
    // (a crop of the shared scaled canvas, at the frame's one scale) against
    // `source WxH · resample A->B` (a caller-composed image, resized into the
    // band's cell box). A magnification that differs between two pieces of one
    // column is the defect this flag exists to catch, so it must be measurable
    // without a terminal.
    if want_bands {
        let mut st = app::state::AppState::default();
        st.colors = app::colors::ColorScheme::terminal_default();
        st.game_picker = Some(app::render::graphics::kitty_picker(cell_px.0, cell_px.1));
        st.config.v6_render = app::config::V6RenderMode::Hybrid;
        st.config.honor_game_colours = honor_colours;
        let mut buf = ratatui::buffer::Buffer::empty(pane);
        app::render::screen::render_story_pane(&model, false, None, &st, pane, &mut buf);
        println!("  RING AS DRAWN:");
        for c in st.v6_cell_map.borrow().iter() {
            if c.label.starts_with("strip:") || c.label.starts_with("menu:") {
                let (x, y, w, h) = c.cells;
                println!("    {} {}", c.label, fmt(Rect::new(x, y, w, h)));
            }
        }
        for line in &st.graphics_render.borrow().band_log {
            println!("    {line}");
        }
        // ONE FRAME, ONE MAGNIFICATION. Every band's device-per-native factor against
        // the frame's own letterbox scale, with the worst offender named: a column
        // drawn in two pieces at two magnifications is the seam SQ-0894 removed and
        // the corner fragment SQ-0898 is about, and neither is visible in the rects.
        let mags = st.graphics_render.borrow().band_mags.clone();
        for (r, fit, src, dst) in &mags {
            // How far the piece's far edge sits from where the frame's one scale puts
            // it, in device pixels: `|dst − src·s|` on the worse axis. Whole native
            // pixels cost up to half of one, so the honest allowance is one native
            // pixel (`s` device px, floored at 1 for a minifying pane).
            let off = (dst.0 as f32 - src.0 as f32 * scale.s)
                .abs()
                .max((dst.1 as f32 - src.1 as f32 * scale.s).abs());
            let bad = fit.on_the_letterbox_grid() && off > scale.s.max(1.0);
            println!(
                "    mag {:<20} {}x{}px from {}x{}n = {:.4}/{:.4} vs s={:.4}  → {off:.2} device px  [{fit:?}]{}",
                fmt(*r), dst.0, dst.1, src.0, src.1,
                dst.0 as f32 / src.0 as f32, dst.1 as f32 / src.1 as f32, scale.s,
                if bad { "   <-- DRIFT" } else { "" },
            );
        }
    }

    // The GEOMETRIC ring, `pane − viewport`. This is NOT what ships: SQ-0894 carves
    // the ring from CONTENT (`screen::content_ring_bands`, private to the render
    // module), so this is the baseline that rule is read against, not the answer.
    //
    // This block used to print an "axis-inverted" tiling beside it as PROPOSED —
    // flanks full pane height owning the corners. SQ-0894 MEASURED that proposal and
    // rejected it: it tiles exactly on all eight corpus frames and still breaks two,
    // cutting Arthur's full-width status row three ways and swallowing the left 37
    // columns of Journey's verb menu. It is gone rather than left printing, because a
    // scout that offers a known-wrong answer under the heading "PROPOSED" is worse
    // than one that offers none (SQ-0892).
    let bands = v6::chrome_bands(pane, viewport);
    println!("  GEOMETRIC ring ({} bands, pane − viewport):", bands.len());
    for (role, b) in &bands {
        println!("    {:<22} {}", format!("{role:?}"), fmt(*b));
    }
    let pane_area = pane.width as u32 * pane.height as u32;
    let vp_area = viewport.width as u32 * viewport.height as u32;
    let area: u32 = bands.iter().map(|(_, r)| r.width as u32 * r.height as u32).sum();
    println!(
        "  tiling check: pane {pane_area} − viewport {vp_area} = {} · bands {area}{}",
        pane_area - vp_area,
        if area == pane_area - vp_area { " ✓" } else { "  ✗ MISMATCH" },
    );
    println!();
    Ok(())
}

fn fmt(r: Rect) -> String {
    format!("{}x{} at ({},{})", r.width, r.height, r.x, r.y)
}
