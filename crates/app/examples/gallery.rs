//! Regenerate the project gallery: presentable frames across games, presses and
//! pane sizes, all of them from the REAL binary (SQ-0942).
//!
//! ```sh
//! cargo build -p lanthorn                          # the harness runs the real binary
//! cargo run -p lanthorn --example gallery           # the whole manifest
//! cargo run -p lanthorn --example gallery -- --only journey-amiga,arthur-flank
//! cargo run -p lanthorn --example gallery -- --font "/System/Library/Fonts/Monaco.ttf"
//! ```
//!
//! Output lands under `target/gallery/`: one PNG per shot, an `index.html` proof
//! sheet, and `gallery.json` — the record of what was captured from what. None
//! of it is committed. The committed artefact is `examples/gallery.toml`, which
//! is the recipe; the pictures are the result, and this repository's standing
//! rule is to persist the first and not the second.
//!
//! **The frames are resolved, not photographed**: the layout, the art placement
//! and the colour come out of the escape bytes lanthorn actually wrote, and the
//! type is rasterised here from a real terminal face. Which face, at what size,
//! is named under every frame — the honest version of the "RENDER, NOT A
//! SCREENSHOT" disclaimer these carried until SQ-1080, and more useful than it.
//!
//! See `tests/pty_stream/gallery.rs` for why the font lives here and not in the
//! geometry oracle.

#[cfg(unix)]
#[path = "../tests/pty_stream/mod.rs"]
mod pty_stream;

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    use std::path::PathBuf;
    use std::time::Duration;

    use pty_stream::gallery::{self, Manifest, Taken};

    let mut manifest_path: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut font: Option<PathBuf> = None;
    let mut bin: Option<PathBuf> = None;
    let mut only: Vec<String> = Vec::new();
    let mut bitmap = false;
    let mut timeout = 90u64;
    let mut retries = 3usize;
    let mut list = false;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let need = |i: usize| -> String {
            args.get(i + 1).cloned().unwrap_or_else(|| {
                eprintln!("gallery: `{}` needs a value", args[i]);
                std::process::exit(2);
            })
        };
        match args[i].as_str() {
            "--manifest" => {
                manifest_path = Some(PathBuf::from(need(i)));
                i += 1;
            }
            "--out" => {
                out = Some(PathBuf::from(need(i)));
                i += 1;
            }
            "--font" => {
                font = Some(PathBuf::from(need(i)));
                i += 1;
            }
            "--bin" => {
                bin = Some(PathBuf::from(need(i)));
                i += 1;
            }
            "--only" => {
                only.extend(need(i).split(',').filter(|s| !s.trim().is_empty()).map(|s| s.trim().to_string()));
                i += 1;
            }
            "--bitmap-font" => bitmap = true,
            "--list" => list = true,
            "--timeout" => {
                timeout = need(i).parse().unwrap_or(timeout);
                i += 1;
            }
            "--retries" => {
                retries = need(i).parse().unwrap_or(retries);
                i += 1;
            }
            "-h" | "--help" => {
                println!("{HELP}");
                return std::process::ExitCode::SUCCESS;
            }
            other => {
                eprintln!("gallery: unknown option `{other}`\n\n{HELP}");
                return std::process::ExitCode::from(2);
            }
        }
        i += 1;
    }

    let manifest_path = manifest_path.unwrap_or_else(Manifest::default_path);
    let manifest = match Manifest::load(&manifest_path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("gallery: {e}");
            return std::process::ExitCode::from(2);
        }
    };
    let wanted: Vec<_> = manifest
        .shots
        .iter()
        .filter(|s| only.is_empty() || only.iter().any(|o| o == &s.id))
        .collect();
    if wanted.is_empty() {
        eprintln!("gallery: --only matched no shot in {}", manifest_path.display());
        return std::process::ExitCode::from(2);
    }
    if list {
        for s in &wanted {
            println!(
                "{:<24} {:<10} {:<9} {:>3} keypress(es)  {}",
                s.id,
                s.size,
                s.backend.as_str(),
                s.turns(),
                s.media
            );
        }
        return std::process::ExitCode::SUCCESS;
    }

    let Some(bin) = bin.or_else(pty_stream::driver::sibling_lanthorn) else {
        eprintln!(
            "gallery: cannot find the lanthorn binary. Run `cargo build -p lanthorn` first, \
             or pass --bin <path> / set LANTHORN_BIN."
        );
        return std::process::ExitCode::from(2);
    };
    let out = out.unwrap_or_else(|| gallery::repo_root().join("target/gallery"));
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("gallery: {}: {e}", out.display());
        return std::process::ExitCode::from(1);
    }
    let work = std::env::temp_dir().join(format!("lanthorn-gallery-{}", std::process::id()));

    println!("gallery : {} shot(s) from {}", wanted.len(), manifest_path.display());
    println!("binary  : {} (lanthorn {})", bin.display(), buildinfo::LONG);
    println!("out     : {}\n", out.display());

    let mut taken: Vec<Taken> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    for shot in &wanted {
        print!("  {:<24} ", shot.id);
        let _ = std::io::Write::flush(&mut std::io::stdout());

        // A SHOT THAT FAILS TAKES ITS OLD FRAME WITH IT (SQ-0999). Until it did,
        // a failed shot left the previous PNG on disk and `index.html` presented
        // it beside the fresh ones identically — so a run reading "5 frame(s),
        // 8 failure(s)" still produced a sheet of thirteen pictures, eight of them
        // hours old. That is worse than a gap: it made a TOTAL loss of artwork
        // look like a partial regression, and cost two wrong hypotheses before
        // anyone checked the mtimes.
        let png = out.join(format!("{}.png", shot.id));
        let drop_stale = || {
            if png.is_file() {
                match std::fs::remove_file(&png) {
                    Ok(()) => eprintln!("       (removed the stale {})", png.display()),
                    Err(e) => eprintln!("       (could not remove the stale {}: {e})", png.display()),
                }
            }
        };

        // Provenance FIRST, and from the mount rather than the filename: a disk
        // image is a different build of the game, so a frame captioned with a
        // release it did not load is worse than one captioned with none.
        // What this shot opens — a medium, or one of the manifest's libraries —
        // settled once and handed to everything below, so the provenance under
        // the frame and the path the binary is launched with cannot disagree.
        let subject = match manifest.subject(shot) {
            Ok(s) => s,
            Err(e) => {
                println!("SKIP  {e}");
                failed.push(format!("{}: {e}", shot.id));
                drop_stale();
                continue;
            }
        };
        let provenance = match gallery::Provenance::of(&subject, shot.pictures()) {
            Ok(p) => p,
            Err(e) => {
                println!("SKIP  {e}");
                failed.push(format!("{}: {e}", shot.id));
                drop_stale();
                continue;
            }
        };
        let (cell_w, cell_h) = shot.cell_px();

        let face = if bitmap {
            gallery::Face::Bitmap
        } else {
            match gallery::pick_face(font.as_deref(), cell_h) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("\ngallery: {e} — falling back to the bitmap face");
                    gallery::Face::Bitmap
                }
            }
        };
        if let Some(c) = face.cell_complaint(cell_w, cell_h) {
            eprintln!("\ngallery: {c}");
        }

        // ONE RUN PER TILE, and exactly one run for every other shot in the file
        // (SQ-1165). `Shot::runs` answers `[None]` for an ordinary row, so the
        // composite is a longer loop over the same body rather than a second copy
        // of it — which is the whole argument for making this a shot KIND instead
        // of an example beside this one.
        let mut panels: Vec<image::RgbaImage> = Vec::new();
        let mut looks: Vec<(u8, gallery::PaneLook)> = Vec::new();
        let mut attempts = 0usize;
        let mut captured_bytes = 0usize;
        let mut verdict = String::new();
        let (mut cols, mut rows) = (0u16, 0u16);
        let mut last = String::new();
        let mut lost = false;

        for machine in shot.runs() {
            // RETRIES, because the thing being driven is a real interactive program
            // over a real pty and `wait:` is wall-clock. A key that lands while a
            // slow medium is still paging in reaches a different screen than the same
            // key three seconds later, and Arthur's ProDOS press — five segments
            // reassembled by block — does exactly that under load. Retrying is only
            // defensible because the guard decides: a shot that cannot pass its own
            // `expect` never becomes a picture, so this buys robustness and not a
            // second chance at being wrong.
            let mut cap = None;
            let mut res = None;
            for attempt in 1..=retries.max(1) {
                attempts = attempts.max(attempt);
                let c = match gallery::capture(shot, machine, &subject, &bin, &work, Duration::from_secs(timeout)) {
                    Ok(c) => c,
                    Err(e) => {
                        last = e;
                        continue;
                    }
                };
                // The capture's own bytes are the WIRE stream, and since SQ-0991 a kitty
                // transmit may be zlib-deflated (`o=z`). Resolving those directly finds the
                // placement and then has no pixels to paint, which is a BLANK illustration
                // panel and a shot that fails its art guard for a reason that looks like the
                // game's fault. `terminal_bytes()` inflates first, exactly as a terminal does.
                let r = pty_stream::oracle::resolve(
                    &c.terminal_bytes(),
                    c.spec.cols,
                    c.spec.rows,
                    u32::from(cell_w),
                    u32::from(cell_h),
                    Some((pty_stream::ANSWERED_FG, pty_stream::ANSWERED_BG)),
                );
                match gallery::check_expectations(shot, &r) {
                    Ok(()) => {
                        cap = Some(c);
                        res = Some(r);
                        break;
                    }
                    Err(e) => last = e,
                }
            }
            let (Some(cap), Some(res)) = (cap, res) else {
                lost = true;
                break;
            };
            captured_bytes += cap.bytes.len();
            verdict = cap.negotiated().explain();
            cols = cap.spec.cols;
            rows = cap.spec.rows;
            if let (Some(n), Some(look)) = (machine, gallery::pane_look(shot, &res)) {
                looks.push((n, look));
            }
            let mut frame = pty_stream::raster::render_with(&res, &|canvas, ch, px, py, cw, chh, fg| {
                face.draw(canvas, ch, px, py, cw, chh, fg)
            });
            // EACH TILE NAMES ITSELF, on the picture rather than in a strip above
            // it (SQ-1165) — the label that survives a crop. Where it goes is
            // FOUND per tile off that tile's own resolved screen, never assumed:
            // the badge must not land on the status band, the prose or the caret,
            // which are the four things the frame exists to show, and the six
            // tiles do not fill to the same height. A tile with no clear ground at
            // all keeps its badge off rather than covering the evidence, and says
            // so, because a silently misplaced tag is worse than a missing one.
            if let Some(n) = machine {
                let text = gallery::tile_badge_text(n);
                match gallery::badge_anchor(shot, &res, gallery::badge_cells(shot, &text)) {
                    Some(at) => gallery::stamp_badge(&mut frame, &text, at),
                    None => eprintln!(
                        "\ngallery: `{}` tile {n} ({text}) has no clear ground in its pane for a \
                         badge, so it is unlabelled — the frame is full to the edges, and a badge \
                         over the prose would cover the thing being compared",
                        shot.id
                    ),
                }
            }
            panels.push(frame);
        }
        if lost || panels.is_empty() {
            println!("FAIL after {attempts} attempt(s)  {last}");
            failed.push(last);
            drop_stale();
            continue;
        }

        // THE GUARD THIS KIND OF FRAME EXISTS FOR, and it runs before a single
        // pixel is written: a composite whose tiles came out the same is not a
        // weaker picture, it is a picture of a defect. It cannot be retried into
        // passing either, which is why it sits outside the retry loop.
        if !looks.is_empty() {
            let zversion = match &provenance {
                gallery::Provenance::Story(s) => s.version,
                gallery::Provenance::Library { .. } => 0,
            };
            if let Err(e) = gallery::check_machines_differ(shot, zversion, &looks) {
                println!("FAIL  {e}");
                failed.push(e);
                drop_stale();
                continue;
            }
        }

        let frame = if shot.machines.is_empty() {
            panels.remove(0)
        } else {
            gallery::tile(&panels, shot.tile_columns())
        };

        let native = provenance.native();
        let mut t = Taken {
            id: shot.id.clone(),
            png: png.clone(),
            provenance,
            cols,
            rows,
            cell_w,
            cell_h,
            turns: shot.turns(),
            seed: shot.seed,
            backend: shot.backend,
            face: face.describe(),
            verdict,
            attempts,
            captured_bytes,
            width: frame.width(),
            height: frame.height(),
            native,
            magnification: native.and_then(|n| shot.magnification(n)),
            unresolved_glyphs: face.unresolved(),
            machines: shot.machines.clone(),
        };
        let labelled = gallery::label(&frame, &gallery::label_lines(&t));
        t.width = labelled.width();
        t.height = labelled.height();
        match labelled.save(&png) {
            Ok(()) => {
                println!(
                    "{}x{} px  {}  {} keypress(es){}{}",
                    labelled.width(),
                    labelled.height(),
                    t.provenance.describe(),
                    t.turns,
                    t.scale_note().map(|n| format!("  {n}")).unwrap_or_default(),
                    if t.unresolved_glyphs.is_empty() {
                        String::new()
                    } else {
                        format!("  NO GLYPH ANYWHERE FOR: {}", t.unresolved_glyphs.iter().collect::<String>())
                    }
                );
                taken.push(t);
            }
            Err(e) => {
                println!("FAIL  writing {}: {e}", png.display());
                failed.push(format!("{}: writing the png: {e}", shot.id));
                drop_stale();
            }
        }
    }
    let _ = std::fs::remove_dir_all(&work);

    // A PARTIAL RUN LEAVES THE SHEET ALONE (SQ-0999). `--only` used to rewrite
    // `index.html` with just the shots it regenerated, quietly discarding the
    // other twelve — and it did not need to, because a sheet references frames by
    // FILENAME: the shot it just rewrote is already the one the existing sheet
    // points at. Nothing is lost by not touching it, and a full run still rebuilds
    // it from scratch.
    let sheet = out.join("index.html");
    if only.is_empty() {
        if let Err(e) = std::fs::write(&sheet, gallery::contact_sheet(&taken, &failed)) {
            eprintln!("gallery: writing {}: {e}", sheet.display());
        }
    } else if sheet.is_file() {
        println!("sheet   : left as it was — a partial run points at the same filenames");
    } else if let Err(e) = std::fs::write(&sheet, gallery::contact_sheet(&taken, &failed)) {
        eprintln!("gallery: writing {}: {e}", sheet.display());
    }
    let recipe = out.join("gallery.json");
    if let Err(e) = std::fs::write(&recipe, gallery::recipe_json(&taken, &manifest_path)) {
        eprintln!("gallery: writing {}: {e}", recipe.display());
    }

    println!("\n{} frame(s), {} failure(s)", taken.len(), failed.len());
    println!("proof sheet : {}", sheet.display());
    println!("recipe      : {}", recipe.display());
    println!(
        "\nResolved from the bytes lanthorn wrote, with the type rasterised from a real terminal\n\
         face — which face, at what size, is named under every frame."
    );
    if failed.is_empty() {
        std::process::ExitCode::SUCCESS
    } else {
        // A gallery with a silent hole in it is how a page ends up showing last
        // release's frame beside this release's caption.
        std::process::ExitCode::from(1)
    }
}

#[cfg(unix)]
const HELP: &str = "\
gallery — regenerate the project gallery from examples/gallery.toml

  --manifest PATH   the shot list (default: crates/app/examples/gallery.toml)
  --out DIR         where the frames go (default: target/gallery)
  --only IDS        comma list of shot ids to regenerate
  --list            print the shots that would be taken, and stop
  --font PATH       a TTF to draw the text with (default: Fira Code Nerd Font
                    Mono if it is installed, else the first candidate that
                    loads, else the bitmap master). Any face works — a glyph it
                    lacks falls back to the bitmap master — but a face whose
                    cell is not 1:2 will say so, because that is what makes a
                    half-block sample square
  --bitmap-font     force the harness's own Uni-VGA face
  --bin PATH        lanthorn binary (default: beside this example, or $LANTHORN_BIN)
  --timeout SECS    per-shot ceiling (default 90)
  --retries N       attempts per shot before giving up (default 3). The guard
                    decides, so a retry never lowers the bar

Exit status 1 means at least one shot produced no frame — a missing medium, or
a capture that negotiated the wrong graphics backend. Both are refusals rather
than warnings: a gallery of the wrong renderer is worse than a gallery with a
hole in it.";

#[cfg(not(unix))]
fn main() {
    eprintln!("gallery: a pty harness is unix-only; nothing to run on this platform.");
}
