//! The region prompt (SQ-0439): the one modal that asks which rooms move, and where to.
//!
//! Three quite different questions come through here and they are all the same two questions —
//! *which rooms*, and *onto what layer* — so they share one piece of chrome:
//!
//! * the map's own suggestion that a set of rooms wants a layer of its own, whose options are the
//!   destinations and whose buttons are the three settled outcomes (separate now / ask again next
//!   crossing / never ask);
//! * a manual `move-region` where several passages lead into the selected room, whose options are
//!   those passages — including the case a direction cannot express at all, because two of them
//!   share one;
//! * a manual `move-region` whose rooms are settled and whose destination is not.
//!
//! Drawing takes only [`AppState`], so every string the player reads is composed against the graph
//! when the prompt opens (see `app::input::open_*`) and carried here. That is what keeps the modal
//! out of the mapper's way.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{ButtonId, DialogButton, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::{AppState, RegionOption, RegionPrompt, RegionPromptAct, RegionPromptKind};

/// Below this the modal cannot show its list and its buttons at once, so it does not draw.
const MIN_W: u16 = 34;
const MIN_H: u16 = 8;

/// Widest the modal grows, however long a layer name is. Longer lines are clipped into it.
const MAX_W: u16 = 72;

/// The bullet each room in the list is marked with (SQ-0858).
const ROOM_BULLET: char = '•';

/// How many rooms the list names before it starts counting instead (SQ-0858).
///
/// The modal grows TALLER for a big region, not wider — one name per row — but not without bound,
/// and truncating is the right end rather than scrolling: this dialog is answered with a radio ring
/// and three buttons, and a scrollable pane would add a fourth thing for `Tab` to mean in a modal
/// whose whole point is a quick separate / not now / never.
///
/// Eight is twice `mapper::suggest::STRUCTURAL_FLOOR`, so every region small enough to have only
/// just triggered a suggestion is listed in full, and a mature underground is summarised instead.
/// The count header always states the true total, so a truncated list never misreports the region.
const ROOMS_SHOWN: usize = 8;

// ── RegionPromptRects ─────────────────────────────────────────────────────────

pub struct RegionPromptRects {
    pub area: Rect,
    pub close: Option<Rect>,
    /// Hit-rect per option row, in `options` order. Clicking one chooses it.
    pub options: Vec<Rect>,
    /// The confirm button — `Separate` for a suggestion, `Move` for a pick.
    pub accept: Option<Rect>,
    /// Suggestion only: put it off, and ask again next crossing.
    pub later: Option<Rect>,
    /// Suggestion only: never ask about this passage again.
    pub never: Option<Rect>,
    /// Suggestion only: never ask about ANY passage on this map again (SQ-1298).
    pub never_story: Option<Rect>,
    /// Pick only: close without moving anything.
    pub cancel: Option<Rect>,
}

// ── Buttons ───────────────────────────────────────────────────────────────────

/// The buttons this prompt shows, left to right. A suggestion offers the design's declining
/// gradient — put it off, silence this passage, silence the whole story — beside the accept; a
/// pick is an ordinary confirm/cancel because declining to pick decides nothing and remembers
/// nothing.
fn buttons_for(prompt: &RegionPrompt) -> &'static [DialogButton] {
    const SUGGEST: &[DialogButton] = &[
        DialogButton { id: ButtonId::Separate, label: "Separate" },
        DialogButton { id: ButtonId::Later, label: "Not now" },
        DialogButton { id: ButtonId::Never, label: "Not this passage" },
        DialogButton { id: ButtonId::NeverStory, label: "Never for this story" },
    ];
    const PICK: &[DialogButton] = &[
        DialogButton { id: ButtonId::MoveRegion, label: "Move" },
        DialogButton { id: ButtonId::Cancel, label: "Cancel" },
    ];
    match prompt.kind {
        RegionPromptKind::Suggest { .. } => SUGGEST,
        _ => PICK,
    }
}

/// The label of one option row, with its radio marker.
fn option_line(opt: &RegionOption, chosen: bool) -> String {
    let mark = if chosen { '•' } else { ' ' };
    let label = match opt {
        RegionOption::Dest { label, .. } | RegionOption::Seam { label, .. } => label,
    };
    format!("({mark}) {label}")
}

/// How wide the button row draws, laid out the same way `draw_dialog` lays it out: each button is
/// its label padded by one reversed space on each side, with one plain space between buttons.
///
/// Counted into the modal's width (SQ-1298) so a fourth, longer button — "Never for this story" —
/// widens the modal to hold it rather than being silently dropped off the left the way
/// `draw_dialog`'s own button layout gives up on a button it has no room left for.
fn button_row_width(buttons: &[DialogButton]) -> u16 {
    let widths: u16 = buttons.iter().map(|b| b.label.chars().count() as u16 + 2).sum();
    let seps = buttons.len().saturating_sub(1) as u16;
    widths + seps
}

/// The room block as it is drawn: a count header, up to [`ROOMS_SHOWN`] bulleted names, and an
/// "…and N more" tail when the region runs past what is shown (SQ-0858).
///
/// `budget` is the rows the block may occupy — what is left of the modal once the body, the
/// options, the buttons and the two blanks have been paid for. The room list is what YIELDS on a
/// short terminal, because the options are the answer and the buttons are how it is given; below
/// three rows it is dropped entirely rather than degenerating into a header and a count.
///
/// The `bool` is "this line is a room name", so the caller can style names apart from the two count
/// lines without either of them having to guess which rows are which.
fn room_lines(rooms: &[String], budget: usize) -> Vec<(String, bool)> {
    if rooms.is_empty() || budget < 3 {
        return Vec::new();
    }
    let n = rooms.len();
    let mut shown = ROOMS_SHOWN.min(n).min(budget - 1);
    if shown < n && shown + 2 > budget {
        shown -= 1; // the "…and N more" tail wants a row of its own
    }
    let s = if n == 1 { "" } else { "s" };
    let mut out = Vec::with_capacity(shown + 2);
    out.push((format!("{n} room{s}:"), false));
    out.extend(rooms.iter().take(shown).map(|r| (format!("  {ROOM_BULLET} {r}"), true)));
    if shown < n {
        out.push((format!("  …and {} more", n - shown), false));
    }
    out
}

// ── draw_region_prompt ────────────────────────────────────────────────────────

/// Draw the region prompt centered over `area`, or `None` when it is closed or will not fit.
pub fn draw_region_prompt(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<RegionPromptRects> {
    let prompt = state.overlays.region_prompt.as_ref()?;

    // Content rows: the body lines, the room list, a blank, one row per option, and a second blank
    // so the answers never sit flush against the buttons (SQ-0858). The button row and the two
    // border rows are on top of that.
    //
    // Everything except the room list is fixed, so the list gets whatever the modal can still
    // afford — which is what keeps a big region from pushing the options off a short terminal.
    let fixed = 2 + 1 + 1 + prompt.body.len() as u16 + 1 + prompt.options.len() as u16;
    let budget = area.height.saturating_sub(2).saturating_sub(fixed);
    let rooms = room_lines(&prompt.rooms, budget as usize);

    let content_rows = prompt.body.len() as u16 + rooms.len() as u16 + 1 + prompt.options.len() as u16 + 1;
    let want_h = (content_rows + 3).max(MIN_H);
    // The room names count towards the width too: leaving them out is what let real room names run
    // off the edge of a modal sized for its body alone (SQ-0858). The button row counts too
    // (SQ-1298): a fourth, longer button would otherwise be sized for and then dropped off the
    // left by `draw_dialog`'s own layout, which gives up on whatever does not fit.
    let widest = prompt
        .body
        .iter()
        .map(|s| s.chars().count())
        .chain(rooms.iter().map(|(s, _)| s.chars().count()))
        .chain(prompt.options.iter().map(|o| option_line(o, true).chars().count()))
        .chain(std::iter::once(button_row_width(buttons_for(prompt)) as usize))
        .max()
        .unwrap_or(0) as u16;
    let want_w = (widest + 4).clamp(MIN_W, MAX_W);

    let modal_w = want_w.min(area.width.saturating_sub(4));
    let modal_h = want_h.min(area.height.saturating_sub(2));
    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);
    let buttons = buttons_for(prompt);
    // The focus ring runs options first, then buttons, so only the tail of it selects a button.
    let button_focus = state.overlays.dialog_focus.checked_sub(prompt.options.len());
    let spec = DialogSpec {
        title: &prompt.title,
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(buttons[0].id),
        focus: button_focus,
        field: None,
    };
    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    let body_style = state.colors.theme.get("dialog.region_prompt.body").style;
    let rooms_style = state.colors.theme.get("dialog.region_prompt.rooms").style;
    let room_style = state.colors.theme.get("dialog.region_prompt.room").style;
    let option_style = state.colors.theme.get("dialog.region_prompt.option").style;
    let chosen_style = state.colors.theme.get("dialog.region_prompt.option:chosen").style;

    let mut y = content.y;
    let put = |buf: &mut Buffer, text: &str, style, y: u16| {
        if y < content.bottom() {
            crate::render::draw_str_clipped(buf, content.x, y, text, style, content);
        }
    };
    for line in &prompt.body {
        put(buf, line, body_style, y);
        y += 1;
    }
    for (line, is_name) in &rooms {
        put(buf, line, if *is_name { room_style } else { rooms_style }, y);
        y += 1;
    }
    y += 1; // a blank row between what is being asked and the answers

    let mut option_rects = Vec::with_capacity(prompt.options.len());
    for (i, opt) in prompt.options.iter().enumerate() {
        let chosen = i == prompt.choice;
        let line = option_line(opt, chosen);
        let style = if chosen { chosen_style } else { option_style };
        put(buf, &line, style, y);
        let w = (line.chars().count() as u16).min(content.width);
        option_rects.push(if y < content.bottom() {
            Rect::new(content.x, y, w, 1)
        } else {
            Rect::new(content.x, content.y, 0, 0)
        });
        y += 1;
    }

    // …and the row `content_rows` reserved past the last option is left as it was drawn: blank,
    // directly above the button row (SQ-0858). It is also the first thing a terminal too short for
    // the whole modal takes back, which is the right order — the buttons and the options matter
    // more than the gap between them.

    let find = |id: ButtonId| rects.buttons.iter().find(|(b, _)| *b == id).map(|(_, r)| *r);
    Some(RegionPromptRects {
        area: rects.area,
        close: rects.close,
        options: option_rects,
        accept: find(ButtonId::Separate).or_else(|| find(ButtonId::MoveRegion)),
        later: find(ButtonId::Later),
        never: find(ButtonId::Never),
        never_story: find(ButtonId::NeverStory),
        cancel: find(ButtonId::Cancel),
    })
}

// ── Keyboard routing ──────────────────────────────────────────────────────────

/// Decode one key press against the focus ring. `None` means the prompt swallowed it.
///
/// Tab / Shift-Tab (and the arrows) are the caller's, as in every other modal here; this is only
/// the part that depends on where the ring is resting. Enter on an OPTION confirms rather than
/// merely selecting, because focusing an option already selects it — there would be nothing left
/// for a second keystroke to do.
pub fn region_prompt_key_focused(
    code: crossterm::event::KeyCode,
    prompt: &RegionPrompt,
    focus: usize,
) -> Option<RegionPromptAct> {
    use crossterm::event::KeyCode;
    let suggest = matches!(prompt.kind, RegionPromptKind::Suggest { .. });
    match code {
        // Esc on a suggestion is not "no": it is "not now", which re-arms the seam. On a pick
        // there is nothing to remember, so it just closes.
        KeyCode::Esc => Some(if suggest { RegionPromptAct::Defer } else { RegionPromptAct::Dismiss }),
        KeyCode::Enter => match focus.checked_sub(prompt.options.len()) {
            None | Some(0) => Some(RegionPromptAct::Accept),
            Some(1) => Some(if suggest { RegionPromptAct::Defer } else { RegionPromptAct::Dismiss }),
            Some(2) if suggest => Some(RegionPromptAct::Never),
            Some(3) if suggest => Some(RegionPromptAct::NeverForStory),
            _ => None,
        },
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use crate::state::RegionPromptKind;
    use mapper::layer::{MoveTarget, Region, MAIN_LAYER};
    use mapper::suggest::{SeamKey, Trigger};
    use std::collections::BTreeSet;

    /// Draw `prompt` into a `w`x`h` terminal and hand back the rects and the screen, row by row.
    fn render(prompt: RegionPrompt, w: u16, h: u16) -> (Option<RegionPromptRects>, Vec<String>) {
        render_themed(prompt, w, h, AppState::default())
    }

    /// …and the same with a state the caller has already themed.
    fn render_themed(
        prompt: RegionPrompt,
        w: u16,
        h: u16,
        mut state: AppState,
    ) -> (Option<RegionPromptRects>, Vec<String>) {
        use ratatui::{backend::TestBackend, Terminal};
        state.overlays.region_prompt = Some(prompt);
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_region_prompt(&state, f.area(), f.buffer_mut()); }).unwrap();
        let buf = terminal.backend().buffer();
        let rows = (0..h)
            .map(|y| (0..w).map(|x| buf.cell((x, y)).unwrap().symbol().to_string()).collect())
            .collect();
        (rects, rows)
    }

    /// The one row of a render that holds `needle`, or a panic naming what was on screen instead.
    fn row_with(rows: &[String], needle: &str) -> usize {
        let hits: Vec<usize> =
            rows.iter().enumerate().filter(|(_, r)| r.contains(needle)).map(|(i, _)| i).collect();
        assert_eq!(hits.len(), 1, "{needle:?} should be on exactly one row, screen was:\n{}", rows.join("\n"));
        hits[0]
    }

    fn suggestion_prompt() -> RegionPrompt {
        RegionPrompt {
            kind: RegionPromptKind::Suggest {
                trigger: Trigger::Structural,
                seam: SeamKey { from: 1, dir: mapper::direction::Direction::Up },
                region: Region { anchor: 3, rooms: BTreeSet::from([3, 4, 5, 6]) },
            },
            title: "Give these rooms a layer?".to_string(),
            body: vec!["Four rooms sit behind a portal.".to_string()],
            rooms: ["Cellar", "Wine Cellar", "Vault", "Crypt"].map(String::from).to_vec(),
            options: vec![
                RegionOption::Dest { label: "a new layer".to_string(), target: MoveTarget::New },
                RegionOption::Dest {
                    label: "Main".to_string(),
                    target: MoveTarget::Existing(MAIN_LAYER),
                },
            ],
            choice: 0,
        }
    }

    fn pick_prompt() -> RegionPrompt {
        RegionPrompt {
            kind: RegionPromptKind::PickDest {
                region: Region { anchor: 3, rooms: BTreeSet::from([3, 4]) },
                cut: None,
            },
            title: "Where do these rooms go?".to_string(),
            body: vec!["Two rooms could go to either layer.".to_string()],
            rooms: Vec::new(),
            options: vec![
                RegionOption::Dest { label: "a new layer".to_string(), target: MoveTarget::New }
            ],
            choice: 0,
        }
    }

    /// The modal draws its title, its body, its room list and one marked radio row per option,
    /// and hands back a hit-rect for each of them plus the four outcome buttons (SQ-1298: the
    /// declining gradient is now Not now / Not this passage / Never for this story, beside Separate).
    #[test]
    fn suggestion_renders_options_and_four_outcomes() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = AppState::default();
        state.overlays.region_prompt = Some(suggestion_prompt());
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_region_prompt(&state, f.area(), f.buffer_mut()); }).unwrap();
        let r = rects.expect("an open prompt draws");
        assert_eq!(r.options.len(), 2, "one hit-rect per option");
        assert!(
            r.accept.is_some() && r.later.is_some() && r.never.is_some() && r.never_story.is_some()
        );
        assert!(r.cancel.is_none(), "a suggestion has no Cancel — every outcome is a decision");
        let all: String =
            terminal.backend().buffer().content().iter().flat_map(|c| c.symbol().chars()).collect();
        assert!(all.contains("Give these rooms a layer?"), "title");
        assert!(all.contains("Four rooms sit behind a portal."), "body");
        assert!(all.contains("4 rooms:"), "how many would move");
        for room in ["Cellar", "Wine Cellar", "Vault", "Crypt"] {
            assert!(all.contains(&format!("• {room}")), "{room} is a bullet in the list");
        }
        assert!(all.contains("(•) a new layer"), "the chosen option is marked");
        assert!(all.contains("( ) Main"), "the unchosen one is not");
        assert!(all.contains("Separate"));
        assert!(all.contains("Not now"));
        assert!(all.contains("Not this passage"), "the renamed per-seam never");
        assert!(all.contains("Never for this story"), "the new story-wide never");
    }

    /// A pick is an ordinary confirm/cancel: Move and Cancel, no memory buttons.
    #[test]
    fn a_pick_offers_move_and_cancel() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = AppState::default();
        state.overlays.region_prompt = Some(pick_prompt());
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_region_prompt(&state, f.area(), f.buffer_mut()); }).unwrap();
        let r = rects.expect("an open prompt draws");
        assert!(r.accept.is_some() && r.cancel.is_some());
        assert!(
            r.later.is_none() && r.never.is_none() && r.never_story.is_none(),
            "nothing is remembered about a pick"
        );
    }

    // ── SQ-0858: the list the player complained was cut off ───────────────────

    /// The rooms are a BULLETED LIST, one per row, under a count header — not a comma-joined line
    /// truncated at four names. The reported symptom was that line running off the modal's edge.
    #[test]
    fn the_room_list_is_bulleted_one_room_to_a_row() {
        let (r, rows) = render(suggestion_prompt(), 80, 24);
        r.expect("an open prompt draws");
        let header = row_with(&rows, "4 rooms:");
        let names = ["Cellar", "Wine Cellar", "Vault", "Crypt"];
        let mut at: Vec<usize> = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let y = row_with(&rows, &format!("• {name}"));
            assert_eq!(y, header + 1 + i, "{name} is the {i}th bullet under the header");
            at.push(y);
        }
        at.dedup();
        assert_eq!(at.len(), names.len(), "each room gets a row of its own");
        assert!(
            !rows.iter().any(|r| r.contains("Cellar, Wine Cellar")),
            "and nothing is comma-joined into one line any more"
        );
    }

    /// The room names decide the modal's WIDTH too. The old sizing looked only at the body and the
    /// options, so a long room name simply ran off the edge — which is what was reported.
    #[test]
    fn a_long_room_name_widens_the_modal_rather_than_being_cut_off() {
        let mut p = suggestion_prompt();
        p.rooms = vec!["At West End of Long Hall of Mists".to_string()];
        let (r, rows) = render(p, 80, 24);
        let r = r.expect("an open prompt draws");
        row_with(&rows, "• At West End of Long Hall of Mists");
        assert!(
            r.area.width >= "  • At West End of Long Hall of Mists".chars().count() as u16 + 4,
            "the modal grew to hold the name: {}",
            r.area.width
        );
    }

    /// Past a sensible height the list stops naming and starts counting: eight rooms, then how many
    /// were left out. It never grows without bound and it never misreports the region — the header
    /// still says 20.
    #[test]
    fn a_big_region_names_eight_rooms_and_counts_the_rest() {
        let mut p = suggestion_prompt();
        p.rooms = (1..=20).map(|i| format!("Room {i}")).collect();
        let (r, rows) = render(p, 80, 40);
        let r = r.expect("an open prompt draws");
        row_with(&rows, "20 rooms:");
        for i in 1..=ROOMS_SHOWN {
            row_with(&rows, &format!("• Room {i} "));
        }
        assert!(
            !rows.iter().any(|row| row.contains(&format!("• Room {} ", ROOMS_SHOWN + 1))),
            "the ninth room is counted, not named"
        );
        let more = row_with(&rows, "…and 12 more");
        assert_eq!(more, row_with(&rows, &format!("• Room {ROOMS_SHOWN} ")) + 1, "and it is the tail");
        assert!(
            r.accept.is_some() && r.later.is_some() && r.never.is_some() && r.never_story.is_some(),
            "all four still fit"
        );
    }

    /// On a terminal too short for all of it, the ROOM LIST is what gives up rows — the options are
    /// the answer and the buttons are how it is given, so neither may be squeezed out first.
    #[test]
    fn a_short_terminal_takes_rows_from_the_room_list_not_the_options() {
        let mut p = suggestion_prompt();
        p.rooms = (1..=20).map(|i| format!("Room {i}")).collect();
        let (r, rows) = render(p, 80, 16);
        let r = r.expect("it still draws");
        assert_eq!(r.options.len(), 2, "both destinations are still offered");
        for opt in &r.options {
            assert!(opt.height > 0, "…and neither was clipped away: {:?}", r.options);
        }
        assert!(
            r.accept.is_some() && r.later.is_some() && r.never.is_some() && r.never_story.is_some()
        );
        row_with(&rows, "20 rooms:");
        assert!(
            rows.iter().any(|row| row.contains("more")),
            "the list said how much it left out:\n{}",
            rows.join("\n")
        );
        assert!(r.area.height <= 16, "and the modal stayed inside the terminal: {}", r.area.height);
    }

    /// A blank row between the answers and the buttons, which is what the report asked for: the
    /// last option sits two rows above the button row, not one.
    #[test]
    fn a_blank_row_separates_the_options_from_the_buttons() {
        for (what, p) in [("a suggestion", suggestion_prompt()), ("a pick", pick_prompt())] {
            let (r, rows) = render(p, 80, 24);
            let r = r.expect("an open prompt draws");
            let last = r.options.last().expect("every prompt has options").y;
            let buttons = r.accept.expect("every prompt has a confirm button").y;
            // At LEAST one clear row. A prompt short enough to be padded up to `MIN_H` gets more,
            // and that is slack rather than a second rule.
            assert!(
                buttons >= last + 2,
                "{what}: the buttons sit flush against the options ({last} then {buttons})"
            );
            let inside: String = rows[(last + 1) as usize]
                .chars()
                .skip(r.area.x as usize + 1)
                .take(r.area.width as usize - 2)
                .collect();
            assert!(
                inside.trim().is_empty(),
                "…and that row is genuinely blank: {inside:?}"
            );
        }
    }

    /// Esc means "not now" on a suggestion — the answer that re-arms — and a plain close on a
    /// pick. Enter confirms from an option row and from the first button; the second, third and
    /// fourth buttons carry the other three outcomes (SQ-1298).
    #[test]
    fn keys_map_to_the_four_outcomes() {
        use crossterm::event::KeyCode;
        let s = suggestion_prompt();
        assert_eq!(region_prompt_key_focused(KeyCode::Esc, &s, 0), Some(RegionPromptAct::Defer));
        // Focus 0 and 1 are the two options; 2/3/4/5 are Separate / Not now / Not this passage /
        // Never for this story.
        assert_eq!(region_prompt_key_focused(KeyCode::Enter, &s, 1), Some(RegionPromptAct::Accept));
        assert_eq!(region_prompt_key_focused(KeyCode::Enter, &s, 2), Some(RegionPromptAct::Accept));
        assert_eq!(region_prompt_key_focused(KeyCode::Enter, &s, 3), Some(RegionPromptAct::Defer));
        assert_eq!(region_prompt_key_focused(KeyCode::Enter, &s, 4), Some(RegionPromptAct::Never));
        assert_eq!(
            region_prompt_key_focused(KeyCode::Enter, &s, 5),
            Some(RegionPromptAct::NeverForStory)
        );
        // Space stays widget-reserved: it decides nothing here.
        assert_eq!(region_prompt_key_focused(KeyCode::Char(' '), &s, 0), None);

        let p = pick_prompt();
        assert_eq!(region_prompt_key_focused(KeyCode::Esc, &p, 0), Some(RegionPromptAct::Dismiss));
        // Focus 0 is the lone option; 1 = Move, 2 = Cancel.
        assert_eq!(region_prompt_key_focused(KeyCode::Enter, &p, 1), Some(RegionPromptAct::Accept));
        assert_eq!(region_prompt_key_focused(KeyCode::Enter, &p, 2), Some(RegionPromptAct::Dismiss));
    }

    /// Both option styles are themed, not hard-coded: overriding the two selectors repaints the
    /// chosen and unchosen rows independently.
    #[test]
    fn option_rows_take_their_style_from_style_toml() {
        use ratatui::style::Color;
        use ratatui::{backend::TestBackend, Terminal};
        for honor in [true, false] {
            let mut state = AppState::default();
            state.config.honor_game_colours = honor;
            let parsed = crate::theme::toml_schema::parse(
                "[dialog]\n\
                 \"region_prompt.option\" = { fg = \"blue\" }\n\
                 \"region_prompt.option:chosen\" = { fg = \"green\" }\n",
            )
            .unwrap();
            state.colors.theme = crate::theme::resolve::resolve_theme(
                &crate::colors::GhosttyScheme::default(),
                &parsed,
            );
            state.overlays.region_prompt = Some(suggestion_prompt());
            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            let mut rects = None;
            terminal.draw(|f| { rects = draw_region_prompt(&state, f.area(), f.buffer_mut()); }).unwrap();
            let r = rects.expect("an open prompt draws");
            let buf = terminal.backend().buffer();
            let fg_at = |rect: Rect| buf.cell((rect.x + 1, rect.y)).unwrap().style().fg;
            assert_eq!(
                fg_at(r.options[0]),
                Some(Color::Green),
                "the chosen row uses region_prompt.option:chosen (honor_game_colours={honor})"
            );
            assert_eq!(
                fg_at(r.options[1]),
                Some(Color::Blue),
                "an unchosen row uses region_prompt.option (honor_game_colours={honor})"
            );
        }
    }

    /// Where `needle` starts on screen. The modal draws single-width symbols, so a byte offset into
    /// a row is a column count away from a column.
    fn locate(buf: &Buffer, needle: &str) -> (u16, u16) {
        let a = *buf.area();
        for y in a.y..a.bottom() {
            let row: String =
                (a.x..a.right()).map(|x| buf.cell((x, y)).unwrap().symbol().to_string()).collect();
            if let Some(i) = row.find(needle) {
                return (a.x + row[..i].chars().count() as u16, y);
            }
        }
        panic!("{needle:?} is nowhere on screen");
    }

    /// The room list is themed like everything else here (SQ-0858): the named rooms carry their own
    /// selector, and the two count lines around them keep the old one — so the names can be lit
    /// apart from the arithmetic, and neither is hard-coded.
    #[test]
    fn the_room_list_takes_its_style_from_style_toml() {
        use ratatui::style::Color;
        use ratatui::{backend::TestBackend, Terminal};
        // First with both selectors set, then with only the old one — which must still reach the
        // bullets, because `room` inherits `rooms` and an existing theme knows nothing of it.
        for (toml, bullet) in [
            ("\"region_prompt.rooms\" = { fg = \"blue\" }\n\"region_prompt.room\" = { fg = \"green\" }\n",
             Color::Green),
            ("\"region_prompt.rooms\" = { fg = \"blue\" }\n", Color::Blue),
        ] {
            for honor in [true, false] {
                let mut state = AppState::default();
                state.config.honor_game_colours = honor;
                let parsed =
                    crate::theme::toml_schema::parse(&format!("[dialog]\n{toml}")).unwrap();
                state.colors.theme = crate::theme::resolve::resolve_theme(
                    &crate::colors::GhosttyScheme::default(),
                    &parsed,
                );
                let mut p = suggestion_prompt();
                p.rooms = (1..=20).map(|i| format!("Room {i}")).collect();
                state.overlays.region_prompt = Some(p);
                let mut terminal = Terminal::new(TestBackend::new(80, 40)).unwrap();
                terminal.draw(|f| { draw_region_prompt(&state, f.area(), f.buffer_mut()); }).unwrap();
                let buf = terminal.backend().buffer();
                let fg = |needle: &str| buf.cell(locate(buf, needle)).unwrap().style().fg;
                assert_eq!(
                    fg("• Room 1"),
                    Some(bullet),
                    "a named room uses region_prompt.room (honor_game_colours={honor}, {toml:?})"
                );
                assert_eq!(fg("20 rooms:"), Some(Color::Blue), "the count header");
                assert_eq!(fg("…and 12 more"), Some(Color::Blue), "and the tail that counts too");
            }
        }
    }
}
