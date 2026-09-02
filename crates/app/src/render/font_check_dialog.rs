//! The first-run font check (SQ-1104): "which of these two rows does your
//! terminal draw properly?"
//!
//! **Why lanthorn has to ask at all.** It writes characters; the font belongs to
//! the terminal, and no escape sequence asks "do you have U+F1A60". The
//! near-miss — write a glyph, read the cursor position back — measures the
//! terminal's assigned WIDTH, which catches a double-width emoji and misses tofu
//! entirely, because a missing-glyph box is still exactly one cell. The human eye
//! is the only oracle there is.
//!
//! **Why a comparison rather than a yes/no.** "Can you see this?" invites "well,
//! I see *something*" — and that something is often the terminal's font FALLBACK
//! drawing the codepoint out of an unrelated face at the wrong metrics, which is
//! a "yes" that yields a subtly crooked map for ever. Two rows side by side make
//! both failures obvious: tofu is a box next to a triangle, and a fallback glyph
//! is the one that does not line up with its neighbours.
//!
//! **Why one glyph per source family.** The presets are not one font.
//! `Arrows::nerdfont` is Material Design chevrons, `PortalGlyphs::nerdfont-stairs`
//! mixes Font Awesome (the marker and the question mark) with MDI (the stairs,
//! the door in, the runner out), and the Guiding Light's mark is MDI
//! `md-post_lamp`. A partially patched font carries some ranges and not others,
//! so the sample row is built out of the very [`SymbolSet`] the "yes" answer
//! installs — not a hand-written string that can drift from it.
//!
//! Deliberately thin, like [`crate::render::history_prompt`]: the chrome, the
//! focus ring, the button hit-rects and the keyboard ladder are all
//! [`crate::render::dialog::draw_dialog`]'s. Two drivers share this module — the
//! [`Overlay`](crate::render) impl in the run loop (`/run-font-check` and the
//! settings row) and the standalone pre-game loop in `startup::ask_font_check`,
//! exactly as the keep-this-download prompt is driven from both places.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{ButtonId, DialogButton, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;
use crate::symbols::SymbolSet;

const MIN_W: u16 = 52;
const MIN_H: u16 = 14;
const DIALOG_W: u16 = 66;
const DIALOG_H: u16 = 14;

/// Nerd Fonts `md-post_lamp` — the mark of Lanthorn's Guiding Light on a patched
/// font. Verified against the font's own `post` table rather than a cheat sheet
/// (SQ-1045); `symbols::SymbolSet::assist_gutter` documents why the unpatched
/// default is `●` instead.
pub const ASSIST_LAMP: char = '\u{F1A60}';

/// The two preset names a "yes" writes into `style.toml`'s `[map]` — the arrow
/// set and the portal icon set. `nerdfont-stairs` rather than `nerdfont` because
/// it gives up/down/in/out four DISTINCT icons; it also spans both source
/// families on its own (Font Awesome's circle and question mark, MDI's stairs
/// and doors), so the sample row that shows it samples both.
pub const NERD_ARROWS: &str = "nerdfont";
pub const NERD_PORTALS: &str = "nerdfont-stairs";

/// …and the third: the pane-border toggle controls (SQ-1123).
pub const NERD_CONTROLS: &str = "nerdfont";

/// …and the fourth: the story picker's row badges (SQ-1159, cut to three by
/// SQ-1160). Three Material Design icons — a floppy for a save, and a lightbulb
/// pair for the hint slot's two states.
pub const NERD_BADGES: &str = "nerdfont";

/// …and the fifth: the MAP pane's own control cluster (SQ-1148) — room numbers,
/// recentre, the two zooms and the two views. Shares `control_icons` with
/// [`NERD_CONTROLS`], so a "yes" reaches it by the same write;
/// `font_check::a_yes_reaches_the_map_controls_too` walks that end to end.
pub const NERD_MAP_CONTROLS: &str = "nerdfont";

// **THE RULE, because it has already drifted once.** Any glyph the app can draw
// out of a patched font belongs in the sample row, or the check is asking about
// a subset and applying the answer to the whole.
//
// Two clusters were admitted here on an argument instead of a slot — the border
// controls and the picker badges, both excused as "the row already samples MDI,
// and a face that draws MDI draws these". SQ-1148 tested that argument rather
// than inheriting it, and the finding is worth writing down because it is only
// half wrong: across the forty-four patched faces installed on the machine this
// was measured on, NOT ONE draws the whole sample row and misses any of the new
// map glyphs. As a claim about `cmap` coverage — about tofu — it holds up.
//
// It answers the wrong question. This dialog exists because tofu is not the only
// failure, and the module note above says so: the other is the terminal's font
// FALLBACK drawing a codepoint out of an unrelated face at the wrong metrics,
// which is a "yes" that yields a subtly crooked map for ever. Fallback is
// resolved by the terminal PER CODEPOINT, not per range, so a face's coverage of
// U+F1A60 predicts nothing about how U+F02C1 will be drawn — and nothing on our
// side can see the difference. Only the eye in front of the screen can, and only
// if the glyph is on the screen.
//
// So the map cluster gets a slot. Range reasoning cannot answer a metrics
// question, and an argument in a comment is not a thing the player can look at.

/// The map cluster's seven patched marks, in the sample row's slot order.
/// Sampled from the preset the "yes" installs rather than written out here, for
/// the same reason as every other slot — see [`sample_glyphs`].
///
/// **Both halves of each two-mode pair are here, so the plain row repeats a
/// mark** (`# #`, `M M`) where the patched row shows a shape change. That is not
/// a redundant slot: it is the honest picture of what each answer installs, and
/// the rule at the top of this module — every glyph an answer installs is in the
/// row — is what puts `md-numeric_off` and `md-grid_off` on screen at all.
fn map_control_slots(set: &crate::symbols::MapControlGlyphs) -> [char; 7] {
    [
        set.room_numbers_on, set.room_numbers_off, set.centre,
        set.zoom_out, set.zoom_in, set.view_matrix, set.view_drawn,
    ]
}

pub struct FontCheckRects {
    pub area: Rect,
    pub close: Option<Rect>,
    /// "Row 1" — the patched-font answer.
    pub nerd: Option<Rect>,
    /// "Row 2" — the plain answer, and what Esc and the close box mean.
    pub plain: Option<Rect>,
}

/// What a key press or click on the font check means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontCheckAction {
    /// Row 1 drew properly: install the Nerd Font presets.
    Nerd,
    /// Row 2 drew properly (or the player declined): keep the plain glyphs.
    Plain,
    /// Nothing — the caller has already handled focus movement.
    None,
}

/// The glyphs one answer would put on the map, in a fixed slot order:
/// the four cardinal arrows, then the portal marker, up, down, in, out and
/// unknown, then the Guiding Light's mark, then the map pane's seven control
/// marks, then the diagonal corner stubs.
///
/// Built from the [`SymbolSet`] each answer actually installs, so a later
/// improvement to a preset changes what the prompt shows rather than leaving it
/// advertising glyphs the map no longer draws.
///
/// **The last four slots are the same in both rows, and that is deliberate**
/// (SQ-1140). The diagonal corner stubs are `PathGlyphs`' `diag_*` — Unicode 13
/// Legacy Computing (U+1FBA0–1FBA3), which every preset spells identically
/// because they are not a patched-font question: they are BASE typeface
/// coverage, so a Nerd Font patch supplies none of them and answering this
/// prompt either way cannot help. Measured on one machine's 43 installed faces,
/// six carried them — all six Iosevka, and not Symbols Nerd Font Mono.
///
/// So they are here to be SEEN, not to be chosen between. The prompt asks which
/// row draws properly; these four sit in both rows and answer a different
/// question the player can now at least ask — and `map.diagonal_corners = false`
/// in `style.toml` is the one-line answer if they are tofu.
fn sample_glyphs(nerdfont: bool) -> Vec<char> {
    // **`from_preset_names` does NOT carry `control_icons`** — it takes the box,
    // arrow, portal and path presets and leaves every other category at its
    // default. So the map cluster has to be resolved by name here rather than
    // read off `set`, which would have silently sampled the PLAIN marks in the
    // patched row: seven slots showing `# # ¤ − + M M` in both rows, identical,
    // inviting a "yes" on the strength of glyphs the answer does not install.
    // The case below caught exactly that, which is the argument for having it.
    let (set, mark, map_controls) = if nerdfont {
        (
            SymbolSet::from_preset_names("rounded", NERD_ARROWS, NERD_PORTALS, "light"),
            ASSIST_LAMP,
            crate::symbols::MapControlGlyphs::preset(NERD_MAP_CONTROLS)
                .unwrap_or_else(|| SymbolSet::default().map_controls),
        )
    } else {
        let d = SymbolSet::default();
        let (mark, mc) = (d.assist_gutter, d.map_controls);
        (d, mark, mc)
    };
    vec![
        set.arrows.north, set.arrows.south, set.arrows.east, set.arrows.west,
        set.portal.marker, set.portal.up, set.portal.down,
        set.portal.in_, set.portal.out, set.portal.unknown,
        mark,
    ]
    .into_iter()
    .chain(map_control_slots(&map_controls))
    .chain([set.path.diag_ul, set.path.diag_ur, set.path.diag_ll, set.path.diag_lr])
    .collect()
}

/// One sample row as it is drawn: the slots space-separated, in five groups
/// (arrows · portals · the Guiding Light's mark · the map cluster · the diagonal
/// stubs) so a fallback glyph's wrong advance shows up as a group that does not
/// line up with the row above it. Two spaces between groups rather than three,
/// which is what keeps twenty-two slots inside `DIALOG_W`.
pub fn sample_row(nerdfont: bool) -> String {
    let g = sample_glyphs(nerdfont);
    let join = |r: &[char]| r.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" ");
    format!(
        "{}  {}  {}  {}  {}",
        join(&g[0..4]),
        join(&g[4..10]),
        join(&g[10..11]),
        join(&g[11..18]),
        join(&g[18..22]),
    )
}

/// Draw the font check centred over `area`, or `None` when it is closed or the
/// pane is too small to hold it.
pub fn draw_font_check(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<FontCheckRects> {
    if !state.overlays.font_check {
        return None;
    }
    draw_font_check_always(state, area, buf)
}

/// [`draw_font_check`] without the open/closed test — the form `startup`'s
/// standalone pre-game loop uses, where the prompt is the only thing on screen
/// and there is no `AppState` flag governing it.
pub fn draw_font_check_always(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<FontCheckRects> {
    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = DIALOG_H.min(area.height.saturating_sub(2));
    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);
    let buttons = &[
        DialogButton { id: ButtonId::Ok, label: "Row 1 looks right" },
        DialogButton { id: ButtonId::Cancel, label: "Row 2 looks right" },
    ];
    let spec = DialogSpec {
        title: "Your terminal's font",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        // Row 2 is the default: it is the answer that changes nothing, and the
        // one every font can draw. A player who hits Enter without reading has
        // not been talked into a map made of boxes.
        default: Some(ButtonId::Cancel),
        focus: Some(state.overlays.dialog_focus),
        field: None,
    };
    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    let body = state.colors.theme.get("dialog.background").style;
    let sample = body.patch(state.colors.theme.get("dialog.font_check.sample").style);

    // Say what is being asked and what the two answers mean, then get out of the
    // way: the rows are the question, and the prose is only here so the player
    // knows what "looks right" means (drawn, not a box, and lined up).
    let intro = [
        "lanthorn draws its map with these glyphs, but it cannot",
        "read your terminal's font — so it has to ask you.",
        "",
    ];
    let outro = [
        "",
        "Row 1 needs a patched \"Nerd Font\". Pick it only if every",
        "glyph is drawn AND the two rows line up column for column;",
        "an empty box, or a glyph borrowed from another face at the",
        "wrong width, means row 2 is the one that fits your terminal.",
    ];

    let mut y = content.y;
    let mut put = |line: &str, style: ratatui::style::Style, y: &mut u16| {
        if *y < content.bottom() {
            crate::render::draw_str_clipped(buf, content.x, *y, line, style, content);
        }
        *y += 1;
    };
    for line in intro {
        put(line, body, &mut y);
    }
    put(&format!("  1.  {}", sample_row(true)), sample, &mut y);
    put(&format!("  2.  {}", sample_row(false)), sample, &mut y);
    for line in outro {
        put(line, body, &mut y);
    }

    Some(FontCheckRects {
        area: rects.area,
        close: rects.close,
        nerd: rects.buttons.iter().find(|(id, _)| *id == ButtonId::Ok).map(|(_, r)| *r),
        plain: rects.buttons.iter().find(|(id, _)| *id == ButtonId::Cancel).map(|(_, r)| *r),
    })
}

/// Map a key to a [`FontCheckAction`] given the focused button index
/// (`0 = row 1`, `1 = row 2`).
///
/// Esc means row 2 — the plain glyphs — rather than "ask me again later: a player
/// who dismisses the question has answered it, and a question that comes back
/// every launch is worse than either answer. Tab/Shift-Tab belong to the caller
/// (they move `dialog_focus`), and Space is left alone because it is
/// widget-reserved, per the shared-chrome convention.
pub fn font_check_key_focused(code: crossterm::event::KeyCode, focus: usize) -> FontCheckAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc => FontCheckAction::Plain,
        KeyCode::Enter => {
            if focus == 0 {
                FontCheckAction::Nerd
            } else {
                FontCheckAction::Plain
            }
        }
        _ => FontCheckAction::None,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;
    use ratatui::{backend::TestBackend, Terminal};

    /// The two rows must have the same number of SLOTS, or the comparison the
    /// whole design rests on — "do these line up?" — cannot be made.
    #[test]
    fn both_rows_offer_the_same_slots() {
        assert_eq!(sample_glyphs(true).len(), sample_glyphs(false).len());
        assert_eq!(sample_row(true).chars().count(), sample_row(false).chars().count());
    }

    /// One glyph per SOURCE FAMILY, which is the reason the row is a row and not
    /// a single icon. A partially patched font carries some ranges and not
    /// others, so the sample has to span every range the answer installs:
    /// MDI chevrons (the arrows), Font Awesome (the portal marker), MDI again
    /// from a different block (the stairs), and the Guiding Light's lamp.
    #[test]
    fn the_nerd_row_spans_every_family_the_answer_installs() {
        let g = sample_glyphs(true);
        assert!(g.contains(&'\u{F0738}'), "MDI arrow-up-bold-box, from Arrows::nerdfont");
        assert!(g.contains(&'\u{F111}'), "Font Awesome circle, from PortalGlyphs::nerdfont-stairs");
        assert!(g.contains(&'\u{F12BD}'), "MDI stairs-up, from PortalGlyphs::nerdfont-stairs");
        assert!(g.contains(&ASSIST_LAMP), "md-post_lamp, the Guiding Light's mark");
        assert!(g.contains(&'\u{F02C1}'), "md-grid, from MapControlGlyphs::nerdfont");
        // And the plain row must be free of the private-use area entirely, or it
        // is not the answer that works everywhere.
        assert!(
            sample_glyphs(false).iter().all(|c| !('\u{E000}'..='\u{F8FF}').contains(c)
                && !('\u{F0000}'..='\u{FFFFD}').contains(c)),
            "the plain row must not need a patched font"
        );
    }

    /// **SQ-1148: every glyph a "yes" installs is IN the row the player judges.**
    ///
    /// This is the guard for the rule stated at the top of the module, and it is
    /// written against the presets rather than against a list of codepoints, so a
    /// cluster added later fails here until it is sampled — which is the only way
    /// this can keep up. Two clusters were admitted on an argument once already
    /// (the border controls and the picker badges, both excused because the row
    /// "already samples MDI"); the map cluster was very nearly the third.
    ///
    /// The border controls and the badges are deliberately NOT required here.
    /// They are chrome the player meets after the prompt, not map glyphs, and
    /// bringing them in would put twenty more slots in a sixty-six-column dialog
    /// — but the map cluster IS on the map, which is what the row is a picture
    /// of.
    #[test]
    fn every_map_glyph_a_yes_installs_appears_in_the_row_the_player_judges() {
        let g = sample_glyphs(true);
        let set = SymbolSet::from_preset_names("rounded", NERD_ARROWS, NERD_PORTALS, "light");
        let want = crate::symbols::MapControlGlyphs::preset(NERD_MAP_CONTROLS)
            .expect("the preset the answer names");

        for (slot, c) in [
            ("room_numbers_on", want.room_numbers_on),
            ("room_numbers_off", want.room_numbers_off),
            ("centre", want.centre),
            ("zoom_out", want.zoom_out),
            ("zoom_in", want.zoom_in),
            ("view_matrix", want.view_matrix),
            ("view_drawn", want.view_drawn),
        ] {
            assert!(g.contains(&c), "{slot} (U+{:05X}) is installed but never shown", c as u32);
        }

        // …and the arrows and portals the row has always carried, restated the
        // same way so that a preset change moves the assertion with it rather
        // than leaving it pinned to a codepoint that is no longer installed.
        for c in [set.arrows.north, set.arrows.south, set.arrows.east, set.arrows.west] {
            assert!(g.contains(&c), "an installed arrow is not in the row");
        }
        for c in [set.portal.marker, set.portal.up, set.portal.down, set.portal.in_, set.portal.out] {
            assert!(g.contains(&c), "an installed portal icon is not in the row");
        }
    }

    /// The plain row shows the same seven slots, so the comparison is a
    /// comparison: `#` against md-numeric, `¤` against md-crosshairs, and so on.
    /// A patched-only slot would be a blank in row 2 and would read as row 2
    /// failing.
    ///
    /// **The plain row therefore repeats two of its marks** — `# #` and `M M` —
    /// because ASCII has no off-shape for either and colour carries the state
    /// there (SQ-1148). That is the honest picture: the row samples the SLOTS an
    /// answer fills, not the distinct glyphs it happens to use, and collapsing
    /// the duplicate would leave row 2 a slot short of row 1 and unalignable
    /// against it — which is the one thing this whole dialog is for.
    #[test]
    fn the_plain_row_shows_the_map_cluster_too() {
        let g = sample_glyphs(false);
        let want = SymbolSet::default().map_controls;
        for c in [
            want.room_numbers_on, want.room_numbers_off, want.centre,
            want.zoom_out, want.zoom_in, want.view_matrix, want.view_drawn,
        ] {
            assert!(g.contains(&c), "U+{:04X} missing from the plain row", c as u32);
        }
        // Two slots each, not one: the plain preset spells one mark in both
        // halves of each two-mode pair, and both halves are sampled.
        assert_eq!(g.iter().filter(|&&c| c == '#').count(), 2, "both number slots");
        assert_eq!(g.iter().filter(|&&c| c == 'M').count(), 2, "both view slots");
    }

    /// Both rows still fit the dialog. Twenty-two slots at two spaces between
    /// groups is the reason the separator narrowed from three; if a later slot
    /// pushes past this, narrow the groups again or drop a slot — do not widen
    /// the dialog past what a 80x24 terminal can centre.
    #[test]
    fn the_row_still_fits_the_dialog() {
        for nerd in [true, false] {
            let w = sample_row(nerd).chars().count() + "  1.  ".chars().count();
            assert!(w <= DIALOG_W as usize - 4, "row (nerd={nerd}) is {w} wide, dialog is {DIALOG_W}");
        }
    }

    /// SQ-1140: the diagonal corner stubs appear, and appear in BOTH rows.
    ///
    /// Not a preset choice and never was — `PathGlyphs` spells U+1FBA0–1FBA3 the
    /// same in every preset, so no answer to this prompt changes them. Showing
    /// them is the only thing the dialog can usefully do, and showing them in one
    /// row only would falsely imply the other row avoids them.
    #[test]
    fn both_rows_show_the_diagonal_stubs_because_no_answer_changes_them() {
        let (nerd, plain) = (sample_glyphs(true), sample_glyphs(false));
        let stubs = ['\u{1FBA0}', '\u{1FBA1}', '\u{1FBA2}', '\u{1FBA3}'];
        for s in stubs {
            assert!(nerd.contains(&s), "U+{:04X} missing from the patched row", s as u32);
            assert!(plain.contains(&s), "U+{:04X} missing from the plain row", s as u32);
        }
        // The last FOUR slots, named from the end so a cluster added ahead of
        // them cannot silently slide this window off the stubs.
        assert_eq!(
            nerd[nerd.len() - 4..],
            plain[plain.len() - 4..],
            "the stub slots are identical in both rows — they are not the question being asked"
        );
        // And they are NOT private-use, which is the whole point: a patched font
        // supplies none of them, so the assertion above about the plain row being
        // free of the PUA must keep passing with these in it.
        assert!(stubs.iter().all(|c| !('\u{F0000}'..='\u{FFFFD}').contains(c)));
    }

    /// Esc is an answer, not a deferral (see `font_check_key_focused`).
    #[test]
    fn esc_and_the_second_button_both_mean_plain() {
        assert_eq!(font_check_key_focused(KeyCode::Esc, 0), FontCheckAction::Plain);
        assert_eq!(font_check_key_focused(KeyCode::Esc, 1), FontCheckAction::Plain);
        assert_eq!(font_check_key_focused(KeyCode::Enter, 0), FontCheckAction::Nerd);
        assert_eq!(font_check_key_focused(KeyCode::Enter, 1), FontCheckAction::Plain);
        // Space is widget-reserved and must not answer for the player.
        assert_eq!(font_check_key_focused(KeyCode::Char(' '), 0), FontCheckAction::None);
    }

    #[test]
    fn draws_both_rows_and_hands_back_two_button_rects() {
        let mut state = AppState::default();
        state.overlays.font_check = true;
        let mut term = Terminal::new(TestBackend::new(80, 30)).unwrap();
        let mut rects = None;
        term.draw(|f| rects = draw_font_check(&state, f.area(), f.buffer_mut())).unwrap();
        let r = rects.expect("the prompt draws at 80x30");
        assert!(r.nerd.is_some() && r.plain.is_some(), "both answers are clickable");

        let text: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains(&sample_row(true)), "row 1 is on screen");
        assert!(text.contains(&sample_row(false)), "row 2 is on screen");
    }

    /// A pane too small for the rows draws nothing rather than a truncated
    /// comparison — half a row is not a question anyone can answer.
    #[test]
    fn a_tiny_pane_draws_nothing() {
        let mut state = AppState::default();
        state.overlays.font_check = true;
        let mut term = Terminal::new(TestBackend::new(30, 8)).unwrap();
        let mut rects = None;
        term.draw(|f| rects = draw_font_check(&state, f.area(), f.buffer_mut())).unwrap();
        assert!(rects.is_none());
    }
}
