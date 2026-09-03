//! Record asciinema casts from the pty harness (SQ-0943).
//!
//! ```sh
//! cargo build --workspace                 # the harness runs the REAL binaries
//! cargo run -p lanthorn --example cast          # the whole manifest
//! cargo run -p lanthorn --example cast -- --only machines,zork-map
//! cargo run -p lanthorn --example cast -- --list
//! cargo run -p lanthorn --example cast -- --only zork-map --gif
//! ```
//!
//! Output lands under `target/casts/`: one `.cast` per recording plus an
//! `index.md`. None of it is committed — the committed artefact is
//! `examples/casts.toml`, the recipe.
//!
//! **These casts do NOT answer the kitty capability query, on purpose.** The
//! asciinema player renders cells and SGR and drops kitty's APC graphics, so a
//! kitty recording would show no artwork at all. Unanswered, lanthorn falls back
//! to half-blocks — the same v6 pixel path resolved into `▀` with two colours,
//! which is glyphs and SGR and replays exactly. Every file this writes says so
//! in its own header. See `tests/pty_stream/cast.rs`.

#[cfg(unix)]
#[path = "../tests/pty_stream/mod.rs"]
mod pty_stream;

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    use std::path::PathBuf;
    use std::time::Duration;

    use pty_stream::cast::{self, CastManifest, Header};
    use pty_stream::driver::{self, Spec};

    let mut manifest_path: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut only: Vec<String> = Vec::new();
    let mut timeout = 90u64;
    let mut list = false;
    let mut gif = false;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let need = |i: usize| -> String {
            args.get(i + 1).cloned().unwrap_or_else(|| {
                eprintln!("cast: `{}` needs a value", args[i]);
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
            "--only" => {
                only.extend(need(i).split(',').filter(|s| !s.trim().is_empty()).map(|s| s.trim().to_string()));
                i += 1;
            }
            "--list" => list = true,
            "--gif" => gif = true,
            "--timeout" => {
                timeout = need(i).parse().unwrap_or(timeout);
                i += 1;
            }
            "-h" | "--help" => {
                println!("{HELP}");
                return std::process::ExitCode::SUCCESS;
            }
            other => {
                eprintln!("cast: unknown option `{other}`\n\n{HELP}");
                return std::process::ExitCode::from(2);
            }
        }
        i += 1;
    }

    let manifest_path = manifest_path.unwrap_or_else(CastManifest::default_path);
    let manifest = match CastManifest::load(&manifest_path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("cast: {e}");
            return std::process::ExitCode::from(2);
        }
    };
    let wanted: Vec<_> = manifest.casts.iter().filter(|c| only.is_empty() || only.iter().any(|o| o == &c.id)).collect();
    if wanted.is_empty() {
        eprintln!("cast: --only matched no recording in {}", manifest_path.display());
        return std::process::ExitCode::from(2);
    }
    if list {
        for c in &wanted {
            println!("{:<16} {:<10} {:<9}  {}", c.id, c.program.binary(), c.size, c.title);
        }
        return std::process::ExitCode::SUCCESS;
    }

    let out = out.unwrap_or_else(|| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/casts")
    });
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("cast: {}: {e}", out.display());
        return std::process::ExitCode::from(1);
    }
    let work = std::env::temp_dir().join(format!("lanthorn-cast-{}", std::process::id()));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    println!("casts   : {} recording(s) from {}", wanted.len(), manifest_path.display());
    println!("out     : {}", out.display());
    println!("terminal: answering as a terminal with NO kitty graphics — see the note in every header\n");

    let mut rows: Vec<(String, String, String, usize, f64)> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    for entry in &wanted {
        print!("  {:<16} ", entry.id);
        let _ = std::io::Write::flush(&mut std::io::stdout());

        let Some(bin) = driver::sibling_binary(entry.program.binary()) else {
            let e = format!("{}: cannot find `{}` — run `cargo build --workspace`", entry.id, entry.program.binary());
            println!("SKIP  {e}");
            failed.push(e);
            continue;
        };
        if let Some(media) = entry.media_path() {
            if !media.exists() {
                let e = format!("{}: no medium at {} (the media directories are gitignored)", entry.id, media.display());
                println!("SKIP  {e}");
                failed.push(e);
                continue;
            }
        }
        let (cols, rows_n) = match entry.size_cells() {
            Ok(v) => v,
            Err(e) => {
                println!("FAIL  {e}");
                failed.push(e);
                continue;
            }
        };
        let keys = match entry.keys() {
            Ok(k) => k,
            Err(e) => {
                println!("FAIL  {e}");
                failed.push(e);
                continue;
            }
        };

        let user_dir = work.join(&entry.id);
        let _ = std::fs::remove_dir_all(&user_dir);
        if let Err(e) = std::fs::create_dir_all(&user_dir) {
            println!("FAIL  {e}");
            failed.push(format!("{}: {e}", entry.id));
            continue;
        }
        let _ = std::fs::write(user_dir.join("config.toml"), format!("random_seed = {}\n", entry.seed));

        let mut spec = Spec::new(&bin, entry.media_path().unwrap_or_else(|| bin.clone()), &user_dir);
        spec.cols = cols;
        spec.rows = rows_n;
        spec.hide_map = !entry.show_map;
        spec.keys = keys;
        spec.timeout = Duration::from_secs(timeout);
        // THE decision this whole tool turns on. Leave the kitty query
        // unanswered and lanthorn draws through half-blocks, which the player
        // replays; answer it and the artwork silently vanishes on playback.
        spec.answer_kitty = false;
        if entry.program.is_cli() {
            spec.argv = Some(entry.cli_argv());
        } else {
            spec.extra_args = entry.args.clone();
        }

        let cap = match driver::run(spec) {
            Ok(c) => c,
            Err(e) => {
                println!("FAIL  {e}");
                failed.push(format!("{}: {e}", entry.id));
                continue;
            }
        };
        // A cast that negotiated kitty is a cast whose artwork the player will
        // drop. That is the one failure this tool exists to make impossible, so
        // it is checked rather than assumed.
        let drawn = cast::graphics_commands(&cap.bytes);
        if drawn > 0 {
            let e = format!(
                "{}: {drawn} APC `_G` graphics command(s) in the stream — an asciinema player drops \
                 those, so this recording would replay with no artwork at all",
                entry.id
            );
            println!("FAIL  {e}");
            failed.push(e);
            continue;
        }
        let text = String::from_utf8_lossy(&cap.bytes);
        let missing: Vec<&str> = entry.expect.iter().map(|s| s.as_str()).filter(|w| !text.contains(*w)).collect();
        if !missing.is_empty() {
            let e = format!(
                "{}: nothing in the recording says {} — this is not the session the manifest asked for",
                entry.id,
                missing.iter().map(|m| format!("{m:?}")).collect::<Vec<_>>().join(" or ")
            );
            println!("FAIL  {e}");
            failed.push(e);
            continue;
        }

        let header = Header {
            width: cols,
            height: rows_n,
            title: entry.title.clone(),
            idle_time_limit: entry.idle_time_limit,
            timestamp: now,
            // What the harness PRETENDED to be. Not `xterm-kitty`, because the
            // whole point is that this session had no graphics protocol.
            term: "xterm-256color".to_string(),
            note: cast::NO_KITTY_NOTE.to_string(),
        };
        let body = cast::to_cast(&cap, &header);
        if body.len() > entry.max_bytes {
            let e = format!(
                "{}: {} KiB, over the {} KiB ceiling — half-block frames are heavy, so prefer a short \
                 cast of one moment over a play session",
                entry.id,
                body.len() / 1024,
                entry.max_bytes / 1024
            );
            println!("FAIL  {e}");
            failed.push(e);
            continue;
        }
        let path = out.join(format!("{}.cast", entry.id));
        match std::fs::write(&path, &body) {
            Ok(()) => {
                let secs = cap.flushes.last().map(|f| f.at.as_secs_f64()).unwrap_or(0.0);
                print!("{:>6} KiB  {:>5.1}s  {} event(s)  {}", body.len() / 1024, secs, cap.flushes.len(), path.display());
                if gif {
                    match to_gif(&path) {
                        Ok(n) => print!("  + {} KiB gif", n / 1024),
                        Err(e) => {
                            print!("  GIF FAILED");
                            failed.push(format!("{}: {e}", entry.id));
                        }
                    }
                }
                println!();
                rows.push((entry.id.clone(), entry.title.clone(), entry.caption.clone(), body.len(), secs));
            }
            Err(e) => {
                println!("FAIL  writing {}: {e}", path.display());
                failed.push(format!("{}: {e}", entry.id));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&work);

    let index = out.join("index.md");
    if let Err(e) = std::fs::write(&index, cast::contact_sheet(&rows, &failed)) {
        eprintln!("cast: writing {}: {e}", index.display());
    }
    println!("\n{} recording(s), {} failure(s)", rows.len(), failed.len());
    println!("index   : {}", index.display());
    println!("play    : asciinema play {}/<id>.cast", out.display());
    if failed.is_empty() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::from(1)
    }
}

/// Render a recorded cast as an animated GIF beside it, and return its size.
///
/// A `.cast` is JSON and needs a player, which a GitHub README cannot run — so
/// the one artefact a README can actually SHOW is this.
///
/// **`agg` and not `svg-term`, and the reason is geometry.** `svg-term` lays
/// columns out 1.002 units apart while a box-drawing glyph is one unit wide, so
/// every cell boundary gets a hairline seam: invisible in prose, cumulative along
/// a rule, and enough to make lanthorn's own window borders render as dashes. No
/// flag adjusts it — it is baked into the emitted geometry. `agg` is asciinema's
/// own renderer and rasterises with a real font at whole-pixel cell positions, so
/// a `│` column is solid and a `─` run is continuous.
///
/// What that costs is GIF's 256-colour palette, which is why every entry in
/// `casts.toml` is a text or 16-colour session. A half-block v6 recording carries
/// two 24-bit colours per cell and must not be rendered this way; v6 is shown
/// with the gallery's stills instead.
#[cfg(unix)]
fn to_gif(cast: &std::path::Path) -> Result<u64, String> {
    let gif = cast.with_extension("gif");
    let out = std::process::Command::new("agg")
        // A terminal frame changes on a keystroke, not on a vsync, so the default
        // 30 is thirty times more frames than these recordings contain.
        .args(["--fps-cap", "10", "--font-size", "18", "--last-frame-duration", "4"])
        .arg(cast)
        .arg(&gif)
        .output()
        .map_err(|e| format!("running `agg`: {e} — install it with `brew install agg`, and only --gif needs it"))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr);
        return Err(format!("`agg` failed: {}", why.trim().lines().last().unwrap_or("no output")));
    }
    let n = std::fs::metadata(&gif).map(|m| m.len()).map_err(|e| format!("{}: {e}", gif.display()))?;
    if n == 0 {
        return Err(format!("{} is empty — agg wrote nothing", gif.display()));
    }
    Ok(n)
}

#[cfg(unix)]
const HELP: &str = "\
cast — record asciinema casts from examples/casts.toml

  --manifest PATH   the recording list (default: crates/app/examples/casts.toml)
  --out DIR         where the .cast files go (default: target/casts)
  --only IDS        comma list of recording ids
  --list            print the recordings that would be made, and stop
  --gif             also write <id>.gif — an animated GIF a README can show
                    inline (needs `agg`: brew install agg)
  --timeout SECS    per-recording ceiling (default 90)

Every recording answers the terminal queries as a terminal with NO kitty
graphics, so lanthorn draws through half-blocks and the asciinema player can
replay the artwork. A recording that emits kitty graphics anyway is refused:
the player would drop it and the cast would look like lanthorn draws nothing.";

#[cfg(not(unix))]
fn main() {
    eprintln!("cast: a pty harness is unix-only; nothing to run on this platform.");
}
