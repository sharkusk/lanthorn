//! Drive lanthorn under a pty and write out the escape stream it emits, decoded
//! (SQ-0762). The ad-hoc face of the harness the integration test uses.
//!
//! ```sh
//! cargo build -p lanthorn                      # the harness runs the REAL binary
//! cargo run -p lanthorn --example pty_capture -- \
//!     --story "stories/Journey - The Quest Begins.adf" \
//!     --size 117x64 --keys cr,wait:800,cr,wait:800,cr \
//!     --out /tmp/journey.stream.txt --png /tmp/journey.png
//! ```
//!
//! `--png` draws the screen the oracle resolved (SQ-0775), for when the question
//! is "does this frame look right" and there is no terminal to answer it in. It
//! is an oracle for layout, art placement and colour — not a screenshot; the
//! caveats are in `tests/pty_stream/raster.rs`.
//!
//! The report names the graphics protocol that actually negotiated first, and
//! refuses to pretend: if lanthorn fell back to half-blocks, everything below it
//! measures the wrong backend and the exit status says so.

#[cfg(unix)]
#[path = "../tests/pty_stream/mod.rs"]
mod pty_stream;

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    use std::path::PathBuf;
    use std::time::Duration;

    use pty_stream::driver::{self, Key, Spec};

    let mut story: Option<PathBuf> = None;
    let mut bin: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut raw: Option<PathBuf> = None;
    let mut png: Option<PathBuf> = None;
    let mut png_diff: Option<PathBuf> = None;
    let mut user_dir: Option<PathBuf> = None;
    let mut keys: Vec<Key> = Vec::new();
    let mut cols = 117u16;
    let mut rows = 64u16;
    let mut cell = (8u16, 18u16);
    let mut hide_map = true;
    let mut timeout = 45u64;
    let mut extra: Vec<String> = Vec::new();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let need = |i: usize| -> String {
            args.get(i + 1).cloned().unwrap_or_else(|| {
                eprintln!("pty_capture: `{}` needs a value", args[i]);
                std::process::exit(2);
            })
        };
        match args[i].as_str() {
            "--story" => {
                story = Some(PathBuf::from(need(i)));
                i += 1;
            }
            "--bin" => {
                bin = Some(PathBuf::from(need(i)));
                i += 1;
            }
            "--out" => {
                out = Some(PathBuf::from(need(i)));
                i += 1;
            }
            "--raw" => {
                raw = Some(PathBuf::from(need(i)));
                i += 1;
            }
            "--png" => {
                png = Some(PathBuf::from(need(i)));
                i += 1;
            }
            "--png-diff" => {
                png_diff = Some(PathBuf::from(need(i)));
                i += 1;
            }
            "--user-dir" => {
                user_dir = Some(PathBuf::from(need(i)));
                i += 1;
            }
            "--size" => {
                let v = need(i);
                let (c, r) = v.split_once('x').unwrap_or_else(|| {
                    eprintln!("pty_capture: --size wants COLSxROWS, got `{v}`");
                    std::process::exit(2);
                });
                cols = c.trim().parse().unwrap_or(cols);
                rows = r.trim().parse().unwrap_or(rows);
                i += 1;
            }
            "--cell" => {
                let v = need(i);
                if let Some((w, h)) = v.split_once('x') {
                    cell = (w.trim().parse().unwrap_or(8), h.trim().parse().unwrap_or(18));
                }
                i += 1;
            }
            "--keys" => {
                for tok in need(i).split(',').filter(|t| !t.trim().is_empty()) {
                    match Key::parse(tok) {
                        Ok(k) => keys.push(k),
                        Err(e) => {
                            eprintln!("pty_capture: {e}");
                            return std::process::ExitCode::from(2);
                        }
                    }
                }
                i += 1;
            }
            "--show-map" => hide_map = false,
            "--timeout" => {
                timeout = need(i).parse().unwrap_or(timeout);
                i += 1;
            }
            "--arg" => {
                extra.push(need(i));
                i += 1;
            }
            "-h" | "--help" => {
                println!("{HELP}");
                return std::process::ExitCode::SUCCESS;
            }
            other => {
                eprintln!("pty_capture: unknown option `{other}`\n\n{HELP}");
                return std::process::ExitCode::from(2);
            }
        }
        i += 1;
    }

    let Some(story) = story else {
        eprintln!("pty_capture: --story is required\n\n{HELP}");
        return std::process::ExitCode::from(2);
    };
    if !story.is_file() {
        eprintln!("pty_capture: no story at {}", story.display());
        return std::process::ExitCode::from(2);
    }
    let Some(bin) = bin.or_else(driver::sibling_lanthorn) else {
        eprintln!(
            "pty_capture: cannot find the lanthorn binary. Run `cargo build -p lanthorn` first, \
             or pass --bin <path> / set LANTHORN_BIN."
        );
        return std::process::ExitCode::from(2);
    };
    if keys.is_empty() {
        keys = vec![Key::Wait(Duration::from_millis(1500))];
    }

    let user_dir = user_dir.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("lanthorn-pty-{}", std::process::id()))
    });
    let mut spec = Spec::new(bin, story, user_dir);
    spec.cols = cols;
    spec.rows = rows;
    spec.cell_w = cell.0;
    spec.cell_h = cell.1;
    spec.hide_map = hide_map;
    spec.keys = keys;
    spec.timeout = Duration::from_secs(timeout);
    spec.extra_args = extra;

    let cap = match driver::run(spec) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("pty_capture: {e}");
            return std::process::ExitCode::from(1);
        }
    };
    if let Some(path) = &raw {
        if let Err(e) = std::fs::write(path, &cap.bytes) {
            eprintln!("pty_capture: writing {}: {e}", path.display());
        }
    }
    let term = pty_stream::decode_capture(&cap);
    // One resolution, shared by the report and the picture: they must describe
    // the same screen, and a second `resolve` on a multi-megabyte capture is the
    // most expensive thing this program does.
    let res = pty_stream::oracle::resolve(
        // SQ-0976: the oracle's terminal core links no zlib, so it must be handed
        // the stream with `o=z` undone or it drops every image.
        &cap.terminal_bytes(),
        cap.spec.cols,
        cap.spec.rows,
        u32::from(cap.spec.cell_w),
        u32::from(cap.spec.cell_h),
        Some((pty_stream::ANSWERED_FG, pty_stream::ANSWERED_BG)),
    );
    if let Some(path) = &png {
        let mut canvas = pty_stream::raster::render(&res);
        // A before/after pair is one flag, not a second mode: capture the old
        // build to a PNG, then run the new one with that file as `--png-diff`.
        // Nothing here has to know how the earlier picture was produced.
        if let Some(before) = &png_diff {
            match image::open(before) {
                Ok(img) => canvas = pty_stream::raster::side_by_side(&img.to_rgba8(), &canvas),
                Err(e) => eprintln!("pty_capture: reading {}: {e} — writing this frame alone", before.display()),
            }
        }
        match canvas.save(path) {
            Ok(()) => println!(
                "png      : {} ({}x{} px, {} cell(s) at {}x{})",
                path.display(),
                canvas.width(),
                canvas.height(),
                u32::from(cap.spec.cols) * u32::from(cap.spec.rows),
                cap.spec.cell_w,
                cap.spec.cell_h
            ),
            Err(e) => eprintln!("pty_capture: writing {}: {e}", path.display()),
        }
    }
    let mut text = pty_stream::report(&cap, &term);
    text.push_str(&oracle_section(&res, &term));
    match &out {
        Some(path) => {
            if let Err(e) = std::fs::write(path, &text) {
                eprintln!("pty_capture: writing {}: {e}", path.display());
                return std::process::ExitCode::from(1);
            }
            // The decisive parts on stdout, the whole thing in the file.
            let head: String = text.lines().take_while(|l| !l.starts_with("--- flushes")).collect::<Vec<_>>().join("\n");
            println!("{head}");
            print!("{}", pty_stream::screen_report(&term));
            print!("{}", oracle_section(&res, &term));
            println!("\nfull report: {}", path.display());
        }
        None => print!("{text}"),
    }
    if cap.negotiated().is_kitty() {
        std::process::ExitCode::SUCCESS
    } else {
        eprintln!("pty_capture: the graphics protocol did not negotiate to kitty — see the VERDICT above");
        std::process::ExitCode::from(3)
    }
}

/// The second opinion (SQ-0764). Everything above this section is our own
/// decoder's reading of the stream; this is a real terminal emulator's reading of
/// the identical bytes. Two things it can say that ours cannot: which placements
/// would actually put PIXELS on the screen (a placeholder run whose image id no
/// longer resolves is cells on screen and nothing drawn), and — by disagreeing —
/// which of the two decoders to distrust.
#[cfg(unix)]
fn oracle_section(
    res: &pty_stream::oracle::Resolved,
    term: &pty_stream::decode::Term,
) -> String {
    use std::fmt::Write as _;

    let mut s = String::new();
    let _ = writeln!(s, "\n--- oracle: the placements a real terminal emulator would draw ---");
    s.push_str(&res.describe_placements());

    let d = pty_stream::oracle::disagreements(term, res);
    let _ = writeln!(
        s,
        "\n--- oracle disagreements ({}; empty = both decoders read these bytes the same) ---",
        if d.is_empty() { "none".to_string() } else { format!("{} run(s)", d.len()) }
    );
    for line in &d {
        let _ = writeln!(s, "  {line}");
    }
    s
}

#[cfg(unix)]
const HELP: &str = "\
pty_capture — drive lanthorn under a pty and decode the escapes it emits

  --story PATH        story file to run (required)
  --bin PATH          lanthorn binary (default: beside this example, or $LANTHORN_BIN)
  --size COLSxROWS    terminal size in cells (default 117x64 — a 115x61 story pane)
  --cell WxH          cell size in pixels to report for CSI 16 t (default 8x18)
  --keys LIST         comma list: cr esc tab space bs up down left right home end
                      pgup pgdn f1..f4, ctrl+X, text:foo, wait:MS
  --show-map          keep the map pane (default: hidden, so the story pane owns the frame)
  --user-dir PATH     throwaway lanthorn home (default: a temp dir)
  --out PATH          write the full report here (stdout gets the decisive part)
  --raw PATH          also write the raw captured bytes
  --png PATH          also draw the resolved screen here (layout/art/colour only —
                      an 8x8 bitmap font, not a screenshot; see pty_stream/raster.rs)
  --png-diff BEFORE   put an earlier --png beside this run's, so a render change
                      can be reviewed as a before/after pair
  --timeout SECS      hard ceiling on the run (default 45)
  --arg VALUE         extra argument passed through to lanthorn — which is how a
                      story on a volume holding several is named, since lanthorn
                      itself takes the choice: --arg --story --arg arthur

Exit status 3 means the run did not negotiate kitty graphics — the capture then
measures the wrong backend and must not be trusted.";

#[cfg(not(unix))]
fn main() {
    eprintln!("pty_capture: a pty harness is unix-only; nothing to run on this platform.");
}
