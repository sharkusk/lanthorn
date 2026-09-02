//! The "keep this download in your library?" prompt (SQ-1086).
//!
//! Raised once, immediately after a story fetched from a URL has booted. A fetch
//! that nobody keeps plays out of the temp directory it landed in and is
//! forgotten; keeping it copies the file into the library directory the story
//! picker reads, so the next launch finds it without a second fetch and the IFDB
//! metadata/cover sweep has something to attach to.
//!
//! Thin, like [`crate::render::history_prompt`]: the chrome, the focus ring, the
//! button hit-rects and the keyboard ladder are all
//! [`crate::render::dialog::draw_dialog`]'s and the `Overlay` trait's. What is
//! local to this file is the wording, and the fact that the button row has two
//! shapes.
//!
//! ## Why three buttons when the name collides
//!
//! Because "the library already holds a `curses.z5`" has two right answers and
//! neither may happen in silence. Replacing loses a file the player put there;
//! renaming to `curses-2.z5` behind their back leaves them hunting for a story
//! under a name they never chose. So the collision case says so in the body and
//! offers **Replace** and **Keep both** side by side, with focus starting on the
//! non-destructive one. Without a collision the prompt is the ordinary two-button
//! yes/no.
//!
//! ## The second shape: a downloaded zip of disk images (SQ-1096)
//!
//! Archive sites ship C64, Amiga and Apple II floppies inside zips as a matter
//! of course, and lanthorn's zip loader is a volume of raw stories — it does not
//! mount media. So a download that holds five `.dsk` files and no story reaches
//! the SAME prompt, before anything boots, worded for what it is: **Unpack into
//! library** copies the images out (and only the images), **Cancel** ends the
//! launch saying why.
//!
//! The order is the point. For a story the prompt comes AFTER the boot, because
//! that is when the player knows whether they want it. For an archive the load
//! fails before any prompt could be reached, so the offer has to come first —
//! there is nothing to play until it is answered.
//!
//! ## Styling
//!
//! Existing selectors only — no new theme rows, in both shapes. The chrome is
//! `DialogStyle`'s (`dialog.border` / `dialog.title` / `dialog.button` /
//! `dialog.shadow`), the body is `dialog.background`, the URL and destination
//! lines take the dim secondary-line style `dialog.hint_suggestion`, and the
//! collision warning — plus the archive shape's "lanthorn does not run these
//! from inside a zip" line — takes `dialog.launch_caveat`: `alert` without
//! `dim`, which is exactly the register of a line that exists to stop the player
//! being surprised by the outcome.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{ButtonId, DialogButton, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;
use crate::story_url::KeepMode;

const MIN_W: u16 = 44;
const MIN_H: u16 = 10;
const DIALOG_W: u16 = 68;
const DIALOG_H: u16 = 13;
/// The archive shape says three things the story shape does not — what the zip
/// holds, that lanthorn will not run it from in there, and what unpacking does —
/// so it is three rows taller and wants three more rows before it gives up.
const ARCHIVE_H: u16 = 16;
const ARCHIVE_MIN_H: u16 = 13;

pub struct FetchKeepRects {
    pub area: Rect,
    pub close: Option<Rect>,
    /// "Keep in library" ("Unpack into library" for an archive), or "Replace"
    /// when the name collides.
    pub keep: Option<Rect>,
    /// "Keep both" — present only when the name collides.
    pub keep_both: Option<Rect>,
    /// "Just play it".
    pub decline: Option<Rect>,
}

/// How many focus positions the prompt has right now — 3 when the library
/// already holds this name, else 2. The `Overlay` impl's `cycle_focus` and this
/// renderer must agree, so both ask here.
pub fn button_count(state: &AppState) -> usize {
    match &state.overlays.fetch_keep {
        Some(p) if p.collision => 3,
        _ => 2,
    }
}

/// Draw the prompt centred over `area`, or `None` when it is closed or the pane
/// is too small to hold it.
pub fn draw_fetch_keep_dialog(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<FetchKeepRects> {
    let prompt = state.overlays.fetch_keep.as_ref()?;
    // SQ-1096: an archive prompt is the same dialog with a different subject —
    // several files that have not been written yet, rather than one that is
    // already playing.
    let archive = !prompt.disk_images.is_empty();

    let (want_h, min_h) =
        if archive { (ARCHIVE_H, ARCHIVE_MIN_H) } else { (DIALOG_H, MIN_H) };
    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = want_h.min(area.height.saturating_sub(2));
    if modal_w < MIN_W || modal_h < min_h {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);
    // "Just play it" is the honest answer only when something IS playing.
    // Declining an archive plays nothing — the launch simply ends — so the
    // archive shape says so.
    let two = [
        DialogButton {
            id: ButtonId::Ok,
            label: if archive { "Unpack into library" } else { "Keep in library" },
        },
        DialogButton { id: ButtonId::Cancel, label: if archive { "Cancel" } else { "Just play it" } },
    ];
    // "Keep both" comes FIRST, so focus index 0 is the harmless keep in BOTH
    // shapes. That is not cosmetic: `dialog_focus` is shared across the whole
    // common-dialog ladder, so this prompt can inherit a 0 left behind by
    // whichever dialog was answered before it — and a 0 that means "replace the
    // file you already had" would be a destructive default arrived at by
    // accident.
    let three = [
        DialogButton { id: ButtonId::KeepBoth, label: "Keep both" },
        DialogButton { id: ButtonId::Ok, label: "Replace" },
        DialogButton { id: ButtonId::Cancel, label: if archive { "Cancel" } else { "Just play it" } },
    ];
    let buttons: &[DialogButton] = if prompt.collision { &three } else { &two };
    // The underlined default is the harmless answer in both shapes: keep it when
    // that costs nothing, keep BOTH when replacing would cost a file.
    let default = if prompt.collision { ButtonId::KeepBoth } else { ButtonId::Ok };
    debug_assert_eq!(buttons.len(), button_count(state), "focus ring and button row must agree");

    let spec = DialogSpec {
        title: if archive { "Unpack this download?" } else { "Keep this story?" },
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(default),
        focus: Some(state.overlays.dialog_focus),
        field: None,
    };
    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    let body_style = state.colors.theme.get("dialog.background").style;
    let detail_style = state.colors.theme.get("dialog.hint_suggestion").style;
    let caveat_style = state.colors.theme.get("dialog.launch_caveat").style;

    // Name the file, then the address it came from (a player who typed three
    // URLs has to know which one this is), then where keeping it would put it.
    let mut lines: Vec<(String, ratatui::style::Style)> = vec![
        (prompt.fetched.filename(), body_style),
        (format!("from {}", prompt.fetched.url), detail_style),
        (String::new(), body_style),
    ];
    if archive {
        // The count is what will be EXTRACTED, not the archive's entry count —
        // a zip of five floppies and forty text files is five disk images here,
        // because five files are what land in the library.
        let n = prompt.disk_images.len();
        lines.push((
            format!(
                "This archive holds {n} disk image{}, and lanthorn does not",
                if n == 1 { "" } else { "s" }
            ),
            caveat_style,
        ));
        lines.push(("run disk images from inside a zip.".to_string(), caveat_style));
        lines.push((String::new(), body_style));
        lines.push((
            format!(
                "Unpacking copies {} into your library, and nothing else",
                if n == 1 { "it" } else { "them" }
            ),
            body_style,
        ));
        lines.push(("out of the archive:".to_string(), body_style));
    } else {
        lines.push(("Keeping it copies the file into your library:".to_string(), body_style));
    }
    lines.push((prompt.library_dir.display().to_string(), detail_style));
    if prompt.collision {
        lines.push((String::new(), body_style));
        lines.push((
            if archive {
                "Some of those names are already in your library.".to_string()
            } else {
                "A file of that name is already in your library.".to_string()
            },
            caveat_style,
        ));
    }

    for (i, (line, style)) in lines.iter().enumerate() {
        let y = content.y + i as u16;
        if y < content.bottom() && !line.is_empty() {
            crate::render::draw_str_clipped(buf, content.x, y, line, *style, content);
        }
    }

    let find = |want: ButtonId| rects.buttons.iter().find(|(id, _)| *id == want).map(|(_, r)| *r);
    Some(FetchKeepRects {
        area: rects.area,
        close: rects.close,
        keep: find(ButtonId::Ok),
        keep_both: find(ButtonId::KeepBoth),
        decline: find(ButtonId::Cancel),
    })
}

// ── Keyboard routing ─────────────────────────────────────────────────────────

/// What a key press on the keep prompt means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchKeepAction {
    /// Copy it into the library, resolving a name collision this way.
    Keep(KeepMode),
    /// Play it from where it landed and forget it.
    Decline,
    /// Nothing (the caller has already handled focus movement).
    None,
}

/// Map a key to a [`FetchKeepAction`] given the focused button index.
///
/// Focus indices follow the drawn button row: without a collision `0 = Keep,
/// 1 = Just play it`; with one `0 = Keep both, 1 = Replace, 2 = Just play it`.
/// Index 0 is the harmless keep either way — see the button row for why.
/// Tab/Shift-Tab are the caller's (they mutate `dialog_focus`), per the standing
/// shared-chrome convention; Esc always declines, and Space is left alone
/// because it is widget-reserved.
pub fn fetch_keep_key_focused(
    code: crossterm::event::KeyCode,
    focus: usize,
    collision: bool,
) -> FetchKeepAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc => FetchKeepAction::Decline,
        KeyCode::Enter => match (collision, focus) {
            (false, 0) => FetchKeepAction::Keep(KeepMode::KeepBoth),
            (true, 0) => FetchKeepAction::Keep(KeepMode::KeepBoth),
            (true, 1) => FetchKeepAction::Keep(KeepMode::Replace),
            _ => FetchKeepAction::Decline,
        },
        _ => FetchKeepAction::None,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use crate::state::FetchKeepPrompt;
    use crate::story_url::FetchedStory;
    use crossterm::event::KeyCode;
    use ratatui::{backend::TestBackend, Terminal};

    fn prompt(collision: bool) -> FetchKeepPrompt {
        FetchKeepPrompt {
            fetched: FetchedStory {
                url: "https://example.org/if/curses.z5".to_string(),
                path: std::path::PathBuf::from("/tmp/lanthorn-fetch/curses.z5"),
            },
            library_dir: std::path::PathBuf::from("/home/p/stories"),
            collision,
            disk_images: Vec::new(),
        }
    }

    /// The SQ-1096 shape: a downloaded zip of floppies, nothing booted.
    fn archive_prompt(collision: bool, n: usize) -> FetchKeepPrompt {
        FetchKeepPrompt {
            fetched: FetchedStory {
                url: "https://example.org/if/journey.zip".to_string(),
                path: std::path::PathBuf::from("/tmp/lanthorn-fetch/journey.zip"),
            },
            library_dir: std::path::PathBuf::from("/home/p/stories"),
            collision,
            disk_images: (1..=n).map(|i| format!("journey_s{i}.dsk")).collect(),
        }
    }

    fn render(state: &AppState) -> (Option<FetchKeepRects>, String) {
        let backend = TestBackend::new(90, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| rects = draw_fetch_keep_dialog(state, f.area(), f.buffer_mut())).unwrap();
        let all: String =
            terminal.backend().buffer().content().iter().flat_map(|c| c.symbol().chars()).collect();
        (rects, all)
    }

    #[test]
    fn closed_by_default_and_drawn_when_raised() {
        let mut state = AppState::default();
        assert!(render(&state).0.is_none(), "nothing drawn when no fetch is pending");
        state.overlays.fetch_keep = Some(prompt(false));
        let (rects, all) = render(&state);
        let r = rects.expect("drawn once the prompt is raised");
        assert!(r.keep.is_some() && r.decline.is_some());
        assert!(r.keep_both.is_none(), "no third answer when nothing collides");
        assert!(all.contains("Keep this story?"), "title present");
        assert!(all.contains("curses.z5"), "the file is named");
        assert!(all.contains("example.org"), "the address it came from is shown");
        assert!(all.contains("stories"), "and where keeping it would put it");
    }

    /// A collision must be visible and must offer both answers — never a silent
    /// clobber and never a silent rename.
    #[test]
    fn a_name_collision_says_so_and_offers_a_third_answer() {
        let mut state = AppState::default();
        state.overlays.fetch_keep = Some(prompt(true));
        let (rects, all) = render(&state);
        let r = rects.expect("drawn");
        assert!(r.keep.is_some() && r.keep_both.is_some() && r.decline.is_some());
        assert!(all.contains("already in your library"), "the collision is stated: {all}");
        assert!(all.contains("Replace") && all.contains("Keep both"));
    }

    #[test]
    fn button_count_follows_the_collision() {
        let mut state = AppState::default();
        assert_eq!(button_count(&state), 2, "closed prompts still answer sanely");
        state.overlays.fetch_keep = Some(prompt(false));
        assert_eq!(button_count(&state), 2);
        state.overlays.fetch_keep = Some(prompt(true));
        assert_eq!(button_count(&state), 3);
    }

    #[test]
    fn enter_follows_focus_and_esc_always_declines() {
        // Two-button shape: 0 = keep, 1 = decline.
        assert_eq!(
            fetch_keep_key_focused(KeyCode::Enter, 0, false),
            FetchKeepAction::Keep(KeepMode::KeepBoth)
        );
        assert_eq!(fetch_keep_key_focused(KeyCode::Enter, 1, false), FetchKeepAction::Decline);
        // Three-button shape: 0 = keep both, 1 = replace, 2 = decline. Index 0 is
        // the harmless answer in both shapes, because `dialog_focus` is shared
        // across the ladder and an inherited 0 must never mean "replace".
        assert_eq!(
            fetch_keep_key_focused(KeyCode::Enter, 0, true),
            FetchKeepAction::Keep(KeepMode::KeepBoth)
        );
        assert_eq!(
            fetch_keep_key_focused(KeyCode::Enter, 1, true),
            FetchKeepAction::Keep(KeepMode::Replace)
        );
        assert_eq!(fetch_keep_key_focused(KeyCode::Enter, 2, true), FetchKeepAction::Decline);
        // Esc cancels from anywhere, in both shapes.
        assert_eq!(fetch_keep_key_focused(KeyCode::Esc, 0, true), FetchKeepAction::Decline);
        assert_eq!(fetch_keep_key_focused(KeyCode::Esc, 1, false), FetchKeepAction::Decline);
        // Space is widget-reserved, so it must not activate anything here.
        assert_eq!(fetch_keep_key_focused(KeyCode::Char(' '), 0, false), FetchKeepAction::None);
        assert_eq!(fetch_keep_key_focused(KeyCode::Char('x'), 0, false), FetchKeepAction::None);
    }

    /// SQ-1096. The archive shape has to say three things the story shape never
    /// does — what the download holds, that lanthorn will not run it from in
    /// there, and that unpacking takes the images and NOTHING else — because a
    /// player who answers "no" gets no game at all and must know why.
    #[test]
    fn the_archive_shape_says_what_it_holds_and_what_lanthorn_will_not_do() {
        let mut state = AppState::default();
        state.overlays.fetch_keep = Some(archive_prompt(false, 5));
        let (rects, all) = render(&state);
        let r = rects.expect("drawn");
        assert!(r.keep.is_some() && r.decline.is_some());
        assert!(r.keep_both.is_none(), "no third answer when nothing collides");
        assert!(all.contains("Unpack this download?"), "titled for what it is: {all}");
        assert!(all.contains("journey.zip"), "the archive is named");
        assert!(all.contains("holds 5 disk images"), "the count is stated: {all}");
        assert!(all.contains("does not"), "and that lanthorn will not run them from in there");
        assert!(all.contains("nothing else"), "and that only the images come out: {all}");
        assert!(all.contains("Unpack into library"), "the affirmative says what it does");
        // "Just play it" is a lie here: declining plays nothing.
        assert!(!all.contains("Just play it"), "nothing would be played: {all}");
    }

    /// One image is one image — an archive with a single floppy must not be told
    /// it holds "1 disk images".
    #[test]
    fn a_single_image_is_worded_in_the_singular() {
        let mut state = AppState::default();
        state.overlays.fetch_keep = Some(archive_prompt(false, 1));
        let (_r, all) = render(&state);
        assert!(all.contains("holds 1 disk image,"), "singular: {all}");
        assert!(all.contains("copies it into"), "and so is the verb: {all}");
    }

    /// A collision on a SET is still the three-answer shape, worded for several
    /// files rather than one.
    #[test]
    fn an_archive_collision_offers_the_same_three_answers() {
        let mut state = AppState::default();
        state.overlays.fetch_keep = Some(archive_prompt(true, 3));
        let (rects, all) = render(&state);
        let r = rects.expect("drawn");
        assert!(r.keep.is_some() && r.keep_both.is_some() && r.decline.is_some());
        assert!(all.contains("Some of those names are already"), "plural collision: {all}");
        assert!(all.contains("Replace") && all.contains("Keep both"));
        assert_eq!(button_count(&state), 3);
    }

    /// The story shape must be untouched by all of the above — no "disk image"
    /// wording may leak into the SQ-1086 prompt.
    #[test]
    fn the_story_shape_is_unchanged() {
        let mut state = AppState::default();
        state.overlays.fetch_keep = Some(prompt(false));
        let (_r, all) = render(&state);
        assert!(all.contains("Keep this story?") && all.contains("Keep in library"));
        assert!(all.contains("Just play it"), "a story IS playing either way");
        assert!(!all.contains("disk image"), "no archive wording in the story shape: {all}");
    }

    /// The prompt must vanish rather than paint over a pane too small to hold it.
    #[test]
    fn a_tiny_pane_draws_nothing() {
        let mut state = AppState::default();
        state.overlays.fetch_keep = Some(prompt(false));
        let backend = TestBackend::new(30, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| rects = draw_fetch_keep_dialog(&state, f.area(), f.buffer_mut())).unwrap();
        assert!(rects.is_none());
    }
}
