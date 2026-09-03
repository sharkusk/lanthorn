//! What a v6 side flank is MADE OF — the measurement `v6_border::recognize` reduces
//! to one enum, printed in full (SQ-0899).
//!
//! `recognize` decides which of three per-title recipes extends a flank down a pane
//! taller than the artwork, and it decides from the column's own shape. When it
//! decides wrong the player sees the frame's ornament, capital or banner reprinted
//! down the side, and the enum alone never says why. This prints the shape the
//! decision is made from:
//!
//! * the **span profile** — each row's painted width, run-length encoded down the
//!   column. This is what [`v6_border`]'s statistics are computed over, and reading
//!   it is the difference between "Arthur classified as Zork Zero" and knowing that
//!   his three ornaments are thirty-four rows each where a capital is one run of 82;
//! * the **classification**, with the two numbers it turns on: the art's inset from
//!   the screen's edges, and its narrowest ÷ widest painted row;
//! * the **extended** profile — the source actually composed for a pane taller than
//!   the art. A band that appears only here is a reprint, and that is the defect
//!   class this exists to catch. Compare it against the line above: extending a
//!   flank should lengthen its shaft and add nothing.
//!
//! ```sh
//! cargo run -q -p lanthorn --example flank_shape -- --story stories/Arthur.po --keys n
//! cargo run -q -p lanthorn --example flank_shape -- --story stories/zork0-r393-s890714.z6 \
//!     --archive stories/zork0.cg1
//! cargo run -q -p lanthorn --example flank_shape -- --all
//! cargo run -q -p lanthorn --example flank_shape -- --story stories/InfocomMasterpieces.img \
//!     --entry arthur --keys n
//! ```
//!
//! `--entry <n|name>` says WHICH story, on a volume holding several — a 1-based
//! position in the browser's list or enough of a name, matched exactly as
//! `lanthorn --story` matches it (SQ-1078). Without it a compilation disc
//! measures whatever the mount prefers, which on `InfocomMasterpieces.img` is
//! Zork Zero whichever flank you meant to look at.
//!
//! `--archive` boots against a named picture archive — the tier-3 door the player
//! uses to pick a rendition — because renditions of one title do not agree on the
//! shape: Zork Zero's CGA capital is dithered and its span oscillates the whole way
//! down, where its MCGA capital is one clean run.
//!
//! `--cols N` is how many native columns in from each edge to measure (default 17,
//! which is the whole flank on every title that has poles; Shogun's slab is wider).
//! `--rows N` is the height the extension is composed to (default 586, what a 129x60
//! pane at an 8x18 cell asks of a 560x384 screen).
//!
//! This measures the flank a LIVE frame draws. The composed scene borders — Zork
//! Zero's underground and jungle, which no affordable play session reaches — are
//! swept instead by `crates/app/tests/suites/v6_archive_border_sweep.rs`, over all
//! sixty-eight flanks the corpus states.

use app::engine::Engine;
use app::render::v6_border;
use app::render::v6_layout as v6;
use app::session::{GameSession, InputKind};

/// Titles that draw a flank at all, with the key their intro asks for.
const CORPUS: &[(&str, &str)] = &[
    ("stories/Arthur.po", "n"),
    ("stories/Arthur - The Quest for Excalibur.adf", "n"),
    ("stories/arthur-r74-s890714.z6", "n"),
    ("stories/zork0-r393-s890714.z6", ""),
    ("stories/Zork Zero - The Revenge of Megaboz.adf", ""),
    ("stories/shogun-r322-s890706.z6", ""),
    ("stories/James Clavell's Shogun.adf", ""),
];

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let get = |flag: &str| a.iter().position(|v| v == flag).and_then(|i| a.get(i + 1)).cloned();
    let cols: u32 = get("--cols").and_then(|v| v.parse().ok()).unwrap_or(17);
    let rows: u32 = get("--rows").and_then(|v| v.parse().ok()).unwrap_or(586);
    let taps: usize = get("--taps").and_then(|v| v.parse().ok()).unwrap_or(6);
    let archive = get("--archive");
    let keys = get("--keys");
    let entry = get("--entry");

    let targets: Vec<(String, String)> = if a.iter().any(|v| v == "--all") {
        CORPUS.iter().map(|(s, k)| (s.to_string(), keys.clone().unwrap_or_else(|| k.to_string()))).collect()
    } else if let Some(s) = get("--story") {
        vec![(s, keys.unwrap_or_default())]
    } else {
        eprintln!("flank_shape: pass --story <file> or --all");
        std::process::exit(2);
    };

    for (path, k) in targets {
        println!("═══ {path}");
        match shape(&path, entry.as_deref(), &k, archive.as_deref(), taps, cols, rows) {
            Ok(()) => {}
            Err(e) => println!("  SKIP: {e}\n"),
        }
    }
}

/// Each row's painted span over `[x0, x1)`, run-length encoded — `WIDTHxROWS`.
fn profile(img: &image::RgbaImage, x0: u32, x1: u32, rows: std::ops::Range<u32>) -> String {
    let mut runs: Vec<(u32, u32)> = vec![];
    for y in rows.start..rows.end.min(img.height()) {
        let (mut first, mut last) = (None, 0u32);
        for x in x0..x1.min(img.width()) {
            if img.get_pixel(x, y)[3] >= 128 {
                first.get_or_insert(x);
                last = x;
            }
        }
        let span = first.map_or(0, |f| last - f + 1);
        match runs.last_mut() {
            Some((s, n)) if *s == span => *n += 1,
            _ => runs.push((span, 1)),
        }
    }
    runs.iter().map(|(s, n)| format!("{s}x{n}")).collect::<Vec<_>>().join(" ")
}

#[allow(clippy::too_many_arguments)]
fn shape(path: &str, entry: Option<&str>, keys: &str, archive: Option<&str>, taps: usize, cols: u32, rows: u32) -> Result<(), String> {
    let p = std::path::Path::new(path);
    // `--entry` names WHICH story, on a volume that holds several: a 1-based
    // position or enough of a name, by the rule `lanthorn --story` matches
    // (SQ-1078). Without it a compilation disc measures whatever the mount
    // prefers, which on `InfocomMasterpieces.img` is Zork Zero whichever flank
    // you meant to look at.
    let entry = match entry.map(|w| app::story_pick::entry_on(p, w)) {
        Some(Ok(e)) => e,
        Some(Err(msg)) => return Err(msg),
        None => None,
    };
    let entry = entry.as_deref();
    let (loaded, medium) =
        app::hints::load_mounted_story_from(p, entry).map_err(|e| format!("{e:?}"))?;
    let bytes = loaded.bytes().to_vec();
    if bytes.first() != Some(&6) {
        return Err("not a v6 story".into());
    }
    // The same two boots `startup.rs` offers: the medium's own artwork, or a named
    // archive whose flavour picks the machine (SQ-0790 tier 3).
    let (mut picts, profile_) = match archive {
        Some(a) => {
            let raw = std::fs::read(a).map_err(|e| format!("{a}: {e}"))?;
            let pics = blorb::infocom_pics::InfocomPics::parse(raw).map_err(|e| format!("{a}: {e:?}"))?;
            let prof = app::interpreter::InterpreterProfile::for_art_flavour(pics.flavour());
            (app::graphics::PictSource::from_native(pics), prof)
        }
        None => (
            // The ENTRY rides along: a compilation keeps each game's plates in
            // its own folder, and pairing the artwork with the volume instead of
            // with the story drew Zork Zero's plates for every game on the disc
            // (SQ-0876, the artwork half of `native_disk_font`'s case).
            app::graphics::PictSource::resolve(p, entry),
            app::interpreter::InterpreterProfile::resolve(p, None, None, medium),
        ),
    };
    zvm::screen::set_palette(profile_.palette());
    let dims = picts.all_pict_dims();
    // SQ-1022. This dropped the CELL and the named-archive link; both ride along
    // now, and neither is this file's business to remember.
    let boot = app::machine_boot::MachineBoot::resolve(
        profile_,
        &picts,
        None,
        profile_.interpreter_number(),
        profile_.default_colours(),
        true,
        app::native_font::FaceSet::none(),
    );
    let std_win = boot.screen_px;
    println!(
        "  booted as {:?}  ·  release {} serial {}  ·  screen {}{}",
        profile_,
        u16::from_be_bytes([bytes[2], bytes[3]]),
        String::from_utf8_lossy(&bytes[0x12..0x18]),
        std_win.map_or("(none)".into(), |(w, h)| format!("{w}x{h}")),
        archive.map(|a| format!("  ·  archive {a}")).unwrap_or_default(),
    );
    let mut s = GameSession::new_for_machine(bytes, true, false, false, dims, None, None, &boot)
    .map_err(|e| format!("boot: {e:?}"))?;
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..taps {
        let t = match s.pending_input() {
            InputKind::Line => s.submit("").transcript,
            InputKind::Char => s.submit_char(keys.bytes().next().unwrap_or(13)).transcript,
            InputKind::Event => s.submit("").transcript,
        };
        if t.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
    }

    let model = s.screen();
    let app::engine::WinNode::Layered(items) = &model.root else {
        return Err("not a Layered v6 frame".into());
    };
    let native = v6::native_extent(items.as_slice(), &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = v6::classify_windows(items.as_slice(), zvm::screen::V6Cell::DEFAULT);
    let gfx = v6::build_graphics_canvas(&layout.chrome, native);
    let (w, h) = (native.0 as u32, native.1 as u32);
    println!("  native {w}x{h}, measuring {cols} columns in from each edge");
    for (side, x0, x1) in [("left ", 0, cols.min(w)), ("right", w.saturating_sub(cols), w)] {
        let art = v6_border::art_extent(&gfx, x0, x1);
        if art.1 <= art.0 {
            println!("    {side}: no art in these columns");
            continue;
        }
        let kind = v6_border::recognize(&gfx, x0, x1, art, h);
        let inset = art.0 + h.saturating_sub(art.1);
        println!("    {side} art rows {art:?}  inset {inset}  -> {kind:?}");
        println!("      art      {}", profile(&gfx, x0, x1, art.0..art.1));
        {
            // SQ-1063: the three-section reading — banner / middle / footer.
            let strip = {
                let w = x1.min(gfx.width()).saturating_sub(x0);
                let mut im = image::RgbaImage::new(w.max(1), h);
                for y in 0..h.min(gfx.height()) {
                    for x in 0..w {
                        im.put_pixel(x, y, *gfx.get_pixel(x0 + x, y));
                    }
                }
                im
            };
            match v6_border::flank_sections(&strip, art.0, art.1) {
                Some(sec) => {
                    let mh = sec.middle_end - sec.middle_top;
                    let unit = (mh / sec.period).max(1) * sec.period;
                    println!(
                        "      sections banner {}..{} ({} rows) · middle {}..{} (period {}, repeats {} rows) · footer {}..{} ({} rows)",
                        art.0, sec.middle_top, sec.middle_top - art.0,
                        sec.middle_top, sec.middle_end, sec.period, unit,
                        sec.middle_end, art.1, art.1 - sec.middle_end
                    )
                }
                None => println!("      sections NO MIDDLE — nothing in this column repeats"),
            }
        }
        match v6_border::flank_source(&gfx, &gfx, x0, x1, art, h, 0, rows) {
            Some(img) => {
                let ext = profile(&img, 0, img.width(), 0..img.height());
                println!("      to {rows:<5}{ext}");
            }
            None => println!("      to {rows:<5}(no extension — the art already covers it)"),
        }
    }
    println!();
    Ok(())
}
