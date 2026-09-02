//! The launch-options dialog (SQ-0789): the boot-time choices a story can only
//! be *started* with, shown on demand before it boots.
//!
//! Not to be confused with `launch_dialog.rs`, which is the "Resume saved game?"
//! prompt shown once a story is already running.
//!
//! Opened from the story browser on an explicit gesture — Shift-Enter, or a
//! double right-click — never automatically. Plain Enter and a double left-click
//! launch exactly as they always did, so nobody who does not want this dialog
//! ever meets it, and there is no cleverness about when it is "worth" showing.
//!
//! The state, the key model and every decision live in
//! [`crate::launch_options`]; this file only draws.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::colors::ColorScheme;
use crate::launch_options::LaunchOptionsState;
use crate::render::dialog::{
    draw_dialog, ButtonId, DialogButton, DialogSpec, DialogStyle, Placement,
};
use crate::render::draw_str_clipped;

/// Widest the dialog will grow, and the narrowest it will render at all.
const MAX_W: u16 = 78;
const MIN_W: u16 = 44;

/// The art list's two text columns: the archive's name, then its rendition.
///
/// **The name column is 20 rather than the 14 it was** (SQ-0865), and the width
/// is set by the DEFAULT row rather than by any filename. That row now reads
/// `Automatic — CPic.data (Amiga)` — 29 characters at its longest across the
/// corpus, `Pic.data (Mac B&W)` and `Zork0.blb (Blorb)` being the other two
/// contenders — and it has to end exactly where the rendition column does, or
/// the picture counts stop lining up in one column, which is the complaint that
/// started this. `NAME_W + 1 + REND_W` is that 29, and it is the narrowest pair
/// that reaches it while leaving the rendition column able to hold `Mac B&W`.
///
/// The cost is six spaces of padding after a nine-character `ZORK0.EG1`, which
/// is a table with room in it; the alternative was a default row whose count sat
/// four columns right of every other row's.
const NAME_W: usize = 20;
const REND_W: usize = 8;
/// The span the default row's `Automatic — …` phrase fills: both name columns
/// and the space between them, so its picture count lands in the same column as
/// every candidate's.
const DEFAULT_W: usize = NAME_W + 1 + REND_W;
/// Tall enough for a long art list without becoming the whole screen; the list
/// scrolls past this.
const MAX_H: u16 = 24;
const MIN_H: u16 = 10;

/// Hit-rects the picker needs for mouse handling.
pub struct LaunchOptionsRects {
    pub area: Rect,
    pub close: Option<Rect>,
    pub buttons: Vec<(ButtonId, Rect)>,
    /// `(flat cursor index, rect)` for each row actually drawn. The art list
    /// scrolls, so this is the visible window, not every row.
    pub rows: Vec<(usize, Rect)>,
}

/// The checkbox glyphs. Plain ASCII inside brackets rather than ☑/☐: the dialog
/// is drawn in the story browser, which runs before any font probing, and a
/// missing box glyph would leave the one control whose state matters unreadable.
fn checkbox(on: bool) -> &'static str {
    if on { "[x]" } else { "[ ]" }
}

/// A trailing note, spaced off the column before it — and nothing at all when
/// there is no note, so an ordinary loose archive's row simply ends.
fn note(s: &str) -> String {
    if s.is_empty() { String::new() } else { format!("  {s}") }
}

/// Draw the dialog centred over `area`. `None` when the terminal is too small
/// for it to be honest about what it is offering.
pub fn draw_launch_options(
    st: &LaunchOptionsState,
    area: Rect,
    cs: &ColorScheme,
    buf: &mut Buffer,
) -> Option<LaunchOptionsRects> {
    // Rows: art choices (+ "inherit"), a blank, the interpreter line, its
    // provenance line, a blank, the checkbox and the escape-hatch note, plus a
    // caveat line for each thing the current choice warrants one for. Plus
    // dialog chrome + button row.
    //
    // Counting the caveats HERE is what keeps a short list from scrolling: the
    // list is filtered to this story's own archives now — four for Zork Zero,
    // five for Arthur — so it fits, and a dialog that paged a five-row list
    // because a warning line took its last row would be paging for nothing.
    let caveat_lines = u16::from(st.clears_inherited_art())
        + u16::from(st.chosen_art().is_some_and(|c| c.caveat().is_some()));
    let body_rows = st.row_count() as u16 + 4 + caveat_lines;
    let w = MAX_W.min(area.width.saturating_sub(4));
    let h = (body_rows + 3).min(MAX_H).min(area.height.saturating_sub(2));
    if w < MIN_W || h < MIN_H {
        return None;
    }

    let ds = DialogStyle::from_colors(cs);
    let buttons = &[
        DialogButton { id: ButtonId::PlayAgain, label: "Play" },
        DialogButton { id: ButtonId::Cancel, label: "Cancel" },
    ];
    let spec = DialogSpec {
        title: &format!("Launch options — {}", st.title),
        placement: Placement::Centered { w, h },
        buttons,
        show_close: true,
        default: Some(ButtonId::PlayAgain),
        focus: Some(st.focus),
        field: None,
    };
    let rects = draw_dialog(buf, area, &spec, &ds);
    let content = rects.content;

    let text = cs.theme.get("dialog.background").style;
    let header = cs.theme.get("dialog.list_header").style;
    let selected = cs.theme.get("dialog.list_selected").style;
    let dim = cs.theme.get("dialog.list_footer").style;
    let caveat = cs.theme.get("dialog.launch_caveat").style;

    let cursor = st.cursor_index();
    let mut rows: Vec<(usize, Rect)> = Vec::new();
    let mut y = content.y;
    let line = |buf: &mut Buffer, s: &str, style, y: &mut u16| {
        if *y < content.bottom() {
            draw_str_clipped(buf, content.x, *y, s, style, content);
            *y += 1;
        }
    };
    // A row the cursor can land on: painted across the full content width so the
    // selection reads as a bar, not as coloured words.
    let option_row = |buf: &mut Buffer,
                      idx: usize,
                      s: &str,
                      rows: &mut Vec<(usize, Rect)>,
                      y: &mut u16| {
        if *y >= content.bottom() {
            return;
        }
        let style = if idx == cursor { selected } else { text };
        for x in content.x..content.right() {
            if let Some(c) = buf.cell_mut((x, *y)) {
                c.set_symbol(" ").set_style(style);
            }
        }
        draw_str_clipped(buf, content.x, *y, s, style, content);
        rows.push((idx, Rect::new(content.x, *y, content.width, 1)));
        *y += 1;
    };

    // The tail below the art list is fixed-size and must always be reachable:
    // the interpreter number and the checkbox are the whole point of the dialog.
    // The list is short now that it is filtered to this story's own archives —
    // five at the very most in the real library — but a folder can hold anything,
    // so it still scrolls; nothing else does.
    // blank, interpreter, provenance, checkbox, escape hatch
    const TAIL: u16 = 5;
    // The "N more above/below" markers cost rows too, and only exist when the
    // list actually scrolls — so budget for them only then, in two passes.
    let fixed = 1 + TAIL + caveat_lines; // + the "Artwork" heading
    let art_total = st.candidates.len() + 1;
    let loose = content.height.saturating_sub(fixed);
    let marks = if art_total > usize::from(loose) { 2 } else { 0 };
    let art_rows = usize::from(content.height.saturating_sub(fixed + marks).max(1));

    // The heading says what the list IS, in the one row it already costs: the
    // archives beside this story that carry its name. It is still a name guess
    // and the user is still the one asserting the pairing — "detected for" says
    // that, where "the archives for" would imply a guarantee the file format
    // cannot give.
    line(buf, "Artwork — archives detected for this story", header, &mut y);
    // Scroll the art window to keep the cursor visible, exactly as a list does.
    // Indices here are the flat cursor indices: 0 is "inherit", 1.. the
    // candidates in the order `discover_art_candidates` sorted them.
    let art_cursor = cursor.min(art_total - 1);
    let first = art_cursor.saturating_sub(art_rows.saturating_sub(1)).min(art_total.saturating_sub(art_rows));
    let last = (first + art_rows).min(art_total);
    if first > 0 {
        line(buf, &format!("  ↑ {first} more above"), dim, &mut y);
    }
    for idx in first..last {
        match idx.checked_sub(1) {
            // Row 0 is "inherit": whatever the story resolves on its own — its
            // Blorb, or the archive the release it came off supplies.
            //
            // It NAMES that archive (SQ-0865). It used to read "Use this story's
            // own art (Blorb / disk image)", which was prose among columns and
            // was the one row that said nothing about what accepting it would
            // do. What it names is resolved by the boot path itself, once, when
            // the dialog opens — see `launch_options::resolved_default_art`.
            None => {
                // Selected or not, like every other radio row; `·` rather than
                // `•` is what marks this one as the automatic choice.
                let mark = if st.art == 0 { "(·)" } else { "( )" };
                let label = match &st.default_art {
                    Some(d) => format!(
                        "  {mark} {:<DEFAULT_W$} {:>4} pictures{}",
                        format!("Automatic — {} ({})", d.filename, d.rendition),
                        d.pictures,
                        note(&d.medium_note()),
                    ),
                    None => format!("  {mark} Automatic — no artwork found"),
                };
                option_row(buf, 0, &label, &mut rows, &mut y);
            }
            Some(i) => {
                let c = &st.candidates[i];
                let mark = if st.art == idx { "(•)" } else { "( )" };
                // Where an archive with no path of its own lives: `CPic.data` is
                // not in the folder the story appears to be in, it is inside the
                // volume (SQ-0843) — and since SQ-0862 that volume may be a
                // sibling of the one booted, so the row says WHICH (SQ-0865).
                let label = format!(
                    "  {mark} {:<NAME_W$} {:<REND_W$} {:>4} pictures{}{}",
                    c.filename,
                    c.rendition,
                    c.pictures,
                    crate::launch_options::parts_note(c),
                    note(&crate::launch_options::medium_note(c)),
                );
                option_row(buf, idx, &label, &mut rows, &mut y);
            }
        }
    }
    if last < art_total {
        line(buf, &format!("  ↓ {} more below", art_total - last), dim, &mut y);
    }
    // Only when there is genuinely NOTHING — the automatic row counts. An
    // empty candidate list no longer means no artwork: a release carrying one
    // archive has it named by row 0 and nothing left to list beside it, and
    // saying "none detected" directly under a row naming one would be a plain
    // contradiction on screen.
    if st.candidates.is_empty() && st.default_art.is_none() {
        line(buf, "  no picture archives detected for this story", dim, &mut y);
    }
    // Selecting "inherit" over a sidecar that names an archive cannot be
    // expressed as a session override — there is no "override with nothing" — so
    // say plainly that only the checkbox can carry it, rather than appearing to
    // do something this launch and then not doing it.
    if st.clears_inherited_art() {
        line(buf, "  tick the box below to forget the saved choice", caveat, &mut y);
    }
    if let Some(msg) = st.chosen_art().and_then(|c| c.caveat()) {
        line(buf, &format!("  ⚠ {msg}"), caveat, &mut y);
    }

    // Pin the tail to the bottom of the content so it never slides with the list.
    y = content.bottom().saturating_sub(TAIL).max(y);
    // The number, and where it came from. Showing the provenance is the point:
    // picking an Amiga archive MOVES an auto interpreter to the Amiga, and doing
    // that silently — changing the emulated machine because someone chose
    // prettier art — is the surprise this dialog exists to prevent.
    let shown = match st.interpreter {
        Some(n) => format!("{n} {}", crate::launch_options::interpreter_name(n)),
        None => "auto".to_string(),
    };
    y += 1;
    option_row(buf, st.candidates.len() + 1, &format!("  Interpreter   {shown}"), &mut rows, &mut y);
    let derived = match st.derived() {
        Some((n, src)) => format!(
            "      header 0x1E = {n} ({}) — {}",
            crate::launch_options::interpreter_name(n),
            src.label()
        ),
        None => "      this story has no Z-machine header to advertise it in".to_string(),
    };
    line(buf, &derived, dim, &mut y);

    let persist = format!("  {} Save as this game's default", checkbox(st.persist));
    option_row(buf, st.candidates.len() + 2, &persist, &mut rows, &mut y);
    // The escape hatch, said on screen rather than left to the docs. The list
    // above is filtered by name, so an archive under an unrelated name — the
    // renamed `FMVPOKER.EG1` case — will not appear in it, and someone who has
    // one must not be left thinking it is unreachable. Naming a path outright
    // has always been the durable form and still wins over everything.
    line(buf, "  another name? pictures = \"…\" in the game's config, or --pictures", dim, &mut y);

    Some(LaunchOptionsRects {
        area: rects.area,
        close: rects.close,
        buttons: rects.buttons,
        rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn tmp(tag: &str) -> std::path::PathBuf {
        crate::scratch_dir(&format!("lodlg-{tag}"))
    }

    fn render(st: &LaunchOptionsState, w: u16, h: u16) -> (String, Option<LaunchOptionsRects>) {
        let cs = ColorScheme::terminal_default();
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut rects = None;
        term.draw(|f| {
            rects = draw_launch_options(st, f.area(), &cs, f.buffer_mut());
        })
        .unwrap();
        let text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .flat_map(|c| c.symbol().chars())
            .collect();
        (text, rects)
    }

    #[test]
    fn the_dialog_names_the_story_the_choices_and_the_derived_machine() {
        let dir = tmp("draw");
        let story = dir.join("story.z6");
        std::fs::write(&story, b"x").unwrap();
        let st = LaunchOptionsState::new("Zork Zero", &story, None, None, Some(6), None);
        let (text, rects) = render(&st, 90, 24);
        let r = rects.expect("dialog renders at 90x24");
        assert!(text.contains("Launch options"), "title: {text:?}");
        assert!(text.contains("Zork Zero"));
        assert!(text.contains("Artwork"));
        assert!(text.contains("Interpreter"));
        // The provenance is not optional — it is the whole reason the number is
        // shown at all.
        assert!(text.contains("header 0x1E = 6"), "derived number: {text:?}");
        assert!(text.contains("default for this story"), "provenance: {text:?}");
        assert!(text.contains("Save as this game's default"));
        assert!(text.contains("[ ]"), "checkbox starts clear");
        assert_eq!(r.rows.len(), st.row_count(), "one hit-rect per selectable row");
        assert_eq!(r.buttons.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_checkbox_shows_its_state() {
        let dir = tmp("check");
        let story = dir.join("story.z6");
        std::fs::write(&story, b"x").unwrap();
        let mut st = LaunchOptionsState::new("Story", &story, None, None, Some(5), None);
        st.persist = true;
        let (text, _) = render(&st, 90, 24);
        assert!(text.contains("[x]"), "ticked checkbox: {text:?}");
        // A v5 story's default is 1 (DECSystem-20), not 6 — Frotz's rule, both halves.
        assert!(text.contains("header 0x1E = 1"), "{text:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_explicit_number_reports_itself_as_explicit() {
        let dir = tmp("explicit");
        let story = dir.join("story.z6");
        std::fs::write(&story, b"x").unwrap();
        let mut st = LaunchOptionsState::new("Story", &story, None, None, Some(6), None);
        st.interpreter = Some(4);
        let (text, _) = render(&st, 90, 24);
        assert!(text.contains("Interpreter   4 Amiga"), "{text:?}");
        assert!(text.contains("set here"), "provenance says explicit: {text:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_terminal_too_small_draws_nothing_rather_than_a_lie() {
        let dir = tmp("small");
        let story = dir.join("story.z6");
        std::fs::write(&story, b"x").unwrap();
        let st = LaunchOptionsState::new("Story", &story, None, None, Some(6), None);
        let (_, rects) = render(&st, 30, 8);
        assert!(rects.is_none(), "too small → no dialog");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
