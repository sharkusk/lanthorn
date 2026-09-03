//! Compose every border flank an archive states, and write it out as a picture —
//! so Zork Zero's vines, ice, jungle and underground can be LOOKED at without
//! playing to the rooms that show them.
//!
//! ```sh
//! cargo run -p lanthorn --example border_preview -- \
//!     --archive "stories/Zork Zero Disk.image" --pictures Pic.data \
//!     --rows 900 --out /tmp/borders/mac-mono
//! ```
//!
//! The ad-hoc face of `tests/suites/v6_archive_border_sweep.rs`, in the same
//! spirit as `pty_capture`: the sweep asserts that every flank in the corpus
//! composes to a well-formed band, and this draws the same bands for when the
//! question is *"does this actually look right"* and there is no room to walk to.
//!
//! **Discovery is shared, not copied** — [`border_corpus`] is the sweep's own,
//! lifted beside it. A preview that found its flanks by a different rule would be
//! previewing art the sweep never checked, which is precisely the failure a tool
//! standing next to a test must not have. Nothing here writes down a picture id:
//! the border sets are found by shape, exactly as they are under test.
//!
//! ## What comes out
//!
//! * **One PNG per flank** in `--out`, full resolution, named for the archive,
//!   the pictures it was composed from, the side and the flank width.
//! * **`contact-sheet.png`** — every one of them tiled with a caption, which is
//!   the artefact: a whole archive's border corpus judged in one image.
//! * **An index on stdout** — what was found, how [`border::recognize`]
//!   classified it, and the file it went to.
//!
//! Every band is composited over a **checkerboard**, deliberately. A hole in a
//! border is a real defect (SQ-0698's black band) and transparent pixels left as
//! transparent read as whatever the viewer happens to paint behind them — black,
//! and indistinguishable from Zork Zero's own black masonry. Over a checker they
//! are unmistakable. The thin frame around each tile is drawn for the same
//! reason: it says where the band's last row is, which is where a pillar's foot
//! is supposed to land.

#[path = "../tests/border_corpus/mod.rs"]
mod border_corpus;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use app::graphics::{PictSource, PictureOverride};
use app::render::v6_border as border;
use app::render::v6_border::BorderArt;
use border_corpus::{Archive, flanks};
use image::{Rgba, RgbaImage};

/// The default band height, in the archive's own unit rows.
///
/// A real full-screen pane is this order of magnitude — Journey's side rules are
/// shipped as 8x900 and 16x900 bitmaps in a maximised terminal — and it is
/// comfortably taller than every picture space in the corpus (400 at the most),
/// so every flank has to be genuinely extended rather than merely cropped.
const DEFAULT_ROWS: u32 = 900;

/// Contact-sheet geometry: how tall each band is drawn, and how wide its column
/// is. The width is set by the caption — seventeen 8-pixel glyphs — because the
/// bands themselves are only 12 to 86 units wide, whatever their picture space
/// happens to state.
const TILE_H: u32 = 320;
const TILE_W: u32 = 136;
/// Three caption lines of an 8x8 font, plus a little air.
const CAPTION_H: u32 = 3 * 8 + 6;
const PAD: u32 = 6;
const MAX_COLS: u32 = 16;

const INK: Rgba<u8> = Rgba([0xe4, 0xe4, 0xea, 0xff]);
const SHEET_BG: Rgba<u8> = Rgba([0x16, 0x16, 0x1a, 0xff]);
const FRAME: Rgba<u8> = Rgba([0x6a, 0x6a, 0x78, 0xff]);

fn main() -> ExitCode {
    let mut archive: Option<PathBuf> = None;
    let mut pictures: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut rows = DEFAULT_ROWS;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let need = |i: usize| -> String {
            args.get(i + 1).cloned().unwrap_or_else(|| {
                eprintln!("border_preview: `{}` needs a value", args[i]);
                std::process::exit(2);
            })
        };
        match args[i].as_str() {
            "--archive" => {
                archive = Some(PathBuf::from(need(i)));
                i += 1;
            }
            "--pictures" => {
                pictures = Some(need(i));
                i += 1;
            }
            "--out" => {
                out = Some(PathBuf::from(need(i)));
                i += 1;
            }
            "--rows" => {
                rows = need(i).parse().unwrap_or_else(|_| {
                    eprintln!("border_preview: --rows wants a number");
                    std::process::exit(2);
                });
                i += 1;
            }
            "--help" | "-h" => {
                usage();
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("border_preview: unknown argument `{other}`");
                usage();
                return ExitCode::from(2);
            }
        }
        i += 1;
    }

    let Some(archive) = archive else {
        eprintln!("border_preview: --archive is required");
        usage();
        return ExitCode::from(2);
    };
    let out = out.unwrap_or_else(|| std::env::temp_dir().join("lanthorn-border-preview"));

    let Some((label, src)) = open(&archive, pictures.as_deref()) else {
        return ExitCode::from(1);
    };
    let Some(mut a) = Archive::wrap(label, src) else {
        eprintln!(
            "border_preview: {} carries no NATIVE Infocom archive — there is no picture space \
             and no art scale to compose a flank in",
            archive.display(),
        );
        return ExitCode::from(1);
    };
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("border_preview: cannot create {}: {e}", out.display());
        return ExitCode::from(1);
    }

    let native_h = a.native.1;
    println!(
        "{}: picture space {}x{} at scale {}x{} → art screen {}x{}, native screen {}x{}; \
         bands are {rows} rows",
        a.label,
        a.space.0,
        a.space.1,
        a.scale.0,
        a.scale.1,
        a.art_screen.0,
        a.art_screen.1,
        a.native.0,
        a.native.1,
    );

    let label_slug = slug(&a.label);
    let found = flanks(&mut a);
    let discovered = &found.flanks;
    println!(
        "  flank width stated by this archive: {:?} unit column(s); {} full-screen plate(s) state \
         no story window and so state no flank",
        found.stated_width,
        found.stateless_plates,
    );
    let mut tiles: Vec<Tile> = Vec::new();
    let mut unrecognised = 0usize;

    for f in discovered {
        let kind = border::recognize(&f.canvas, f.x0, f.x1, f.art, native_h);
        let width = f.x1 - f.x0;
        let tag = kind_tag(kind);
        let composed =
            border::flank_source(&f.canvas, &f.canvas, f.x0, f.x1, f.art, native_h, 0, rows);
        let Some(band) = composed else {
            unrecognised += 1;
            println!(
                "  {:<34}  w{:<3}  art {}..{}  {:<9}  (no composition — {})",
                f.what,
                width,
                f.art.0,
                f.art.1,
                tag,
                if kind.is_none() { "no border art here" } else { "the art already fills the band" },
            );
            continue;
        };
        let plate = on_checker(&band, 8);
        let name = format!("{label_slug}--{}--w{width}--{}.png", slug(&f.what), slug(tag));
        let path = out.join(&name);
        if let Err(e) = plate.save(&path) {
            eprintln!("border_preview: cannot write {}: {e}", path.display());
            return ExitCode::from(1);
        }
        println!(
            "  {:<34}  w{:<3}  art {}..{}  {:<9}  -> {name}",
            f.what, width, f.art.0, f.art.1, tag,
        );
        tiles.push(Tile {
            plate,
            lines: [caption(&f.what), format!("W{width} H{}..{}", f.art.0, f.art.1), tag.to_string()],
        });
    }

    if tiles.is_empty() {
        eprintln!(
            "border_preview: {} states no composable flank — {} discovered, and {} full-screen \
             plate(s) carry no story window at all, so nothing in this archive fixes a flank \
             width. Journey's four archives are all like this: their border is glyphs, not art",
            a.label,
            discovered.len(),
            found.stateless_plates,
        );
        return ExitCode::from(1);
    }

    let header = format!(
        "{}  SPACE {}X{}  SCALE {}X{}  NATIVE {}X{}  BAND {}  FLANKS {}",
        a.label.to_uppercase(),
        a.space.0,
        a.space.1,
        a.scale.0,
        a.scale.1,
        a.native.0,
        a.native.1,
        rows,
        tiles.len(),
    );
    let sheet = contact_sheet(&header, &tiles);
    let sheet_path = out.join("contact-sheet.png");
    if let Err(e) = sheet.save(&sheet_path) {
        eprintln!("border_preview: cannot write {}: {e}", sheet_path.display());
        return ExitCode::from(1);
    }
    println!(
        "\n{} flank(s) composed, {unrecognised} not; contact sheet {}x{} -> {}",
        tiles.len(),
        sheet.width(),
        sheet.height(),
        sheet_path.display(),
    );
    ExitCode::SUCCESS
}

fn usage() {
    eprintln!(
        "usage: border_preview --archive <path> [--pictures <name>] [--rows <n>] [--out <dir>]\n\
         \n\
         --archive   a loose .MG1/.EG1/.CG1, a disk image, or a story with a resource Blorb\n\
         --pictures  an archive INSIDE the medium, by name — `--pictures Pic.data` is how the\n\
         \x20           Macintosh's two-colour art is reached on a Zork Zero disk image\n\
         --rows      band height in the archive's own unit rows (default {DEFAULT_ROWS})\n\
         --out       directory for the per-flank PNGs and contact-sheet.png",
    );
}

/// Open an archive by the same doors a player has.
///
/// `--pictures` is the tier-3 override, which also reaches a name that exists
/// only inside a disk image. Without it the medium's own archive answers (an
/// `.adf`'s `Pic.data`, the Macintosh's colour `CPic.data`, a resource Blorb),
/// and failing that the path is taken to BE a loose archive and named absolutely,
/// which is the same door with the file spelled out.
fn open(path: &Path, pictures: Option<&str>) -> Option<(String, PictSource)> {
    let base =
        path.file_name().and_then(|s| s.to_str()).unwrap_or("archive").to_string();
    let dir = border_corpus::scratch();
    let _ = std::fs::create_dir_all(&dir);

    if let Some(name) = pictures {
        let over = PictureOverride::resolve_with_session(path, &dir, Some(name));
        if !matches!(over, PictureOverride::Loaded { .. }) {
            eprintln!(
                "border_preview: `{name}` is not a usable picture archive, on disk beside {} or \
                 inside it ({over:?})",
                path.display(),
            );
            return None;
        }
        return Some((format!("{base} [{name}]"), PictSource::resolve_with_override(path, over, None)));
    }

    let src = PictSource::resolve(path, None);
    if src.native_std_window().is_some() {
        return Some((base, src));
    }

    let abs = std::fs::canonicalize(path).ok()?;
    let over = PictureOverride::resolve_with_session(path, &dir, abs.to_str());
    if !matches!(over, PictureOverride::Loaded { .. }) {
        eprintln!(
            "border_preview: {} is neither a medium carrying native art nor an Infocom picture \
             archive ({over:?})",
            path.display(),
        );
        return None;
    }
    Some((base, PictSource::resolve_with_override(path, over, None)))
}

// ── Drawing ──────────────────────────────────────────────────────────────────

/// One composed flank, ready to tile.
struct Tile {
    plate: RgbaImage,
    lines: [String; 3],
}

fn kind_tag(kind: Option<BorderArt>) -> &'static str {
    match kind {
        Some(BorderArt::ZorkZeroPillars) => "ZZPILLARS",
        Some(BorderArt::ShogunSinglePiece) => "SLAB",
        Some(BorderArt::ArthurPoles) => "POLES",
        None => "NONE",
    }
}

/// The discovery's own phrasing, squeezed to fit a tile: `plate 25, left flank
/// 86 wide` becomes `P25 L86`, `strip 5 over column 166` becomes `S5/C166`.
fn caption(what: &str) -> String {
    what.replace("plate ", "P")
        .replace(", left flank ", " L")
        .replace(", right flank ", " R")
        .replace(" wide", "")
        .replace("strip ", "S")
        .replace(" over column ", "/C")
        .to_uppercase()
}

/// A filesystem-safe, human-readable form of a label.
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Composite `band` over a checkerboard of `square`-pixel cells, so a
/// transparent row reads as a hole rather than as black artwork.
fn on_checker(band: &RgbaImage, square: u32) -> RgbaImage {
    let mut out = RgbaImage::new(band.width().max(1), band.height().max(1));
    for y in 0..out.height() {
        for x in 0..out.width() {
            let light = (x / square + y / square).is_multiple_of(2);
            let bg: [u8; 3] = if light { [0x3c, 0x3c, 0x44] } else { [0x28, 0x28, 0x2e] };
            let p = band.get_pixel(x.min(band.width() - 1), y.min(band.height() - 1)).0;
            let a = u32::from(p[3]);
            let mix = |c: u8, b: u8| ((u32::from(c) * a + u32::from(b) * (255 - a)) / 255) as u8;
            out.put_pixel(
                x,
                y,
                Rgba([mix(p[0], bg[0]), mix(p[1], bg[1]), mix(p[2], bg[2]), 0xff]),
            );
        }
    }
    out
}

/// Box-average `src` down to `w x h`.
///
/// Averaging rather than sampling on purpose: a subsample of a 900-row band into
/// 320 drops two rows in three, and the row it drops could be the one carrying a
/// pillar's foot. An average keeps every source row in the answer, so a thin
/// feature survives as a lighter smear instead of vanishing.
fn box_down(src: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    let (w, h) = (w.max(1), h.max(1));
    let mut out = RgbaImage::new(w, h);
    for y in 0..h {
        let y0 = y * src.height() / h;
        let y1 = ((y + 1) * src.height() / h).max(y0 + 1).min(src.height());
        for x in 0..w {
            let x0 = x * src.width() / w;
            let x1 = ((x + 1) * src.width() / w).max(x0 + 1).min(src.width());
            let (mut acc, mut n) = ([0u32; 4], 0u32);
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = src.get_pixel(sx, sy).0;
                    for (a, c) in acc.iter_mut().zip(p) {
                        *a += u32::from(c);
                    }
                    n += 1;
                }
            }
            let n = n.max(1);
            out.put_pixel(
                x,
                y,
                Rgba([
                    (acc[0] / n) as u8,
                    (acc[1] / n) as u8,
                    (acc[2] / n) as u8,
                    (acc[3] / n) as u8,
                ]),
            );
        }
    }
    out
}

/// Draw `text` at `(x, y)` in the 8x8 bitmap font the renderer already carries,
/// each pixel blown up `scale` times.
fn text(dst: &mut RgbaImage, x: u32, y: u32, s: &str, scale: u32, ink: Rgba<u8>) {
    use font8x8::UnicodeFonts;
    for (n, ch) in s.chars().enumerate() {
        let Some(bits) = font8x8::BASIC_FONTS.get(ch) else { continue };
        let cx = x + n as u32 * 8 * scale;
        for (row, byte) in bits.iter().enumerate() {
            for col in 0..8u32 {
                if byte & (1 << col) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let (px, py) =
                            (cx + col * scale + dx, y + row as u32 * scale + dy);
                        if px < dst.width() && py < dst.height() {
                            dst.put_pixel(px, py, ink);
                        }
                    }
                }
            }
        }
    }
}

/// A hollow one-pixel rectangle — the band's own edge, so the row a foot is
/// meant to land on is visible.
fn frame(dst: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32) {
    for dx in 0..w {
        for py in [y, y + h - 1] {
            if x + dx < dst.width() && py < dst.height() {
                dst.put_pixel(x + dx, py, FRAME);
            }
        }
    }
    for dy in 0..h {
        for px in [x, x + w - 1] {
            if px < dst.width() && y + dy < dst.height() {
                dst.put_pixel(px, y + dy, FRAME);
            }
        }
    }
}

/// Tile every composed flank into one image, captioned, under a header naming
/// the archive and its geometry.
///
/// **The tiles are not to aspect, deliberately.** A band is drawn at its own
/// width — 46 or 86 unit columns, 1:1 — and squeezed on the HEIGHT alone, 900
/// rows into 320. At true aspect an 86x900 flank is a thirty-pixel thread on the
/// sheet and its masonry is unreadable, which defeats the purpose of a sheet.
/// What the vertical squeeze preserves is exactly what one is looking for here:
/// where the repeats fall, whether a stride is uniform, and whether the foot is
/// on the last row. The per-flank PNGs beside it are 1:1 in both axes.
fn contact_sheet(header: &str, tiles: &[Tile]) -> RgbaImage {
    let cols = (tiles.len() as u32).clamp(1, MAX_COLS);
    let rows = tiles.len().div_ceil(cols as usize) as u32;
    let cell_h = TILE_H + 4 + CAPTION_H + PAD;
    let head_h = 16 + 2 * PAD;
    let mut sheet = RgbaImage::from_pixel(cols * TILE_W, head_h + rows * cell_h, SHEET_BG);
    text(&mut sheet, PAD, PAD, header, 2, INK);

    for (n, t) in tiles.iter().enumerate() {
        let (cx, cy) = (n as u32 % cols, n as u32 / cols);
        let cell_x = cx * TILE_W;
        let cell_y = head_h + cy * cell_h;
        // Full width, compressed height — see `contact_sheet`'s doc.
        let bw = t.plate.width().clamp(1, TILE_W - 2 * PAD);
        let small = box_down(&t.plate, bw, TILE_H);
        let bx = cell_x + (TILE_W - bw) / 2;
        border_corpus::blit(&mut sheet, &small, bx, cell_y);
        frame(&mut sheet, bx.saturating_sub(1), cell_y, bw + 2, TILE_H + 2);
        for (line, s) in t.lines.iter().enumerate() {
            text(&mut sheet, cell_x + 3, cell_y + TILE_H + 6 + line as u32 * 8, s, 1, INK);
        }
    }
    sheet
}
