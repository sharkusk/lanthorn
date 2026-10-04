//! A real terminal emulator's verdict on lanthorn's bytes (SQ-0764).
//!
//! `pty_emitted_stream.rs` asserts on what OUR decoder read out of the stream.
//! That decoder and the renderer it audits were written by the same hands, so a
//! shared misreading of the kitty protocol is invisible to both — the harness
//! agrees with the bug. This binary adds the second opinion: the same bytes fed
//! to `qwertty-term-vt` (Ghostty's terminal core, ported), which resolves
//! placements the way a terminal actually would, and disagrees where it must.
//!
//! Two halves, deliberately unequal:
//!
//!   * `protocol` — PORTABLE, always runs, no fixture, no pty. Hand-authored
//!     kitty streams, a few hundred bytes each, pinning the continuation rule
//!     that makes lanthorn's placement painting fragile. This is the part that
//!     is a real test rather than a snapshot: it asserts both directions, so it
//!     fails if the rule is broken AND if it is over-applied.
//!   * `emitter` — PORTABLE, always runs. lanthorn's real emitter driven through
//!     a real `Terminal` over a byte sink, so the bytes judged are the ones a
//!     player's terminal receives, frame boundaries and buffer diff included.
//!     This is where SQ-0772 lives: the defect was a placement the damage model
//!     could not see, which no amount of hand-authored stream proves anything
//!     about.
//!   * `real_capture` — unix only, drives a real story through the pty, and
//!     asserts the two decoders agree on BOTH backgrounds and image coverage.
//!
//! Slow-test gating follows `pty_emitted_stream.rs`: nothing here is `#[ignore]`
//! (SQ-0368 reserved that for the multi-second full-game walkthroughs), and the
//! gitignored fixture makes the capture half skip vacuously rather than fail.

// Declared once by the group binary (`tests/pty.rs`) and shared by every pty
// suite in it; see `pty_emitted_stream.rs`.
use super::pty_stream;

/// The kitty row/column diacritics, by the value they encode. Index in kitty's
/// `rowcolumn-diacritics.txt` IS the value; these four were read out of
/// `qwertty-term-vt`'s own `src/kitty/unicode.rs` table (which the crate
/// unit-tests as sorted, and spot-checks at indices 30 and 294), not recalled.
const D: [char; 4] = ['\u{0305}', '\u{030D}', '\u{030E}', '\u{0310}'];

/// Index 164 in that same table — the third diacritic's job is the image id's
/// HIGH BYTE, and 164 is a value with bits set, which is the whole point: an id
/// whose high byte is zero survives losing this diacritic.
const HIGH_164: char = '\u{1DC0}';

/// An image id whose high byte is 164. `ESC[38;2;r;g;b` can only carry the low
/// 24 bits, so this id exists ONLY when the high-byte diacritic is present.
const ID_HIGH: u32 = (164 << 24) | 0x00b0_0001;
const ID_LOW_R: u8 = 0xb0;
const ID_LOW_G: u8 = 0x00;
const ID_LOW_B: u8 = 0x01;

/// A terminal wide enough to hold the 4-cell art with room either side, and a
/// cell size in the shape a real one answers `CSI 16 t` with.
const COLS: u16 = 20;
const ROWS: u16 = 6;
const CELL_W: u32 = 8;
const CELL_H: u32 = 16;

/// Where the art is painted: 4 cells wide starting at column 2, on rows 1..2.
const ART_LEFT: u16 = 2;
const ART_COLS: u16 = 4;
const ART_TOP: u16 = 1;
const ART_ROWS: u16 = 2;

/// Base64 without a dependency. `qwertty-term-vt` has one of these inside it;
/// pulling `base64` into this crate to encode 32 bytes of test payload would be
/// a production dependency's worth of ceremony for eight lines.
fn b64(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { A[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { A[n as usize & 63] as char } else { '=' });
    }
    out
}

/// A `a=T,U=1` transmit-and-display of a solid RGBA image, declaring a
/// `ART_COLS x ART_ROWS` cell grid — the shape lanthorn sends. `z=3` is
/// deliberately non-default so the authored z can be told apart from the -1
/// upstream reports for every virtual placement.
fn transmit(id: u32) -> String {
    let (w, h) = (u32::from(ART_COLS) * CELL_W, u32::from(ART_ROWS) * CELL_H);
    let rgba = [7u8, 8, 9, 255].repeat((w * h) as usize);
    format!(
        "\x1b_Gq=2,a=T,U=1,i={id},f=32,t=d,s={w},v={h},c={ART_COLS},r={ART_ROWS},z=3,m=0;{}\x1b\\",
        b64(&rgba)
    )
}

/// [`transmit`]'s twin with its payload deflated and `o=z` declared — the shape
/// lanthorn actually sends to a terminal that can inflate (SQ-0991).
fn transmit_compressed(id: u32) -> String {
    use std::io::Write as _;
    let (w, h) = (u32::from(ART_COLS) * CELL_W, u32::from(ART_ROWS) * CELL_H);
    let rgba = [7u8, 8, 9, 255].repeat((w * h) as usize);
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(&rgba).expect("an in-memory encoder cannot fail");
    let z = e.finish().expect("an in-memory encoder cannot fail");
    assert!(z.len() < 3072, "one chunk keeps the fixture readable; chunking is inflate.rs's own case");
    format!(
        "\x1b_Gq=2,a=T,U=1,i={id},f=32,o=z,t=d,s={w},v={h},c={ART_COLS},r={ART_ROWS},z=3,m=0;{}\x1b\\",
        b64(&z)
    )
}

/// One row of placeholders in lanthorn's own shape: the LEAD cell carries the
/// full diacritic triple (image row, image column, id high byte) and every cell
/// after it is a bare `U+10EEEE` relying on the continuation rule.
fn placeholder_row(row: u16, high: char) -> String {
    let mut s = format!(
        "\x1b[{};{}H\x1b[38;2;{ID_LOW_R};{ID_LOW_G};{ID_LOW_B}m",
        ART_TOP + row + 1,
        ART_LEFT + 1
    );
    s.push('\u{10EEEE}');
    s.push(D[row as usize]);
    s.push(D[0]);
    s.push(high);
    for _ in 1..ART_COLS {
        s.push('\u{10EEEE}');
    }
    s.push_str("\x1b[39m");
    s
}

/// The whole frame: the upload plus both placeholder rows.
fn full_frame(id: u32, high: char) -> String {
    let mut s = transmit(id);
    for row in 0..ART_ROWS {
        s.push_str(&placeholder_row(row, high));
    }
    s
}

/// Overpaint the lead cell of every art row with a plain space, exactly as a
/// later frame drawing a divider down that column would.
fn overpaint_lead_cells(s: &mut String) {
    for row in 0..ART_ROWS {
        let _ = std::fmt::Write::write_fmt(
            s,
            format_args!("\x1b[{};{}H\x1b[0m ", ART_TOP + row + 1, ART_LEFT + 1),
        );
    }
}

mod protocol {
    use super::*;
    use crate::pty_stream::oracle::{self, Origin};

    /// SQ-1000: the oracle undoes `o=z` ITSELF, so a caller holding a raw capture
    /// cannot silently resolve a screen with no pixels on it.
    ///
    /// This is the defect that took the project's own proof sheet out. The
    /// terminal core links no zlib, so a compressed transmit is dropped outright —
    /// and the PLACEMENT still resolves, because the placeholder cells are just
    /// cells. `examples/gallery.rs` kept finding each illustration's rect and
    /// painting nothing into it, which reads as a game that drew a blank screen.
    /// Every gallery illustration went blank the moment compression shipped and
    /// 5,945 tests passed throughout (SQ-0991/SQ-0999). Undoing it was each
    /// caller's job until now, which is a contract nobody can see they have
    /// broken.
    ///
    /// Asserted as a TWIN of the uncompressed frame rather than against a copied
    /// expectation: same image, same placement, one of them deflated. A stream
    /// the oracle cannot inflate resolves the placement with no `source_y` at all,
    /// so the last assertion is the one that separates "art" from "art-shaped
    /// hole".
    #[test]
    fn a_compressed_transmit_resolves_exactly_as_its_uncompressed_twin() {
        let by = |stream: String| {
            oracle::resolve(stream.as_bytes(), COLS, ROWS, CELL_W, CELL_H, Some((pty_stream::ANSWERED_FG, pty_stream::ANSWERED_BG)))
        };
        let mut plain = transmit(ID_HIGH);
        let mut zipped = transmit_compressed(ID_HIGH);
        for row in 0..ART_ROWS {
            plain.push_str(&placeholder_row(row, HIGH_164));
            zipped.push_str(&placeholder_row(row, HIGH_164));
        }
        assert!(zipped.contains("o=z"), "the fixture must actually be compressed");

        let (a, b) = (by(plain), by(zipped));
        assert_eq!(a.placements.len(), 1, "the direction: {}", a.describe_placements());
        assert_eq!(
            b.describe_placements(),
            a.describe_placements(),
            "the deflated twin resolves to the same placement, cell for cell"
        );
        let rows: Vec<Option<u32>> =
            (ART_TOP..ART_TOP + ART_ROWS).map(|r| b.cell(r, ART_LEFT).source_y).collect();
        assert!(
            rows.iter().all(Option::is_some),
            "every art cell draws a row of the IMAGE; a dropped upload leaves the placement \
             standing with nothing behind it, which is what a blank illustration panel is: {rows:?}"
        );
    }

    /// The baseline: a run with its lead cell intact is an image, and the oracle
    /// says where. Without this direction the next test would pass for a
    /// terminal that never resolves anything at all.
    #[test]
    fn a_run_with_its_lead_cell_intact_resolves_to_the_expected_placement() {
        let res = oracle::resolve(full_frame(ID_HIGH, HIGH_164).as_bytes(), COLS, ROWS, CELL_W, CELL_H, Some((pty_stream::ANSWERED_FG, pty_stream::ANSWERED_BG)));

        assert_eq!(
            res.placements.len(),
            1,
            "one image, aggregated from its per-row entries: {}",
            res.describe_placements()
        );
        let p = &res.placements[0];
        assert_eq!(p.image_id, ID_HIGH, "the full 32-bit id, high byte and all");
        assert_eq!(
            (p.top, p.bottom, p.left, p.right),
            (ART_TOP, ART_TOP + ART_ROWS - 1, ART_LEFT, ART_LEFT + ART_COLS - 1),
            "the rect the placeholder cells describe: {}",
            p.describe()
        );
        assert_eq!(p.cells, usize::from(ART_ROWS) * usize::from(ART_COLS));
        assert_eq!(p.origin, Origin::Virtual);
        // The authored z, not the -1 upstream reports for every virtual
        // placement — the difference is the reason `ImageRect::z` reads storage.
        assert_eq!(p.z, 3, "the transmit asked for z=3");

        // Every cell of the rect, and nothing outside it.
        for row in 0..ROWS {
            for col in 0..COLS {
                let inside = (ART_TOP..ART_TOP + ART_ROWS).contains(&row)
                    && (ART_LEFT..ART_LEFT + ART_COLS).contains(&col);
                assert_eq!(
                    res.cell(row, col).image_id,
                    if inside { Some(ID_HIGH) } else { None },
                    "cell ({row},{col})"
                );
            }
        }
    }

    /// The rule that paid for the crate. Overpaint the lead cell and the run
    /// loses its high-byte diacritic; the id truncates to the low 24 bits, the
    /// lookup misses, and a real terminal draws NOTHING — while the surviving
    /// cells are still `U+10EEEE` placeholders our own decoder happily reports
    /// as an image.
    #[test]
    fn a_run_whose_lead_cell_was_overpainted_resolves_to_nothing() {
        let mut bytes = full_frame(ID_HIGH, HIGH_164);
        overpaint_lead_cells(&mut bytes);
        let res = oracle::resolve(bytes.as_bytes(), COLS, ROWS, CELL_W, CELL_H, Some((pty_stream::ANSWERED_FG, pty_stream::ANSWERED_BG)));

        assert!(
            res.placements.is_empty(),
            "the orphaned run names {:#010x} truncated to its low 24 bits, an image the \
             terminal does not hold — it must draw nothing:\n{}",
            ID_HIGH,
            res.describe_placements()
        );
        for row in 0..ROWS {
            for col in 0..COLS {
                assert_eq!(res.cell(row, col).image_id, None, "cell ({row},{col}) must be bare");
            }
        }

        // And the trap this whole file exists to spring: OUR decoder still sees
        // the placeholder cells and still calls them an image. Both readings are
        // honest about what they measure; only the oracle's is about pixels.
        let mut ours = crate::pty_stream::decode::Term::new(COLS, ROWS);
        ours.feed(bytes.as_bytes());
        let mine = ours.placements();
        assert_eq!(mine.len(), 1, "our decoder reports placeholder cells, which are still there");
        assert_eq!(mine[0].image_id, ID_HIGH & oracle::ID_MASK, "and only their low 24 bits");
        assert!(
            !oracle::disagreements(&ours, &res).is_empty(),
            "so the two decoders MUST disagree on this stream"
        );
    }

    /// The other direction, which is what stops the test above from passing for
    /// the wrong reason: overpaint the lead cells and then RE-EMIT them, as a
    /// correct repaint would, and the placement resolves exactly as before.
    #[test]
    fn a_partial_overpaint_that_re_emits_the_lead_cell_still_resolves() {
        let mut bytes = full_frame(ID_HIGH, HIGH_164);
        overpaint_lead_cells(&mut bytes);
        for row in 0..ART_ROWS {
            bytes.push_str(&placeholder_row(row, HIGH_164));
        }
        let res = oracle::resolve(bytes.as_bytes(), COLS, ROWS, CELL_W, CELL_H, Some((pty_stream::ANSWERED_FG, pty_stream::ANSWERED_BG)));

        assert_eq!(res.placements.len(), 1, "{}", res.describe_placements());
        let p = &res.placements[0];
        assert_eq!(p.image_id, ID_HIGH);
        assert_eq!(
            (p.top, p.bottom, p.left, p.right),
            (ART_TOP, ART_TOP + ART_ROWS - 1, ART_LEFT, ART_LEFT + ART_COLS - 1),
            "{}",
            p.describe()
        );
        assert_eq!(p.cells, usize::from(ART_ROWS) * usize::from(ART_COLS));
    }

    /// The failure mode is WORSE when the id's high byte is zero, which is
    /// lanthorn's own id range (`0x00B0_xxxx`, `render/graphics.rs`): the
    /// truncated id still names a real image, so the lookup succeeds and the
    /// orphaned run resolves — but with no row diacritic it claims image row 0,
    /// so every row of the art redraws the art's FIRST row, and it starts one
    /// cell right of where it should. Silent corruption instead of a blank.
    #[test]
    fn a_zero_high_byte_id_survives_the_overpaint_but_draws_the_wrong_fragment() {
        let id: u32 = ID_HIGH & oracle::ID_MASK;
        let mut bytes = full_frame(id, D[0]);
        overpaint_lead_cells(&mut bytes);
        let res = oracle::resolve(bytes.as_bytes(), COLS, ROWS, CELL_W, CELL_H, Some((pty_stream::ANSWERED_FG, pty_stream::ANSWERED_BG)));

        assert_eq!(res.placements.len(), 1, "the truncated id still resolves: {}", res.describe_placements());
        let p = &res.placements[0];
        assert_eq!(
            (p.left, p.right),
            (ART_LEFT + 1, ART_LEFT + ART_COLS - 1),
            "the run now starts where the lead cell used to be: {}",
            p.describe()
        );
        assert_eq!(p.cells, usize::from(ART_ROWS) * (usize::from(ART_COLS) - 1));
    }

    /// A painted background is not an image, to the oracle either. The mirror of
    /// `decode.rs`'s own first test, run through the real emulator so the two
    /// models are pinned to the same distinction.
    #[test]
    fn a_painted_background_is_not_a_placement() {
        let mut bytes = String::new();
        for row in 0..ART_ROWS {
            bytes.push_str(&format!(
                "\x1b[{};{}H\x1b[48;2;40;30;90m    \x1b[0m",
                ART_TOP + row + 1,
                ART_LEFT + 1
            ));
        }
        let res = oracle::resolve(bytes.as_bytes(), COLS, ROWS, CELL_W, CELL_H, Some((pty_stream::ANSWERED_FG, pty_stream::ANSWERED_BG)));

        assert!(res.placements.is_empty(), "paint is not a placement");
        for col in ART_LEFT..ART_LEFT + ART_COLS {
            let c = res.cell(ART_TOP, col);
            assert_eq!(c.bg, crate::pty_stream::decode::Color::Rgb(40, 30, 90));
            assert_eq!(c.image_id, None);
        }

        // And our decoder reads it the same way, so this stream is agreement.
        let mut ours = crate::pty_stream::decode::Term::new(COLS, ROWS);
        ours.feed(bytes.as_bytes());
        let d = oracle::disagreements(&ours, &res);
        assert!(d.is_empty(), "the two decoders must agree on a plain painted fill: {d:#?}");
    }

    /// An image id whose high byte is zero, so the low-24-bit foreground colour
    /// below carries the whole of it and the placeholder run needs no real
    /// high-byte diacritic.
    const STACK_ID: u32 = 0x0000_0007;
    /// How many cells wide the stacked placements and their placeholder run are.
    const STACK_COLS: u16 = 2;

    /// One image, several PIN placements on the home cell, and a placeholder run
    /// printed over the top of them. Each pin is
    /// `(placement id, z, source row, declared columns)`.
    ///
    /// Contrived, and it has to be: this is the shape that reaches the ambiguity
    /// SQ-0982 is about, where more than one resolved placement lands on one
    /// `(image, col, row)` and something has to choose between them. The transmit
    /// is `a=t` — transmit ONLY — so the image arrives with no placement of its
    /// own and every candidate below is one the test authored. Because none of
    /// them is `U=1`, `Placement::grid` finds no virtual placement for the id and
    /// the emulator resolves the placeholder run to nothing, which is exactly what
    /// leaves the run to be explained by the pins.
    fn stacked_pins(pins: &[(u32, i32, u32, u16)]) -> String {
        let (w, h) = (u32::from(STACK_COLS) * CELL_W, CELL_H);
        let mut s = format!(
            "\x1b_Gq=2,a=t,i={STACK_ID},f=32,t=d,s={w},v={h},m=0;{}\x1b\\",
            b64(&[9u8, 9, 9, 255].repeat((w * h) as usize))
        );
        for &(placement, z, source_row, cols) in pins {
            // `C=1` so displaying one does not walk the cursor off the cell the
            // next one has to be anchored to.
            s.push_str(&format!(
                "\x1b[1;1H\x1b_Gq=2,a=p,i={STACK_ID},p={placement},c={cols},r=1,\
                 z={z},y={source_row},C=1\x1b\\"
            ));
        }
        // The run itself, in lanthorn's own shape: the lead cell carries the
        // diacritic triple (image row 0, image column 0, id high byte 0) and the
        // rest lean on the continuation rule.
        s.push_str("\x1b[1;1H\x1b[38;2;0;0;7m");
        s.push('\u{10EEEE}');
        s.push(D[0]);
        s.push(D[0]);
        s.push(D[0]);
        for _ in 1..STACK_COLS {
            s.push('\u{10EEEE}');
        }
        s.push_str("\x1b[39m");
        s
    }

    /// One resolution of such a stream.
    fn stacked_resolve(bytes: &str) -> oracle::Resolved {
        oracle::resolve(
            bytes.as_bytes(),
            COLS,
            ROWS,
            CELL_W,
            CELL_H,
            Some((pty_stream::ANSWERED_FG, pty_stream::ANSWERED_BG)),
        )
    }

    /// The source row the oracle reports for the home cell of such a stream.
    fn stacked_source_row(bytes: &str) -> Option<u32> {
        stacked_resolve(bytes).cell(0, 0).source_y
    }

    /// The rects it aggregates those placements into, as text — `ImageRect` is not
    /// `PartialEq`, and its own description names everything two resolutions of one
    /// stream could differ on.
    fn stacked_rects(bytes: &str) -> String {
        stacked_resolve(bytes).describe_placements()
    }

    /// Six placements of one image on one cell: the cell reports the source row
    /// of the one a renderer draws on TOP.
    ///
    /// `OracleCell::source_y` answers "which pixel row of that image lands here",
    /// and what lands on the glass is the topmost draw — so the protocol's own
    /// z-order is the precedence, not whichever candidate a `HashMap` happened to
    /// hand over last (SQ-0982). Each placement reads a different row of the image
    /// (`y=0,2,…,10`) and sits at its own z, so exactly one answer is right and the
    /// other five are the readings a wrong precedence gives.
    #[test]
    fn several_placements_on_one_cell_report_the_topmost_source_row() {
        let pins: Vec<(u32, i32, u32, u16)> =
            (0..6).map(|i| (i + 1, i as i32, i * 2, STACK_COLS)).collect();
        let bytes = stacked_pins(&pins);

        assert_eq!(
            stacked_source_row(&bytes),
            Some(10),
            "z=5 is the top of the stack, so the cell shows image row 10 — any other \
             row here is a lower placement winning, and `None` means the run never \
             resolved at all"
        );

        // The other direction, which is what stops the above from passing for a
        // reader that simply prefers the largest source row: turn the stack over
        // and the bottom-most row is the one on top.
        let flipped: Vec<(u32, i32, u32, u16)> =
            (0..6).map(|i| (i + 1, -(i as i32), i * 2, STACK_COLS)).collect();
        assert_eq!(
            stacked_source_row(&stacked_pins(&flipped)),
            Some(0),
            "with the z order reversed, y=0 is the placement on top"
        );
    }

    /// The same bytes must resolve to the same reading.
    ///
    /// The case SQ-0982 needed and did not have. `resolve_placements` documents
    /// itself as returning "placements in arbitrary order" — it walks a `HashMap`
    /// — so the candidate list for a cell arrives in a fresh random permutation on
    /// every call, and `resolve_rects` used to take whichever ended up last. An
    /// instrument that answers differently on different runs is worse than one that
    /// is merely wrong, because nobody can tell which answer they got.
    ///
    /// Six candidates at ONE z, so the protocol's own rule cannot separate them and
    /// only the deterministic tail can, read twelve times: an unordered pick passes
    /// only if all twelve land on the same candidate, about one run in 10^9. Not a
    /// probabilistic test in the direction that matters — the fix makes it pass
    /// every time, and only the failure is chance.
    ///
    /// Each candidate also declares a DIFFERENT cell grid (`c=1..6`), which reaches
    /// the second unordered read in the same function: the pin branch took the
    /// declared grid off the first placement of that image the `HashMap` happened
    /// to yield, so the rect's own extent flipped between runs too. That is why the
    /// rect is asserted alongside the source row.
    #[test]
    fn the_same_bytes_always_resolve_the_same_way() {
        let pins: Vec<(u32, i32, u32, u16)> =
            (0..6).map(|i| (i + 1, 0, i * 2, i as u16 + 1)).collect();
        let bytes = stacked_pins(&pins);

        let first_row = stacked_source_row(&bytes);
        let first_rect = stacked_rects(&bytes);
        // Non-vacuity: the run has to have resolved to SOMETHING and the pins to a
        // rect, or every resolution below agrees with the first one for free.
        assert!(first_row.is_some(), "the run must resolve to one of the six placements");
        assert!(!first_rect.is_empty(), "the six pins must resolve to a rect");
        for attempt in 1..12 {
            assert_eq!(
                stacked_source_row(&bytes),
                first_row,
                "resolution {attempt} of the identical stream read a different source row — \
                 the oracle's answer is not a function of the bytes (SQ-0982)"
            );
            assert_eq!(
                stacked_rects(&bytes),
                first_rect,
                "resolution {attempt} of the identical stream described a different rect — \
                 the oracle's answer is not a function of the bytes (SQ-0982)"
            );
        }
    }
}

/// The real emitter, the real ratatui diff, a real terminal — no pty, no fixture,
/// no story (SQ-0772).
///
/// The `protocol` module above hand-authors the streams it judges, which pins the
/// RULE but not lanthorn's obedience to it. `real_capture` below judges lanthorn's
/// own bytes, but needs a pty, a commercial story file and a couple of seconds.
/// This module sits between them: it drives `GraphicsRender` through a real
/// `Terminal` over a byte sink, so the bytes are the ones a player's terminal would
/// receive, and resolves them through the same emulator — every frame boundary,
/// buffer diff and cell-skip decision included, which is exactly where this defect
/// lived. It runs everywhere and takes milliseconds.
mod emitter {
    use ratatui::Terminal;
    use ratatui::backend::CrosstermBackend;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Style};
    use ratatui::widgets::Widget;
    use ratatui::{TerminalOptions, Viewport};

    use app::engine::GraphicsWindow;
    use app::render::graphics::{GraphicsRender, kitty_picker};

    use crate::pty_stream::{self, oracle};

    const COLS: u16 = 40;
    const ROWS: u16 = 12;
    const CELL_W: u16 = 8;
    const CELL_H: u16 = 18;

    /// The graphics window's cell rect. Column 3 is the LEAD column — the one a
    /// divider drawn down the screen's left flank lands on, and the one whose loss
    /// used to orphan the rest of every row.
    const ART: Rect = Rect { x: 3, y: 2, width: 12, height: 6 };

    /// A canvas whose every pixel ROW is a different colour, so a placement that
    /// draws the wrong row of it is distinguishable from one that draws the right
    /// one. A flat canvas would let the corrupt reading pass.
    fn window(version: u64) -> GraphicsWindow {
        tinted_window(version, 40)
    }

    /// [`window`], with the green channel under our control so two versions can
    /// differ in their PIXELS and not merely in their version number — which is
    /// what tells a re-transmit apart from a re-place (SQ-0995).
    fn tinted_window(version: u64, green: u8) -> GraphicsWindow {
        let (w, h) = (u32::from(ART.width) * u32::from(CELL_W), u32::from(ART.height) * u32::from(CELL_H));
        let mut canvas = image::RgbaImage::new(w, h);
        for (_, y, p) in canvas.enumerate_pixels_mut() {
            *p = image::Rgba([(y % 251) as u8, green, 200, 255]);
        }
        GraphicsWindow { win: 1, canvas: std::sync::Arc::new(canvas), version, upscale: false, undrawn: false }
    }

    /// The backend's byte sink, kept on our side of the writer: ratatui-crossterm's
    /// own `writer()` accessor is behind an unstable feature gate, and a shared
    /// buffer is a smaller thing to depend on than an unstable API.
    #[derive(Clone, Default)]
    struct Sink(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

    impl std::io::Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A `Terminal` writing into a byte sink we can hand to the emulator. The
    /// viewport is FIXED so nothing consults the real terminal this test may or may
    /// not be attached to.
    fn terminal() -> (Terminal<CrosstermBackend<Sink>>, Sink) {
        let sink = Sink::default();
        let term = Terminal::with_options(
            CrosstermBackend::new(sink.clone()),
            TerminalOptions { viewport: Viewport::Fixed(Rect::new(0, 0, COLS, ROWS)) },
        )
        .expect("a fixed viewport needs no terminal to size itself against");
        (term, sink)
    }

    /// Resolve everything written so far the way a terminal would.
    ///
    /// The emitter compresses its uploads (`o=z`, SQ-0976) and the oracle's
    /// terminal core links no zlib. `oracle::resolve` undoes that itself now
    /// (SQ-1000), so this hands it the wire bytes exactly as a capture would —
    /// which is also what `a_compressed_upload_resolves_without_the_caller_undoing_it`
    /// pins.
    fn resolve(sink: &Sink) -> oracle::Resolved {
        let bytes = sink.0.borrow();
        oracle::resolve(&bytes, COLS, ROWS, u32::from(CELL_W), u32::from(CELL_H), Some((pty_stream::ANSWERED_FG, pty_stream::ANSWERED_BG)))
    }

    /// Draw the art, and nothing else.
    fn frame_with_art(term: &mut Terminal<CrosstermBackend<Sink>>, gr: &mut GraphicsRender, version: u64) {
        let picker = kitty_picker(CELL_W, CELL_H);
        term.draw(|f| gr.render(&picker, &window(version), ART, Style::default(), f.buffer_mut()))
            .expect("drawing into a byte sink cannot fail");
    }

    /// The art's every cell, and the image pixel row landing on each — the reading
    /// that separates a healthy placement from an orphaned one.
    fn source_rows(res: &oracle::Resolved) -> Vec<Option<u32>> {
        (ART.y..ART.y + ART.height).map(|row| res.cell(row, ART.x).source_y).collect()
    }

    /// The baseline, and the direction that stops the rest passing vacuously: the
    /// emitter's own bytes place the whole rect, and each screen row draws a
    /// DIFFERENT row of the image.
    #[test]
    fn the_emitters_bytes_place_every_cell_of_the_art() {
        let mut gr = GraphicsRender::default();
        let (mut term, sink) = terminal();
        frame_with_art(&mut term, &mut gr, 1);
        let res = resolve(&sink);

        assert_eq!(res.placements.len(), 1, "{}", res.describe_placements());
        let p = &res.placements[0];
        assert_eq!(
            (p.top, p.bottom, p.left, p.right),
            (ART.y, ART.y + ART.height - 1, ART.x, ART.x + ART.width - 1),
            "the whole window rect: {}",
            p.describe()
        );
        assert_eq!(p.cells, usize::from(ART.width) * usize::from(ART.height));

        let rows = source_rows(&res);
        assert!(
            rows.windows(2).all(|w| w[0] < w[1]),
            "each screen row must draw a LOWER row of the image than the one above it, else \
             the placement is redrawing one row over and over: {rows:?}"
        );
    }

    /// SQ-0995, judged on the wire and by a real terminal core rather than on our
    /// own buffer. A frame whose canvas CHANGED emits ONE placeholder cell, not the
    /// window's whole grid — and the image behind the id that never moved is the new
    /// canvas, with the placement still covering every cell of the rect.
    ///
    /// The two halves have to be asserted together. The cheap half (bytes) is what
    /// the quest is about: the id is a per-cell value, so re-keying it per canvas
    /// made one changed pixel repaint `width*height` cells. The expensive half
    /// (Ghostty's storage) is what makes the cheap half safe: the protocol says
    /// *"When re-transmitting image data for a specific id, the existing image and
    /// all its placements must be deleted"*, and our `a=T,U=1,r,c` re-creates an
    /// anonymous placement in the same command (SQ-1512 dropped the earlier `p=1`
    /// naming, a workaround for a Ghostty leak now fixed upstream), so the frame
    /// still draws instead of costing one cell and nothing else.
    #[test]
    fn a_changed_canvas_re_transmits_to_the_same_id_and_emits_one_cell() {
        const PLACEHOLDER: &[u8] = "\u{10EEEE}".as_bytes();
        let picker = kitty_picker(CELL_W, CELL_H);
        let mut gr = GraphicsRender::default();
        let (mut term, sink) = terminal();

        // Two frames of the first canvas, so the window is settled: the second
        // sheds the transmit escape and is otherwise identical.
        for version in 1..=2 {
            term.draw(|f| gr.render(&picker, &tinted_window(version, 40), ART, Style::default(), f.buffer_mut()))
                .expect("drawing into a byte sink cannot fail");
        }
        let settled = sink.0.borrow().len();

        // Now the game repaints its window with different pixels.
        term.draw(|f| gr.render(&picker, &tinted_window(3, 90), ART, Style::default(), f.buffer_mut()))
            .expect("drawing into a byte sink cannot fail");
        let frame = sink.0.borrow()[settled..].to_vec();

        let cells = frame.windows(PLACEHOLDER.len()).filter(|w| *w == PLACEHOLDER).count();
        let grid = usize::from(ART.width) * usize::from(ART.height);
        assert_eq!(
            cells, 1,
            "a changed canvas emits the lead cell carrying the transmit and nothing else, \
             not all {grid} placeholders ({} bytes emitted)",
            frame.len()
        );
        assert!(
            frame.windows(4).any(|w| w == b"a=T,"),
            "and the frame does carry the new pixels — one cell and no upload would be a \
             frame that changed nothing"
        );

        // What a real terminal is holding afterwards.
        let res = resolve(&sink);
        assert_eq!(res.placements.len(), 1, "{}", res.describe_placements());
        let p = &res.placements[0];
        assert_eq!(
            (p.top, p.bottom, p.left, p.right, p.cells),
            (ART.y, ART.y + ART.height - 1, ART.x, ART.x + ART.width - 1, grid),
            "the re-transmit must leave the placement covering the whole rect: {}",
            p.describe()
        );
        let rows = source_rows(&res);
        assert!(
            rows.windows(2).all(|w| w[0] < w[1]),
            "and each screen row still draws its own row of the image: {rows:?}"
        );

        // The pixels behind that id are the SECOND canvas: green 90, not 40.
        let img = res.images.get(&p.image_id).unwrap_or_else(|| {
            panic!("the terminal holds no image {:#010x}: {}", p.image_id, res.describe_placements())
        });
        assert_eq!(
            img.rgba.get(1).copied(),
            Some(90),
            "re-transmitting to a live id replaces the data behind it"
        );
    }

    /// SQ-0772's corruption mode, through the real emitter. A later frame draws a
    /// divider down the art's lead column and re-places the art everywhere else —
    /// the shape of Journey's chrome ring trimming the raster composite's left edge.
    /// The survivors must still name their own image rows.
    ///
    /// Before the fix the whole row leaned on that lead cell, so its loss left the
    /// rest of the row anchorless: lanthorn's ids have a zero high byte, so the run
    /// still resolved — to the image's FIRST row, on every screen row.
    #[test]
    fn overpainting_the_lead_column_leaves_the_survivors_naming_their_own_rows() {
        let mut gr = GraphicsRender::default();
        let (mut term, sink) = terminal();
        frame_with_art(&mut term, &mut gr, 1);

        let picker = kitty_picker(CELL_W, CELL_H);
        term.draw(|f| {
            let buf = f.buffer_mut();
            gr.render(&picker, &window(1), ART, Style::default(), buf);
            // …and a divider down the art's first column, drawn after it.
            for y in ART.y..ART.y + ART.height {
                if let Some(cell) = buf.cell_mut((ART.x, y)) {
                    cell.set_symbol("\u{2502}").set_style(Style::default().fg(Color::Rgb(9, 9, 9)));
                }
            }
        })
        .expect("drawing into a byte sink cannot fail");

        let res = resolve(&sink);
        assert_eq!(res.placements.len(), 1, "the art survives the trim: {}", res.describe_placements());
        let p = &res.placements[0];
        assert_eq!(
            (p.left, p.right),
            (ART.x + 1, ART.x + ART.width - 1),
            "the divider took the first column and nothing else: {}",
            p.describe()
        );

        let rows: Vec<Option<u32>> =
            (ART.y..ART.y + ART.height).map(|row| res.cell(row, ART.x + 1).source_y).collect();
        assert!(
            rows.windows(2).all(|w| w[0] < w[1]),
            "every surviving row must still draw its OWN row of the image; all-equal means the \
             run lost its anchor and is redrawing the first row down the whole rect: {rows:?}"
        );
    }

    /// SQ-0996, the same property one layer over: a CHROME BAND, which is placed by
    /// `ratatui-image` rather than by lanthorn's own emitter.
    ///
    /// This is the path the v6 pane is actually drawn through. SQ-0995's lane
    /// established that `render_kitty_virtual` — the window emitter every case above
    /// exercises — is not reached by v6 at all: Journey, Zork Zero, Shogun and
    /// Arthur emit no ids from lanthorn's own range, because their art is a chrome
    /// ring of bands and a raster composite, and both go through the crate. The
    /// crate drew a fresh `rand::random()` id per `Protocol`, so a band that changed
    /// by one pixel repainted its whole placeholder rect; `Picker::new_protocol_with_id`
    /// (added to the fork) hands it back the id it is already placed as.
    ///
    /// Both halves again, and for the same reason as the window case: the bytes are
    /// what the quest is about, and the emulator's storage is what makes them safe.
    /// A re-transmit that replaced the image without re-creating the placement would
    /// cost one cell and draw nothing at all.
    #[test]
    fn a_changed_chrome_band_re_transmits_to_the_same_id_and_emits_one_cell() {
        use app::render::v6_layout::uniform_scale;
        const PLACEHOLDER: &[u8] = "\u{10EEEE}".as_bytes();

        let picker = kitty_picker(CELL_W, CELL_H);
        let mut gr = GraphicsRender::default();
        let (mut term, sink) = terminal();

        let pane = Rect::new(0, 0, COLS, ROWS);
        let native = (u32::from(COLS) * u32::from(CELL_W), u32::from(ROWS) * u32::from(CELL_H));
        let scale = uniform_scale((native.0 as u16, native.1 as u16), native);
        let band = Rect::new(0, 0, COLS, 4);
        let art = |green: u8| {
            image::RgbaImage::from_fn(native.0, native.1, |_x, y| {
                image::Rgba([(y % 251) as u8, green, 200, 255])
            })
        };
        let mut frame = |gr: &mut GraphicsRender, green: u8| {
            term.draw(|f| gr.draw_chrome_band(&picker, &art(green), &scale, pane, band, f.buffer_mut()))
                .expect("drawing into a byte sink cannot fail");
        };

        // Two frames of the first art, so the band is settled.
        frame(&mut gr, 40);
        frame(&mut gr, 40);
        let settled = sink.0.borrow().len();

        // SQ-1188: the change frame stages the encode for the worker and keeps
        // the old upload placed — its emission is byte-empty. The transmit goes
        // out on the frame after the result lands, and THAT frame's emission is
        // what the one-cell claim is about.
        frame(&mut gr, 90);
        {
            let change_frame = sink.0.borrow()[settled..].to_vec();
            assert!(
                !change_frame.windows(4).any(|w| w == b"a=T,"),
                "the change frame re-places the OLD upload and transmits nothing"
            );
            assert_eq!(
                change_frame.windows(PLACEHOLDER.len()).filter(|w| *w == PLACEHOLDER).count(),
                0,
                "and repaints no placeholder cell"
            );
        }
        let settled = sink.0.borrow().len();
        gr.spawn_band_jobs(&picker);
        for _ in 0..500 {
            if gr.poll_v6_job() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        frame(&mut gr, 90);
        let emitted = sink.0.borrow()[settled..].to_vec();

        let cells = emitted.windows(PLACEHOLDER.len()).filter(|w| *w == PLACEHOLDER).count();
        let grid = usize::from(band.width) * usize::from(band.height);
        assert_eq!(
            cells, 1,
            "a changed band emits the lead cell carrying the transmit and nothing else, not \
             all {grid} placeholders ({} bytes emitted)",
            emitted.len()
        );
        assert!(
            emitted.windows(4).any(|w| w == b"a=T,"),
            "and it does carry the new pixels — one cell and no upload would be a frame that \
             changed nothing"
        );
        assert!(
            !emitted.windows(4).any(|w| w == b"a=d,"),
            "and frees nothing: the id re-transmitted to is the id on screen"
        );

        // What a real terminal is holding afterwards.
        let res = resolve(&sink);
        assert_eq!(res.placements.len(), 1, "{}", res.describe_placements());
        let p = &res.placements[0];
        assert_eq!(
            (p.top, p.bottom, p.left, p.right, p.cells),
            (band.y, band.y + band.height - 1, band.x, band.x + band.width - 1, grid),
            "the re-transmit must leave the placement covering the whole band: {}",
            p.describe()
        );
        let img = res.images.get(&p.image_id).unwrap_or_else(|| {
            panic!("the terminal holds no image {:#010x}: {}", p.image_id, res.describe_placements())
        });
        assert_eq!(
            img.rgba.get(1).copied(),
            Some(90),
            "re-transmitting to a live id replaces the data behind it"
        );
    }

    /// The other half of the rule, and the reason the fix is buffer-visible cells
    /// rather than only self-describing ones: a frame that simply STOPS drawing the
    /// art must unpaint every placeholder cell it left behind.
    ///
    /// Honest about its own strength — this one passes on the old emitter too, in
    /// this shape. `Skip` is part of ratatui's cell equality, so a cell that was
    /// `Skip` last frame and plain this frame does diff and does get repainted. What
    /// the old shape could not survive was a placement whose cells stayed `Skip`
    /// frame after frame while its ANCHOR was overpainted (the test above), and this
    /// is the guard that the fix did not trade that away for a leak in the simpler
    /// direction.
    #[test]
    fn a_frame_that_stops_drawing_the_art_unpaints_every_placeholder_cell() {
        let mut gr = GraphicsRender::default();
        let (mut term, sink) = terminal();
        frame_with_art(&mut term, &mut gr, 1);
        assert_eq!(resolve(&sink).placements.len(), 1, "the art was placed to begin with");

        // The next frame draws ordinary text over the art's left third and leaves
        // the rest of its rows untouched — the ring/text layout that replaced the
        // raster composite in the capture.
        term.draw(|f| {
            let buf = f.buffer_mut();
            ratatui::widgets::Paragraph::new("text").render(Rect::new(ART.x, ART.y, 4, ART.height), buf);
        })
        .expect("drawing into a byte sink cannot fail");

        let res = resolve(&sink);
        assert!(
            res.placements.is_empty(),
            "nothing draws the art any more, so nothing may still be on screen: {}",
            res.describe_placements()
        );
        for row in 0..ROWS {
            for col in 0..COLS {
                assert_eq!(res.cell(row, col).image_id, None, "cell ({row},{col}) still carries an image");
            }
        }

        // And our own decoder must read it the same way, or the harness is lying.
        let mut ours = crate::pty_stream::decode::Term::new(COLS, ROWS);
        ours.feed(&sink.0.borrow());
        let d = oracle::disagreements(&ours, &res);
        assert!(d.is_empty(), "the two decoders must agree that the art is gone: {d:#?}");
    }

    /// The cheapness the old shape bought, kept: re-placing an unchanged image
    /// repaints nothing. A fix that made every cell buffer-visible by repainting it
    /// every frame would satisfy every test above and cost a screenful of
    /// placeholders per frame for ever.
    ///
    /// Measured between the SECOND and THIRD identical frames, because the second
    /// legitimately repaints one cell: the first frame's leading cell carries the
    /// image upload and the second's does not, so that one cell differs. From there
    /// on the buffer is identical and the diff is silent.
    #[test]
    fn re_placing_an_unchanged_image_repaints_no_placeholder_cells() {
        let mut gr = GraphicsRender::default();
        let (mut term, sink) = terminal();
        frame_with_art(&mut term, &mut gr, 1);
        frame_with_art(&mut term, &mut gr, 1);
        let settled = sink.0.borrow().len();
        assert!(
            String::from_utf8_lossy(&sink.0.borrow()[..settled]).contains('\u{10EEEE}'),
            "the frames so far did paint placeholders, so the next frame's silence means something"
        );

        frame_with_art(&mut term, &mut gr, 1);
        let added = String::from_utf8_lossy(&sink.0.borrow()[settled..]).to_string();
        assert!(
            !added.contains('\u{10EEEE}'),
            "an identical frame must diff to nothing but cursor bookkeeping; it repainted \
             placeholders: {added:?}"
        );
    }
}

/// The rasteriser (SQ-0775): the resolved screen drawn as pixels.
///
/// PORTABLE, always runs. Every stream here is hand-authored so the expected
/// picture can be stated exactly, and every assertion names a COORDINATE and a
/// COLOUR. That shape is the point: the obvious failure mode for a PNG writer is
/// emitting a plausible-looking blank, which "a file appeared" and "the file is
/// 40kB" both accept happily. A blank canvas fails `art_lands_where_the_placement_put_it`
/// on its first pixel, fails the glyph test for want of any foreground pixel,
/// and fails both z-order tests in opposite directions.
mod raster {
    use super::*;
    use crate::pty_stream::{oracle, raster};

    /// The screen's fill where nothing was written: `qwertty-term-vt`'s default
    /// palette entry 0, which is Ghostty's `Name::Black` — NOT pure black. Every
    /// "nothing is here" assertion below is against this, so a rasteriser that
    /// invented its own background would fail them all.
    const DEFAULT_BG: [u8; 4] = [0x1D, 0x1F, 0x21, 255];

    /// An image whose every pixel ROW is a different colour, so a placement that
    /// draws the wrong row of it is distinguishable from one that draws the right
    /// one. Row `r` is `[20 + r, 0, 0]`; the `+ 20` keeps row 0 clear of black,
    /// so "drew the first row" and "drew nothing" cannot be confused.
    fn gradient_transmit(id: u32) -> String {
        let (w, h) = (u32::from(ART_COLS) * CELL_W, u32::from(ART_ROWS) * CELL_H);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for _ in 0..w {
                rgba.extend_from_slice(&[20 + y as u8, 0, 0, 255]);
            }
        }
        format!(
            "\x1b_Gq=2,a=T,U=1,i={id},f=32,t=d,s={w},v={h},c={ART_COLS},r={ART_ROWS},z=3,m=0;{}\x1b\\",
            b64(&rgba)
        )
    }

    /// The gradient art, placed the way lanthorn places art: one placeholder run
    /// per row, lead cell carrying the diacritic triple.
    fn gradient_frame() -> String {
        let mut s = gradient_transmit(ID_HIGH);
        for row in 0..ART_ROWS {
            s.push_str(&placeholder_row(row, HIGH_164));
        }
        s
    }

    fn draw(bytes: &str) -> image::RgbaImage {
        raster::render(&oracle::resolve(bytes.as_bytes(), COLS, ROWS, CELL_W, CELL_H, Some((pty_stream::ANSWERED_FG, pty_stream::ANSWERED_BG))))
    }

    fn px(canvas: &image::RgbaImage, x: u32, y: u32) -> [u8; 4] {
        canvas.get_pixel(x, y).0
    }

    /// The art occupies exactly the pixels the placement resolved to, and each
    /// screen row draws its OWN row of the image.
    ///
    /// The gradient is what makes the second half real: dest and source are both
    /// 32x32 here, so screen row `ART_TOP` must show image rows 0..15 and screen
    /// row `ART_TOP + 1` image rows 16..31. A rasteriser that drew the image once
    /// into its bounding box, or one that lost `source_y` the way SQ-0772's
    /// orphaned runs do, paints the same band twice and fails on the second row.
    #[test]
    fn art_lands_where_the_placement_put_it() {
        let canvas = draw(&gradient_frame());
        assert_eq!(
            (canvas.width(), canvas.height()),
            (u32::from(COLS) * CELL_W, u32::from(ROWS) * CELL_H),
            "the canvas is the screen at its own cell size"
        );

        let (x0, y0) = (u32::from(ART_LEFT) * CELL_W, u32::from(ART_TOP) * CELL_H);
        let (x1, y1) = (x0 + u32::from(ART_COLS) * CELL_W, y0 + u32::from(ART_ROWS) * CELL_H);

        // Top-left corner of the rect, and the row of the image it must show.
        assert_eq!(px(&canvas, x0, y0), [20, 0, 0, 255], "the rect's first pixel is the image's first row");
        // One pixel down is one image row down (1:1 scale).
        assert_eq!(px(&canvas, x0, y0 + 1), [21, 0, 0, 255]);
        // The SECOND screen row of the placement — a different resolved run, with
        // its own source row. This is the assertion an aggregated rect cannot pass.
        assert_eq!(
            px(&canvas, x0, y0 + CELL_H),
            [20 + CELL_H as u8, 0, 0, 255],
            "screen row {} must draw image row {CELL_H}, not the first row again",
            ART_TOP + 1
        );
        // The rect's far corner, one pixel inside.
        assert_eq!(px(&canvas, x1 - 1, y1 - 1), [20 + (2 * CELL_H - 1) as u8, 0, 0, 255]);

        // …and nothing outside it, on all four sides.
        assert_eq!(px(&canvas, x0 - 1, y0), DEFAULT_BG, "one pixel left of the art");
        assert_eq!(px(&canvas, x1, y0), DEFAULT_BG, "one pixel right of the art");
        assert_eq!(px(&canvas, x0, y0 - 1), DEFAULT_BG, "one pixel above the art");
        assert_eq!(px(&canvas, x0, y1), DEFAULT_BG, "one pixel below the art");
    }

    /// A painted background fills its cell, and a glyph paints foreground pixels
    /// inside it without erasing it.
    ///
    /// The direction that matters: a blank-canvas bug passes nothing here. The
    /// space cell is asserted to be UNIFORMLY the painted colour, and the letter
    /// cell to hold both colours — so a rasteriser that skipped backgrounds, or
    /// one that skipped glyphs, fails a different assertion.
    #[test]
    fn a_painted_cell_is_filled_and_its_glyph_is_drawn_over_it() {
        // Row 4 (1-based row 5), from column 0: "A" then a space, on a painted bg.
        let canvas = draw("\x1b[5;1H\x1b[48;2;40;30;90m\x1b[38;2;200;10;20mA \x1b[0m");
        let y = 4 * CELL_H;
        let (bg, fg) = ([40, 30, 90, 255], [200, 10, 20, 255]);

        let letter: Vec<[u8; 4]> =
            (0..CELL_W).flat_map(|x| (0..CELL_H).map(move |dy| (x, dy))).map(|(x, dy)| px(&canvas, x, y + dy)).collect();
        assert!(letter.contains(&fg), "the 'A' painted no foreground pixel — the glyph never drew");
        assert!(letter.contains(&bg), "the 'A' covered its whole cell — the background never drew");
        assert!(
            letter.iter().all(|p| *p == fg || *p == bg),
            "a cell may only hold its own two colours"
        );

        // The space cell beside it: all background, no glyph.
        for dy in 0..CELL_H {
            for x in CELL_W..2 * CELL_W {
                assert_eq!(px(&canvas, x, y + dy), bg, "the space cell at ({x},{})", y + dy);
            }
        }
        // And a cell nothing ever wrote to keeps the screen's own fill.
        assert_eq!(px(&canvas, 0, 0), DEFAULT_BG);
    }

    /// A `z=-1` placement draws UNDER the text; a `z=1` placement draws OVER it.
    ///
    /// Both directions, because either alone passes for a rasteriser that ignores
    /// z entirely and always picks that one order. The image is pin-anchored
    /// rather than virtual so the text can be printed over it without destroying
    /// the placeholder run that positions it (the SQ-0772 failure, which is a
    /// different subject).
    fn pinned_art_under_text(z: i32) -> image::RgbaImage {
        let (w, h) = (2 * CELL_W, CELL_H);
        let rgba = [0u8, 200, 0, 255].repeat((w * h) as usize);
        draw(&format!(
            "\x1b_Gq=2,a=T,i=7,f=32,t=d,s={w},v={h},c=2,r=1,z={z},m=0;{}\x1b\\\
             \x1b[1;1H\x1b[38;2;255;255;255mW\x1b[0m",
            b64(&rgba)
        ))
    }

    #[test]
    fn a_negative_z_placement_draws_under_the_text() {
        let canvas = pinned_art_under_text(-1);
        let cell: Vec<[u8; 4]> =
            (0..CELL_W).flat_map(|x| (0..CELL_H).map(move |y| (x, y))).map(|(x, y)| px(&canvas, x, y)).collect();
        assert!(
            cell.contains(&[255, 255, 255, 255]),
            "the 'W' must be visible over a z=-1 image"
        );
        assert!(cell.contains(&[0, 200, 0, 255]), "and the image must fill the rest of the cell");
        // The cell beside it has no glyph, so it is all image.
        assert!(
            (CELL_W..2 * CELL_W).all(|x| px(&canvas, x, 0) == [0, 200, 0, 255]),
            "the un-lettered half of the placement is all image"
        );
    }

    #[test]
    fn a_positive_z_placement_draws_over_the_text() {
        let canvas = pinned_art_under_text(1);
        for y in 0..CELL_H {
            for x in 0..2 * CELL_W {
                assert_eq!(
                    px(&canvas, x, y),
                    [0, 200, 0, 255],
                    "a z=1 image covers the text under it; pixel ({x},{y}) shows through"
                );
            }
        }
    }

    /// A glyph printed into a cell a VIRTUAL placement covers does not draw over
    /// the image — it DELETES it, and takes the rest of the run with it.
    ///
    /// This is the measurement SQ-0944 turned on, and it contradicts a reading of
    /// the two z tests above that is easy to arrive at and wrong. Those pin the
    /// protocol's Z index using a PIN-ANCHORED placement, where the image is placed
    /// by cursor position and the cells keep independent text — `z = -1` really
    /// does put such an image under the glyphs. Every placement LANTHORN emits is a
    /// different animal: `U=1`, positioned by `U+10EEEE` placeholder characters, so
    /// the image IS the cell's content and there is no glyph layer in that cell for
    /// anything to be over. (`ratatui-image`'s `transmit_virtual` emits no `z` at
    /// all; the `-1` in `pty_stream/raster.rs`'s note is what the RENDERER sorts
    /// virtual placements at internally, which that module says.)
    ///
    /// Both directions, because the interesting part is the second: the glyph's own
    /// cell losing its image would be survivable, and the run TRUNCATING at the
    /// glyph is what makes "just draw the text on top" unimplementable. A renderer
    /// that layered glyphs over virtual placements would fail the first assertion;
    /// one that dropped only the lettered cell would fail the last.
    #[test]
    fn a_glyph_printed_into_a_virtual_placement_erases_it() {
        // The gradient art, then a 'W' printed into the placement's THIRD cell —
        // a continuation cell, so the lead cell's diacritic triple survives and the
        // run is not simply beheaded.
        let glyph_col = ART_LEFT + 2;
        let mut s = gradient_frame();
        s.push_str(&format!("\x1b[{};{}H\x1b[38;2;255;255;255mW\x1b[0m", ART_TOP + 1, glyph_col + 1));
        let res = oracle::resolve(
            s.as_bytes(),
            COLS,
            ROWS,
            CELL_W,
            CELL_H,
            Some((pty_stream::ANSWERED_FG, pty_stream::ANSWERED_BG)),
        );

        // Non-vacuity: the cells BEFORE the glyph still carry the image, so the
        // stream really did place art on this row and the losses below mean something.
        for col in ART_LEFT..glyph_col {
            assert!(
                res.cell(ART_TOP, col).image_id.is_some(),
                "col {col}, before the glyph, must still carry the placement"
            );
        }
        // The glyph's own cell holds the glyph and NO image.
        let hit = res.cell(ART_TOP, glyph_col);
        assert_eq!(hit.ch, 'W', "the printed character is what the cell holds");
        assert!(hit.image_id.is_none(), "the glyph's cell lost the image rather than layering over it");
        // …and so does every cell after it, though their placeholders are untouched.
        for col in glyph_col + 1..ART_LEFT + ART_COLS {
            let c = res.cell(ART_TOP, col);
            assert_eq!(c.ch, '\u{10EEEE}', "col {col} still holds its placeholder");
            assert!(
                c.image_id.is_none(),
                "col {col} is past the glyph, so the run is truncated and it draws no image"
            );
        }

        // And in pixels: the glyph's cell shows the terminal's default ground, not art.
        let canvas = draw(&s);
        let (x0, y0) = (u32::from(glyph_col) * CELL_W, u32::from(ART_TOP) * CELL_H);
        let cell: Vec<[u8; 4]> = (0..CELL_W)
            .flat_map(|x| (0..CELL_H).map(move |y| (x, y)))
            .map(|(x, y)| px(&canvas, x0 + x, y0 + y))
            .collect();
        assert!(cell.contains(&[255, 255, 255, 255]), "the 'W' is drawn");
        assert!(
            cell.iter().all(|p| *p == [255, 255, 255, 255] || *p == DEFAULT_BG),
            "the rest of the glyph's cell is bare screen — no art survives under it"
        );
    }

    /// A `c=2,r=1` pin placement of a solid colour at the home cell, with the
    /// alpha the caller asks for. Pin-anchored rather than virtual because virtual
    /// placements cannot overlap — a cell holds one placeholder — and overlap is
    /// the whole subject below.
    fn pinned(id: u32, rgba: [u8; 4]) -> String {
        let (w, h) = (2 * CELL_W, CELL_H);
        format!(
            "\x1b[1;1H\x1b_Ga=T,i={id},f=32,t=d,s={w},v={h},c=2,r=1,z=0,m=0;{}\x1b\\",
            b64(&rgba.repeat((w * h) as usize))
        )
    }

    /// Two placements at the same z: the protocol says the LOWER id is underneath,
    /// so the higher one's half-transparency composites onto it.
    ///
    /// Read off the protocol document rather than recalled: "If two images with the
    /// same z-index overlap then the image with the lower id is considered to have
    /// the lower z-index" (kitty graphics protocol, "Controlling displayed image
    /// layout"). Half-transparent on top on purpose — an opaque winner would pass
    /// for a rasteriser that simply drew the last placement it was handed, while a
    /// blend can only come out at this value if the red really is UNDERNEATH.
    #[test]
    fn overlapping_placements_at_one_z_stack_by_image_id() {
        let stream = format!("{}{}", pinned(10, [200, 0, 0, 255]), pinned(20, [0, 200, 0, 128]));
        let canvas = draw(&stream);
        // (0,200,0) at alpha 128 over (200,0,0): (200*127)/255 = 99, (200*128)/255 = 100.
        assert_eq!(
            px(&canvas, 0, 0),
            [99, 100, 0, 255],
            "image 20 must composite OVER image 10 — [200,0,0] is the lower id drawn last, \
             which is the wrong order, and [29,31,33]-ish is image 20 blended onto bare screen"
        );
    }

    /// The same bytes must draw the same picture.
    ///
    /// This is the case SQ-0968 needed and did not have. The draw list comes out of
    /// `ImageStorage::placements`, a `HashMap` whose iteration order is re-seeded
    /// per instance, so a sort on `z` alone leaves two same-z placements in a fresh
    /// random order on every call — measured at roughly 6 orderings in 10 runs of
    /// the identical stream, in ONE process. An instrument whose picture is a coin
    /// flip can show a superseded placement on top and a live one blended into it,
    /// which reads exactly like a defect the emitted bytes say is already gone.
    ///
    /// Four overlapping placements and eight renders: a broken sort passes only if
    /// all eight happen to land on one of the 24 permutations, which is roughly one
    /// run in 10^9. Not a probabilistic test in the direction that matters — the fix
    /// makes it pass every time, and only the failure is chance.
    #[test]
    fn the_same_bytes_always_draw_the_same_picture() {
        let stream: String = [
            pinned(11, [200, 0, 0, 255]),
            pinned(22, [0, 200, 0, 128]),
            pinned(33, [0, 0, 200, 128]),
            pinned(44, [200, 200, 0, 128]),
        ]
        .concat();
        let first = draw(&stream);
        for attempt in 1..8 {
            let again = draw(&stream);
            assert_eq!(
                again.as_raw(),
                first.as_raw(),
                "render {attempt} of the identical stream drew a different picture — the \
                 composite order is not a function of the bytes (SQ-0968)"
            );
        }
        // Non-vacuity: the pixel under the stack has to be a BLEND of all four, or
        // the four placements never overlapped and every render agreed trivially.
        let stacked = px(&first, 0, 0);
        assert_ne!(stacked, [200, 0, 0, 255], "the stack is not just its bottom image");
        assert_ne!(stacked, DEFAULT_BG, "the stack drew something");
    }

    /// A placement the next frame replaced is not in the picture.
    ///
    /// The literal question SQ-0968 was filed on: frame 1 puts art on those cells,
    /// frame 2 paints over them, and the picture must be frame 2's. It passes, and
    /// pinning it is the point — the harness has no frame buffer to carry anything
    /// forward, and this is the case that says so in one second instead of an
    /// afternoon of hand-decoding APC payloads.
    #[test]
    fn art_the_next_frame_painted_over_is_not_in_the_picture() {
        let mut s = gradient_frame();
        // Non-vacuity first: frame 1 alone really does put the gradient there.
        let (x0, y0) = (u32::from(ART_LEFT) * CELL_W, u32::from(ART_TOP) * CELL_H);
        assert_eq!(px(&draw(&s), x0, y0), [20, 0, 0, 255], "frame 1 draws the art");

        // Frame 2: the same cells, repainted as plain background.
        for row in 0..ART_ROWS {
            s.push_str(&format!(
                "\x1b[{};{}H\x1b[48;2;5;60;90m{}\x1b[0m",
                ART_TOP + row + 1,
                ART_LEFT + 1,
                " ".repeat(ART_COLS as usize)
            ));
        }
        let canvas = draw(&s);
        for dy in 0..u32::from(ART_ROWS) * CELL_H {
            for dx in 0..u32::from(ART_COLS) * CELL_W {
                assert_eq!(
                    px(&canvas, x0 + dx, y0 + dy),
                    [5, 60, 90, 255],
                    "pixel ({dx},{dy}) of the repainted rect still carries frame 1's art"
                );
            }
        }
    }

    /// The before/after pair the whole feature is for: two rasters, side by side,
    /// each still readable at its own coordinates.
    #[test]
    fn side_by_side_keeps_both_frames_intact() {
        let before = draw("\x1b[1;1H\x1b[48;2;10;20;30m \x1b[0m");
        let after = draw("\x1b[1;1H\x1b[48;2;90;80;70m \x1b[0m");
        let pair = raster::side_by_side(&before, &after);

        assert_eq!(pair.height(), before.height());
        assert!(pair.width() > before.width() + after.width(), "there is a gutter between them");
        assert_eq!(px(&pair, 0, 0), [10, 20, 30, 255], "the left frame is the before");
        assert_eq!(
            px(&pair, before.width() + (pair.width() - before.width() - after.width()), 0),
            [90, 80, 70, 255],
            "the right frame is the after"
        );
    }
}

#[cfg(not(unix))]
#[test]
fn the_real_capture_half_is_unix_only() {
    eprintln!(
        "SKIP: capturing a real stream needs a pty, which this platform does not have; \
         the hand-authored protocol tests still ran"
    );
}

#[cfg(unix)]
mod real_capture {
    use std::path::PathBuf;
    use std::time::Duration;

    use crate::pty_stream::{self, driver, oracle};

    /// Journey release 30, the Amiga disk image — the same fixture
    /// `pty_emitted_stream.rs` drives, so the two binaries measure one frame.
    /// NOT `journey.z6`, which is release 83 and a different build (SQ-0760).
    const STORY: &str = "Journey - The Quest Begins.adf";

    const COLS: u16 = 117;
    const ROWS: u16 = 64;

    fn out_dir() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/pty-capture");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// Both decoders on one real capture, on BOTH axes: which cells carry which
    /// SGR background, and which cells a renderer would put image pixels on.
    ///
    /// Image coverage used to be printed as a finding rather than asserted, because
    /// the two decoders legitimately disagreed: this capture left 33 runs of
    /// placeholder cells over rows 15–46, cols 47–113 that our decoder counted as
    /// the raster composite and a real terminal declined to draw at all, their
    /// anchoring cell having been overpainted by the chrome ring that replaced them
    /// (SQ-0772). With the placement now buffer-visible, the ring's frame unpaints
    /// those cells instead of stranding them, and the two readings coincide
    /// exactly — so the number is a tripwire again.
    #[test]
    fn our_decoder_and_a_real_terminal_agree_on_what_is_on_screen() {
        let story = driver::stories_dir().join(STORY);
        if !story.is_file() {
            eprintln!("SKIP: gitignored story missing at {}", story.display());
            return;
        }
        let user_dir = out_dir().join("oracle-user-dir");
        let _ = std::fs::remove_dir_all(&user_dir);

        let mut spec = driver::Spec::new(env!("CARGO_BIN_EXE_lanthorn"), &story, &user_dir);
        spec.cols = COLS;
        spec.rows = ROWS;
        spec.keys = vec![
            driver::Key::Wait(Duration::from_millis(1200)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(600)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(900)),
        ];

        let cap = driver::run(spec).expect("the pty harness should boot lanthorn");
        let term = pty_stream::decode_capture(&cap);
        let res = oracle::resolve(
            // SQ-0976: the oracle's terminal core links no zlib, so it must be
            // handed the stream with `o=z` undone or it drops every image.
            &cap.terminal_bytes(),
            cap.spec.cols,
            cap.spec.rows,
            u32::from(cap.spec.cell_w),
            u32::from(cap.spec.cell_h),
            Some((pty_stream::ANSWERED_FG, pty_stream::ANSWERED_BG)),
        );

        assert!(
            term.printed_cells > 1000,
            "only {} cells were ever printed — the app never drew a frame, so neither \
             decoder measured anything",
            term.printed_cells
        );

        let all = oracle::disagreements(&term, &res);
        let (bg, img): (Vec<&String>, Vec<&String>) =
            all.iter().partition(|d| d.starts_with("background"));

        eprintln!(
            "oracle: {} placement(s) a real terminal would draw\n{}",
            res.placements.len(),
            res.describe_placements()
        );

        for (axis, runs) in [("background", &bg), ("image-coverage", &img)] {
            assert!(
                runs.is_empty(),
                "our decoder and a real terminal read {} {axis} run(s) differently on the \
                 same bytes; one of the two is wrong:\n{}",
                runs.len(),
                runs.iter().take(40).map(|s| format!("  {s}")).collect::<Vec<_>>().join("\n")
            );
        }
    }
}
