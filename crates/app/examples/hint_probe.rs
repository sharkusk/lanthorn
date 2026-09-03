//! SCRATCH instrument (SQ-1056/1069/1070): walk a v6 game's InvisiClues route and
//! print, per step, what the HYBRID and RASTER paths each decide about the story
//! window — so the one decision they make differently can be named rather than
//! guessed at.
//!
//! Boots the way `startup.rs` boots (CLAUDE.md) and MIRRORS every `TurnResult`
//! into a real `AppState` the way `turn.rs` does, because the host transcript is
//! what raster draws the story from and a harness with an empty transcript cannot
//! see any of these defects.
//!
//! Usage:
//!   cargo run -p lanthorn --example hint_probe -- --story <path> [--entry N|NAME]
//!       [--pane 132x60] [--route 'L:;L:hint;C:y;C:13;C:13']

use app::engine::{Engine, WinNode};
use app::session::GameSession;
use app::state::TranscriptKind;
use ratatui::layout::Rect;

/// What to print per step, beyond the one-line summary every step gets.
#[derive(Default)]
struct Opts {
    /// `--runs N[,N…]` — every painted run in those windows.
    runs: Vec<usize>,
    /// `--windows` — every live window's box, attributes and grid.
    windows: bool,
    /// `--trace` — the screen ops the turn issued (window_style, window_size, …).
    trace: bool,
    /// `--hybrid-first` — render a hybrid frame BEFORE the raster one, to see
    /// whether per-frame state a hybrid frame leaves behind reaches the composite.
    hybrid_first: bool,
    /// `--hybrid-only` — skip the raster half entirely.
    hybrid_only: bool,
    /// `--paint` — the hybrid pane as characters, and its cell GROUNDS beside it.
    paint: bool,
}

fn main() {
    let mut story: Option<String> = None;
    let mut entry: Option<String> = None;
    let mut route = String::from("L:;L:hint;C:y;C:13;C:13");
    let mut pane = (132u16, 60u16);
    let mut opts = Opts::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--story" => story = args.next(),
            "--entry" => entry = args.next(),
            "--route" => route = args.next().unwrap_or_default(),
            "--hybrid-only" => opts.hybrid_only = true,
            "--paint" => opts.paint = true,
            "--windows" => opts.windows = true,
            "--trace" => opts.trace = true,
            "--hybrid-first" => opts.hybrid_first = true,
            "--runs" => {
                opts.runs = args
                    .next()
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|n| n.parse().ok())
                    .collect();
            }
            "--pane" => {
                let v = args.next().unwrap_or_default();
                let (w, h) = v.split_once('x').expect("--pane WxH");
                pane = (w.parse().unwrap(), h.parse().unwrap());
            }
            other => panic!("unknown arg {other:?}"),
        }
    }
    let story = story.expect("--story <path>");
    let p = std::path::Path::new(&story);
    // `--entry` is a 1-based position or enough of a name to pick out one story
    // — the rule `lanthorn --story` and `zvm-cli --story` both match by
    // (SQ-1078). It used to be the stored name LITERALLY, which you could only
    // learn by mounting the disc yourself.
    let entry = match entry.as_deref().map(|w| app::story_pick::entry_on(p, w)) {
        Some(Ok(e)) => e,
        Some(Err(msg)) => {
            eprintln!("hint_probe: {msg}");
            std::process::exit(2);
        }
        None => None,
    };
    let entry = entry.as_deref();

    // ── boot as startup.rs boots ─────────────────────────────────────────────
    let (bytes, medium) = match app::hints::load_mounted_story_from(p, entry) {
        Ok((loaded, m)) => (loaded.bytes().to_vec(), m),
        Err(e) => {
            eprintln!("SKIP: {story}: {e:?}");
            return;
        }
    };
    let mut picts = app::graphics::PictSource::resolve(p, entry);
    let (profile, source) =
        app::interpreter::InterpreterProfile::resolve_with_source(p, None, None, medium);
    let _g = app::v6_palette(profile.palette());
    let dims = picts.all_pict_dims();
    let face = app::native_font::resolve(&app::native_font::FaceRequest {
        story_path: p,
        entry,
        profile,
        source,
        art_scale: picts.art_scale(),
        disks: Some(&app::system_fonts::UserDisks::new("")),
    });
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        face,
    );
    let art_scale = boot.art_scale;
    let text_face = boot.text_face();
    let mut s = GameSession::new_for_machine(bytes.clone(), true, false, false, dims, None, None, &boot)
        .expect("boots");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    s.machine.trace_screen = opts.trace;
    println!(
        "{story}  ·  r{} s{}  ·  {profile:?}  ·  screen {:?}  art {art_scale:?}  cell {}x{}",
        u16::from_be_bytes([bytes[2], bytes[3]]),
        String::from_utf8_lossy(&bytes[0x12..0x18]),
        boot.screen_px,
        text_face.cell().w(),
        text_face.cell().h(),
    );

    // ── the host state the render paths actually read ────────────────────────
    let mut st = app::state::AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st.game_picker = Some(app::render::graphics::kitty_picker(8, 18));
    st.config.honor_game_colours = true;
    if let Some(sc) = art_scale { st.v6_art_scale = sc; }
    st.v6_text = text_face;

    let steps: Vec<&str> = route.split(';').collect();
    for (i, step) in steps.iter().enumerate() {
        let label = *step;
        let r = match step.split_once(':') {
            Some(("L", text)) => {
                if !text.is_empty() {
                    st.push_transcript(&format!("> {text}"));
                }
                s.submit(text)
            }
            Some(("C", k)) => {
                let b = if k == "13" { 13u8 } else { k.as_bytes()[0] };
                s.submit_char(b)
            }
            // T:n — n intro taps, each answering whatever the game is waiting on.
            Some(("T", n)) => {
                let n: usize = n.parse().expect("T:<count>");
                let mut last = None;
                for _ in 0..n {
                    let r = match s.pending_input() {
                        app::session::InputKind::Line => s.submit(""),
                        app::session::InputKind::Char => s.submit_char(13),
                        app::session::InputKind::Event => s.submit(""),
                    };
                    if r.erase_lower {
                        if let Some(a) = st.clear_anchor {
                            st.truncate_transcript(a);
                        }
                        st.mark_screen_clear();
                    }
                    st.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
                    // Arthur's intro asks a Y/N question the taps cannot answer.
                    if r.transcript.to_lowercase().contains("y or n") {
                        let _ = s.submit_char(b'n');
                    }
                    last = Some(r);
                }
                last.expect("T:0 is not a step")
            }
            _ => panic!("route step {step:?}: want L:text, C:x or T:n"),
        };
        if let Some(f) = &r.fault {
            println!("  !! fault at {label}: {f:?}");
        }
        // What `turn.rs` does with the result, so the transcript is the app's.
        if r.erase_lower {
            if let Some(a) = st.clear_anchor {
                st.truncate_transcript(a);
            }
            st.mark_screen_clear();
        }
        if r.transcript_elems.is_empty() {
            st.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
        } else {
            app::state::apply_transcript_elems(&mut st, &r.transcript_elems);
        }
        report(&s, &mut st, i, label, pane, r.erase_lower, &opts);
    }
}

fn report(
    s: &GameSession,
    st: &mut app::state::AppState,
    i: usize,
    label: &str,
    pane: (u16, u16),
    erased: bool,
    opts: &Opts,
) {
    use app::render::v6_layout as v6;
    let v6w = s.machine.screen.v6.as_ref().expect("v6");
    let w0 = &v6w.windows[0];
    let cur = v6w.current;
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else {
        println!("[{i}] {label}: not a Layered v6 model");
        return;
    };
    let native = v6::native_extent(items.as_slice(), &st.v6_text);
    let layout = v6::classify_windows(items.as_slice(), st.v6_text.cell());
    let kind = match layout.story.map(|w| &w.node) {
        Some(WinNode::Buffer(b)) if b.primary => "Buffer(primary)",
        Some(WinNode::Buffer(_)) => "Buffer(panel)",
        Some(WinNode::Grid(_)) => "Grid",
        Some(WinNode::Graphics(_)) => "Graphics",
        Some(_) => "other",
        None => "NONE",
    };
    println!(
        "[{i}] {label:<10} cur=w{cur} w0={}x{}@({},{}) attrs={:04b} texts={} streamed={} prose={} | story={kind} \
         native={native:?} | transcript={} anchor={:?} erased={erased}",
        w0.x_size, w0.y_size, w0.x_coord, w0.y_coord,
        w0.attributes & 0b1111,
        w0.texts.len(),
        w0.streamed.len(),
        w0.prose.len(),
        st.transcript.len(),
        st.clear_anchor,
    );

    if opts.windows {
        for (n, w) in v6w.windows.iter().enumerate() {
            if w.x_size == 0 && w.texts.is_empty() { continue; }
            println!("       w{n} box={}x{}@({},{}) attrs={:04b} grid={}x{} lm={} rm={} fg={:?} bg={:?} runs={}",
                w.x_size, w.y_size, w.x_coord, w.y_coord, w.attributes & 0b1111,
                w.grid.cols, w.grid.rows, w.left_margin, w.right_margin,
                w.fg, w.bg, w.texts.len());
        }
    }
    for &n in &opts.runs {
        {
            for t in v6w.windows[n].texts.iter() {
                println!("       w{n} run @({:>4},{:>4}) w={:>4} style={} {:?}", t.x, t.y, s.machine.v6_metric.run_px(&t.text, t.style), t.style, t.text);
            }
        }
    }

    if opts.trace {
        for line in s.machine.screen_trace.iter() {
            if line.contains("output_stream") || line.contains("window_style") || line.contains("put_wind_prop") || line.contains("window_size") || line.contains("set_window") {
                println!("       trace {line}");
            }
        }
    }

    if opts.hybrid_first {
        st.config.v6_render = app::config::V6RenderMode::Hybrid;
        let area = Rect::new(0, 0, pane.0, pane.1);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, st, area, &mut buf);
    }

    // ── RASTER: which arm, and how much ink lands in the story box ───────────
    if !opts.hybrid_only {
        st.config.v6_render = app::config::V6RenderMode::Raster;
        let (canvas, metrics) = app::render::screen::build_v6_raster_canvas(&layout, native, st);
        let clear = v6::story_clear_native(layout.story, &v6::build_graphics_canvas(&layout.chrome, native));
        let prose_box = clear.and_then(|c| v6::story_prose_box(c, layout.story_gfx, st.v6_text.cell()));
        println!(
            "     raster: clear={:?} prose_box={:?} metrics={:?} canvas={}x{}",
            clear,
            prose_box,
            metrics,
            canvas.width(),
            canvas.height()
        );
        if opts.paint {
            // The composite, downsampled to one character per 8x16 native cell,
            // keyed by colour — a solid rectangle shows up as a block of one key.
            let (cw, ch) = (8u32, 16u32);
            let mut key: Vec<[u8; 4]> = Vec::new();
            println!("     raster canvas {}x{} as {}x{} cells:", canvas.width(), canvas.height(), canvas.width() / cw, canvas.height() / ch);
            for row in 0..canvas.height() / ch {
                let mut line = String::new();
                for col in 0..canvas.width() / cw {
                    // The cell's most common pixel, so a glyph does not hide its ground.
                    let mut counts: std::collections::HashMap<[u8; 4], u32> = Default::default();
                    for y in row * ch..(row + 1) * ch {
                        for x in col * cw..(col + 1) * cw {
                            *counts.entry(canvas.get_pixel(x, y).0).or_default() += 1;
                        }
                    }
                    let px = counts.into_iter().max_by_key(|(_, n)| *n).map(|(p, _)| p).unwrap_or([0; 4]);
                    let i = key.iter().position(|k| *k == px).unwrap_or_else(|| {
                        key.push(px);
                        key.len() - 1
                    });
                    line.push(char::from_u32('a' as u32 + i as u32).unwrap_or('?'));
                }
                println!("     {row:>3} {line}");
            }
            for (i, k) in key.iter().enumerate() {
                println!("     key {} = rgba{:?}", char::from_u32('a' as u32 + i as u32).unwrap_or('?'), k);
            }
        }
        if let Some((tx, ty, tw, th)) = prose_box {
            let _ = (tx, ty);
            let cols = (tw / u32::from(st.v6_text.cell().w())).max(1) as u16;
            let rows = (th / u32::from(st.v6_text.cell().h())).max(1) as u16;
            let (main, _) = app::render::screen::build_main_text(st, cols, rows);
            println!("     raster story box {cols}x{rows}, {} line(s):", main.lines.len());
            for l in main.lines.iter() {
                println!("       | {l}");
            }
        }
    }

    // ── HYBRID: the pane as cells, and what is legible in it ────────────────
    st.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, pane.0, pane.1);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, st, area, &mut buf);
    let path = st.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default();
    let rows: Vec<String> = (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf.cell((x, y)).map_or(' ', |c| c.symbol().chars().next().unwrap_or(' ')))
                .collect()
        })
        .collect();
    let live = rows.iter().filter(|r| r.trim_matches(|c: char| c == ' ' || c == '\u{10EEEE}').len() > 2).count();
    println!("     hybrid: path={path:?} rows-with-text={live}/{}", rows.len());
    if opts.windows {
        for c in st.v6_cell_map.borrow().iter() {
            if c.label.starts_with("strip:") || c.label.starts_with("menu:") || c.label.starts_with("viewport") {
                let (x, y, w, h) = c.cells;
                println!("       ring {} {}x{}@({},{})", c.label, w, h, x, y);
            }
        }
    }
    if opts.paint {
        // Each cell as its GROUND: '.' untouched, '#' a placeholder (an image
        // cell), a letter for a glyph, and the ground's own colour keyed below.
        let mut key: Vec<ratatui::style::Color> = Vec::new();
        for y in 0..area.height {
            let mut line = String::new();
            for x in 0..area.width {
                let c = buf.cell((x, y)).expect("in area");
                let ch = c.symbol().chars().next().unwrap_or(' ');
                let ground = if c.modifier.contains(ratatui::style::Modifier::REVERSED) { c.fg } else { c.bg };
                if ch == '\u{10EEEE}' {
                    line.push('#');
                } else if ground == ratatui::style::Color::Reset {
                    line.push(if ch == ' ' { '.' } else { ch });
                } else {
                    let i = key.iter().position(|k| *k == ground).unwrap_or_else(|| {
                        key.push(ground);
                        key.len() - 1
                    });
                    line.push(if ch == ' ' {
                        char::from_u32('0' as u32 + i as u32).unwrap_or('?')
                    } else {
                        ch
                    });
                }
            }
            println!("     {y:>3} {line}");
        }
        for (i, k) in key.iter().enumerate() {
            println!("     ground {i} = {k:?}");
        }
    }
}
