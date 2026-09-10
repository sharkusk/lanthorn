//! The emitted-stream harness (SQ-0762): drive the real lanthorn binary under a
//! pty and read back the escape bytes it actually wrote to the terminal.
//!
//! Every other harness in this repo asserts on lanthorn's INTERNAL model — the
//! cell buffer, the layout records, the placement list. None of them can see the
//! stream, so a defect that is correct in the model and wrong on screen is
//! invisible to all of them. This one closes that gap, and its whole value is in
//! being able to say which of two indistinguishable-on-screen things happened:
//! an image was placed over those rows, or a background colour was painted into
//! those cells. Those are different bugs with different fixes.
//!
//! Four parts:
//!   * [`driver`] (unix only) — the pty, the terminal-query answers, the keys;
//!   * [`decode`] (portable) — bytes to named sequences and a screen model;
//!   * [`oracle`] (portable) — the same bytes through a real terminal emulator,
//!     so the answer above has a second, independently-authored witness (SQ-0764);
//!   * [`raster`] (portable) — that resolved screen drawn as a PNG, so a frame
//!     can be LOOKED at instead of reproduced (SQ-0775). Layout, art placement
//!     and colour only; read its module docs before trusting it as a screenshot;
//!   * the report below — the model rendered for a human to read.
//!
//! Ad hoc: `cargo run -p app --example pty_capture -- --help`.

// Each consumer (the test, the example) uses a different slice of this module,
// and neither should have to name the other's parts to keep the build quiet.
#![allow(dead_code)]

/// asciinema casts (SQ-0943): the same capture, serialised as timing plus
/// bytes. Unix-gated with [`driver`], whose `Capture` it is a view of.
#[cfg(unix)]
pub mod cast;
pub mod decode;
#[cfg(unix)]
pub mod driver;
/// The gallery (SQ-0942): the same capture, drawn to be LOOKED at. Unix-gated
/// with [`driver`], because a gallery of frames nobody captured is nothing.
#[cfg(unix)]
pub mod gallery;
/// SQ-0976: undo the kitty protocol's `o=z` so [`oracle`]'s terminal core — which
/// links no zlib at all — can still read a compressed upload. Portable, because
/// bytes captured on unix are resolved everywhere.
pub mod inflate;
pub mod oracle;
pub mod raster;

// ── The terminal this harness claims to be ───────────────────────────────────
//
// PORTABLE ON PURPOSE, and that is the whole of SQ-0966. These live beside the
// modules rather than inside `driver` because `driver` is unix-only while every
// consumer of them is not: `oracle::resolve` must be handed them on any platform
// or its picture is of a different terminal than the one lanthorn was answering.
// While they sat in `driver`, three portable modules of `pty_oracle.rs` imported
// a unix-only module for two constants and the `pty` test binary would not
// COMPILE on Windows — a split that `mod.rs` documented correctly and one import
// crossed.

/// The default foreground this harness answers **OSC 10** with.
///
/// A capture only carries the app→terminal direction, so the reply we type back
/// is not in the bytes the oracle resolves. That makes these two constants the
/// only record of what terminal the app thought it was talking to, and
/// `oracle::resolve` has to be handed them or its picture is of a different
/// terminal than the one lanthorn was answering (SQ-0929).
pub const ANSWERED_FG: [u8; 3] = [0xC0, 0xC0, 0xC0];

/// The default background this harness answers **OSC 11** with.
///
/// It matches the emulator palette's own entry 0, which is what an unpainted cell
/// is filled with — and that agreement is the whole point. While it did not
/// agree, the harness told lanthorn "my background is #000000", lanthorn dutifully
/// painted its chrome cells #000000, and the rasteriser drew every unpainted cell
/// #1D1F21. The result was a black rectangle floating on grey in every v6 capture:
/// a defect that existed nowhere but in the instrument, and that cost a long
/// investigation of the renderer before anyone read this file.
pub const ANSWERED_BG: [u8; 3] = [0x1D, 0x1F, 0x21];


use std::collections::BTreeMap;
use std::fmt::Write as _;

use decode::{Color, Term};

/// Replay a capture's bytes through the decoder, attributing every cell to the
/// flush that wrote it.
#[cfg(unix)]
pub fn decode_capture(cap: &driver::Capture) -> Term {
    let mut term = Term::new(cap.spec.cols, cap.spec.rows);
    for (i, f) in cap.flushes.iter().enumerate() {
        term.set_flush(i);
        term.feed(&cap.bytes[f.offset..f.offset + f.len]);
    }
    term
}

/// The full report: what ran, what protocol it negotiated, what it uploaded,
/// where each image landed, and which cells were painted instead.
#[cfg(unix)]
pub fn report(cap: &driver::Capture, term: &Term) -> String {
    let mut s = String::new();
    let neg = cap.negotiated();
    let _ = writeln!(s, "=== lanthorn emitted-stream capture (SQ-0762) ===");
    let _ = writeln!(s, "binary   : {}", cap.spec.bin.display());
    let _ = writeln!(s, "story    : {}", cap.spec.story.display());
    let _ = writeln!(
        s,
        "terminal : {}x{} cells, {}x{} px per cell (as answered to CSI 16 t)",
        cap.spec.cols, cap.spec.rows, cap.spec.cell_w, cap.spec.cell_h
    );
    let _ = writeln!(s, "map pane : {}", if cap.spec.hide_map { "hidden" } else { "shown" });
    let _ = writeln!(s, "keys     : {}", describe_keys(&cap.spec.keys));
    let _ = writeln!(
        s,
        "captured : {} bytes in {} flush(es) over {:.1}s{}",
        thousands(cap.bytes.len()),
        cap.flushes.len(),
        cap.duration.as_secs_f32(),
        if cap.timed_out { "  (TIMED OUT)" } else { "" }
    );

    let _ = writeln!(s, "\n--- graphics protocol ---");
    for a in &cap.answered {
        let _ = writeln!(s, "  +{:>6.2}s  {:<28} answered {}", a.at.as_secs_f32(), a.query, printable(&a.sent));
    }
    if cap.answered.is_empty() {
        let _ = writeln!(s, "  (lanthorn asked nothing — it never probed the terminal)");
    }
    let _ = writeln!(s, "  VERDICT: {}", neg.explain());

    let _ = writeln!(s, "\n--- flushes (a burst of output with no pause inside it) ---");
    let _ = writeln!(s, "    #      t     bytes    cells  graphics");
    for (i, f) in cap.flushes.iter().enumerate() {
        let apc: Vec<String> =
            term.apc.iter().filter(|a| a.flush == i).map(|a| a.describe().trim_end().to_string()).collect();
        let _ = writeln!(
            s,
            "  {:>3}  {:>6.2}s {:>9} {:>8}  {}",
            i,
            f.at.as_secs_f32(),
            thousands(f.len),
            term.flush_prints.get(i).copied().unwrap_or(0),
            if apc.is_empty() { "-".to_string() } else { apc.join(" | ") }
        );
    }

    s.push_str(&screen_report(term));
    s.push_str("\n--- final screen, glyphs (▒ = kitty placeholder, · = never written) ---\n");
    for row in 0..term.rows {
        let _ = writeln!(s, "  {row:>3} |{}|", term.row_text(row, 0..term.cols));
    }
    s
}

/// The portable half of the report: everything readable off the screen model.
pub fn screen_report(term: &Term) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "\n--- other decoded sequences ---");
    let _ = writeln!(
        s,
        "  {} cursor moves, {} SGR changes, {} cells printed, {} erase(s), {} mode change(s)",
        term.cup_count,
        term.sgr_count,
        term.printed_cells,
        term.erases.len(),
        term.modes.len()
    );
    for (flush, what) in term.modes.iter().take(12) {
        let _ = writeln!(s, "  flush {flush:<4} {what}");
    }
    for (flush, what) in term.erases.iter().take(12) {
        let _ = writeln!(s, "  flush {flush:<4} {what}");
    }
    if !term.unknown.is_empty() {
        let mut distinct: Vec<&String> = term.unknown.iter().map(|(_, w)| w).collect();
        distinct.sort();
        distinct.dedup();
        let _ = writeln!(
            s,
            "  {} sequence(s) with no screen effect modelled, {} distinct: {}",
            term.unknown.len(),
            distinct.len(),
            distinct.iter().take(10).map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        );
    }

    let _ = writeln!(s, "\n--- kitty APC graphics commands ---");
    if term.apc.is_empty() {
        let _ = writeln!(s, "  (none — nothing was uploaded or placed)");
    }
    for a in &term.apc {
        let _ = writeln!(s, "  flush {:<4} {}", a.flush, a.describe());
    }

    let placements = term.placements();
    let _ = writeln!(s, "\n--- virtual placements on the final screen ---");
    if placements.is_empty() {
        let _ = writeln!(s, "  (none — no U+10EEEE placeholder cells were printed)");
    }
    for p in &placements {
        // The placeholder's foreground carries only the LOW 24 BITS of the image
        // id (the protocol's own encoding — lanthorn drops the top byte into the
        // 24-bit colour), so an upload is matched back to its placement by those
        // bits, not by equality.
        let upload = term.apc.iter().find(|a| {
            a.params
                .split(',')
                .find_map(|kv| kv.strip_prefix("i=")?.parse::<u32>().ok())
                .is_some_and(|id| id & 0x00ff_ffff == p.image_id)
        });
        let _ = writeln!(s, "  {}", p.describe());
        if let Some(a) = upload {
            let _ = writeln!(s, "    uploaded in flush {} as {}", a.flush, a.describe().trim_end());
        } else {
            let _ = writeln!(s, "    no upload in this capture — placed from an image sent before it started");
        }
        let ragged = p.rows.values().any(|&(l, r)| (l, r) != (p.left, p.right));
        if ragged {
            let _ = writeln!(s, "    ragged — per-row column spans:");
            for (row, (l, r)) in &p.rows {
                let _ = writeln!(s, "      row {row:>3}: cols {l}..{r}");
            }
        }
    }

    let _ = writeln!(s, "\n--- backgrounds painted, by colour ---");
    let mut by_bg: BTreeMap<Color, (usize, u16, u16)> = BTreeMap::new();
    for row in 0..term.rows {
        for col in 0..term.cols {
            let c = term.cell(row, col);
            if !c.written {
                continue;
            }
            let e = by_bg.entry(c.style.bg).or_insert((0, row, row));
            e.0 += 1;
            e.1 = e.1.min(row);
            e.2 = e.2.max(row);
        }
    }
    for (bg, (n, top, bottom)) in &by_bg {
        let _ = writeln!(s, "  {:<10} {:>6} cells, rows {}..{}", bg.label(), n, top, bottom);
    }

    s.push_str(&background_map(term));
    // A one-cell-wide divider strip says nothing a row of the background map does
    // not; the row-by-row table is for art wide enough to have a column of its own.
    for p in placements.iter().filter(|p| p.right - p.left >= 3) {
        s.push_str(&column_table(term, p));
    }
    s.push_str(&overrun_finding(term));
    s
}

/// Every row of the final screen as its background runs, with the cells an image
/// covers marked. This is the section that shows a fill running past the art it
/// belongs to: the colour continues down the rows and the `(image)` marker stops.
pub fn background_map(term: &Term) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "\n--- background map (each row's SGR background runs; (image) = kitty placeholder cells) ---");
    for row in 0..term.rows {
        let runs = term.bg_runs(row, 0..term.cols);
        let mut line = String::new();
        for r in &runs {
            if r.bg == Color::Default && !r.all_placeholder {
                continue;
            }
            let _ = write!(
                line,
                "{}[{}..{}]{}  ",
                r.bg.label(),
                r.col0,
                r.col1,
                if r.all_placeholder { "(image)" } else { "" }
            );
        }
        if line.is_empty() {
            line.push_str("(nothing but the terminal default)");
        }
        let _ = writeln!(s, "  {row:>3} | {}", line.trim_end());
    }
    s
}

/// A row-by-row reading of one placement's own columns, from two rows above it to
/// the bottom of the screen. This is the table that answers the question the
/// harness exists for: for each row, is the colour there an IMAGE or is it PAINT?
fn column_table(term: &Term, p: &decode::Placement) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "\n--- image {} column-by-row (cols {}..{}) ---",
        p.image_id, p.left, p.right
    );
    let _ = writeln!(s, "  row | covered by image | background across those columns");
    let top = p.top.saturating_sub(2);
    for row in top..term.rows {
        let runs = term.bg_runs(row, p.left..p.right + 1);
        let covered = (p.left..=p.right).filter(|&c| term.cell(row, c).image_id() == Some(p.image_id)).count();
        let width = (p.right - p.left + 1) as usize;
        let cover = match covered {
            0 => "        no       ".to_string(),
            n if n == width => "       yes       ".to_string(),
            n => format!("  {n:>4}/{width} cells  "),
        };
        let bgs: Vec<String> = runs
            .iter()
            .map(|r| {
                format!("{} cols {}..{}{}", r.bg.label(), r.col0, r.col1, if r.all_placeholder { " (image)" } else { "" })
            })
            .collect();
        let _ = writeln!(s, "  {row:>3} |{cover}| {}", bgs.join("  "));
    }
    s
}

/// The `(row, col)` cells one colour reached, split by how it got there: painted
/// into ordinary cells, or showing under an image's placeholder cells.
type PaintedAndCovered = (Vec<(u16, u16)>, Vec<(u16, u16)>);

/// The whole reason this harness exists, stated per colour: on the rows where a
/// colour reaches the terminal, did it arrive as an IMAGE (kitty placeholder
/// cells) or as PAINT (an SGR background on ordinary cells)? Those are different
/// bugs with different fixes, and only the emitted stream tells them apart.
pub fn overrun_finding(term: &Term) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "\n--- finding: for each colour, which rows are image and which are paint ---");
    let placements = term.placements();

    let mut colours: BTreeMap<Color, PaintedAndCovered> = BTreeMap::new();
    for row in 0..term.rows {
        for col in 0..term.cols {
            let c = term.cell(row, col);
            if !c.written || c.style.bg == Color::Default {
                continue;
            }
            let e = colours.entry(c.style.bg).or_default();
            if c.is_placeholder() { &mut e.1 } else { &mut e.0 }.push((row, col));
        }
    }
    if colours.is_empty() {
        let _ = writeln!(s, "  nothing but the terminal default background was ever set");
        return s;
    }

    for (bg, (paint, image)) in &colours {
        let _ = writeln!(
            s,
            "  {}: {} painted cell(s), {} cell(s) under an image",
            bg.label(),
            paint.len(),
            image.len()
        );
        for (label, cells) in [("PAINT (SGR background on cells)", paint), ("IMAGE (kitty placeholders)", image)] {
            if cells.is_empty() {
                continue;
            }
            let rows: Vec<u16> = {
                let mut r: Vec<u16> = cells.iter().map(|(row, _)| *row).collect();
                r.dedup();
                r
            };
            let left = cells.iter().map(|(_, c)| *c).min().unwrap_or(0);
            let right = cells.iter().map(|(_, c)| *c).max().unwrap_or(0);
            let bands: Vec<String> =
                ranges(&rows).iter().map(|(a, b)| if a == b { format!("{a}") } else { format!("{a}..{b}") }).collect();
            let _ = writeln!(s, "    {label:<32} rows {}  within cols {left}..{right}", bands.join(", "));
        }
        // The decisive sentence: a colour that is paint on rows no placement
        // reaches is a fill running past its art, not an image placed too tall.
        if !paint.is_empty() {
            let left = paint.iter().map(|(_, c)| *c).min().unwrap_or(0);
            let right = paint.iter().map(|(_, c)| *c).max().unwrap_or(0);
            let overlapping: Vec<&decode::Placement> =
                placements.iter().filter(|p| p.left <= right && p.right >= left).collect();
            let uncovered: Vec<u16> = {
                let mut r: Vec<u16> = paint
                    .iter()
                    .filter(|(row, col)| {
                        !placements.iter().any(|p| {
                            (p.top..=p.bottom).contains(row) && (p.left..=p.right).contains(col)
                        })
                    })
                    .map(|(row, _)| *row)
                    .collect();
                r.dedup();
                r
            };
            if uncovered.is_empty() {
                let _ = writeln!(s, "    => every painted cell also sits inside some placement's rect");
            } else {
                let bands: Vec<String> = ranges(&uncovered)
                    .iter()
                    .map(|(a, b)| if a == b { format!("{a}") } else { format!("{a}..{b}") })
                    .collect();
                let _ = writeln!(
                    s,
                    "    => rows {} are PAINTED CELLS (SGR background spans), NOT a placement rect: \
                     no kitty placement covers them.",
                    bands.join(", ")
                );
                if overlapping.is_empty() {
                    let _ = writeln!(s, "       no placement shares those columns at all");
                } else {
                    let _ = writeln!(s, "       placements sharing those columns, and where they stop:");
                    for p in &overlapping {
                        let _ = writeln!(
                            s,
                            "         image {} rows {}..{} cols {}..{}",
                            p.image_id, p.top, p.bottom, p.left, p.right
                        );
                    }
                }
            }
        }
    }
    s
}

fn ranges(rows: &[u16]) -> Vec<(u16, u16)> {
    let mut out: Vec<(u16, u16)> = Vec::new();
    for &r in rows {
        match out.last_mut() {
            Some(last) if last.1 + 1 == r => last.1 = r,
            _ => out.push((r, r)),
        }
    }
    out
}

#[cfg(unix)]
fn describe_keys(keys: &[driver::Key]) -> String {
    if keys.is_empty() {
        return "(none)".into();
    }
    keys.iter()
        .map(|k| match k {
            driver::Key::Wait(d) => format!("wait:{}", d.as_millis()),
            driver::Key::AwaitQuery { query, .. } => format!("await-query:{query}"),
            driver::Key::WaitUntil { label, .. } => format!("wait-until:{label}"),
            driver::Key::Bytes(b) => printable(&String::from_utf8_lossy(b)),
            driver::Key::Resize { cols, rows, cell_w, cell_h } => {
                format!("resize:{cols}x{rows}@{cell_w}x{cell_h}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Escapes rendered so a human can read them: `ESC` for 0x1b, `\xNN` otherwise.
pub fn printable(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '\x1b' => out.push_str("ESC"),
            '\r' => out.push_str("CR"),
            '\n' => out.push_str("LF"),
            '\t' => out.push_str("TAB"),
            '\x07' => out.push_str("BEL"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
