//! Modal/overlay dispatch: the z-ordered draw ladder over the graphics-free
//! dialog area, plus the [`Overlay`] trait that unifies the common-dialog
//! modals (aux / reset / save-name / text-entry / confirm-delete / quit /
//! launch) for both drawing and run-loop input decoding (SQ-0307). The richer
//! non-dialog modals (hotkey / saves / …) keep their bespoke draw
//! branches. Each branch/impl calls a render or key fn that already lives in
//! `app::render::*`; this module owns only the dispatch + the returned hit-rects.

use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};

use app::engine::ScreenModel;
use app::input::cycle_focus;
use app::render::aux_dialog::{aux_dialog_key_focused, draw_aux_dialog, AuxDialogAction, AuxDialogRects};
use app::render::config_screen::draw_config_screen;
use app::render::confirm_delete_dialog::{
    confirm_delete_key_focused, draw_confirm_delete_dialog, ConfirmDeleteAction, ConfirmDeleteDialogRects,
};
use app::render::confirm_overwrite_dialog::{
    confirm_overwrite_key_focused, draw_confirm_overwrite_dialog, ConfirmOverwriteAction, ConfirmOverwriteDialogRects,
};
use app::render::dialog::DialogRects;
use app::render::fetch_keep_dialog::{
    button_count as fetch_keep_button_count, draw_fetch_keep_dialog, fetch_keep_key_focused,
    FetchKeepAction,
};
use app::render::filebrowser::draw_file_browser;
use app::render::game_over_dialog::{
    draw_game_over_dialog, game_over_dialog_key_focused, GameOverAction, GameOverDialogRects,
};
use app::render::file_picker::draw_file_picker;
use app::render::hints_panel::{draw_hints_panel, HintsPanelRects};
use app::render::history::draw_history;
use app::render::hotkeys::draw_hotkey_dialog;
use app::render::launch_dialog::{
    draw_launch_dialog, launch_dialog_key_focused, LaunchDialogAction, LaunchDialogRects,
};
use app::render::palette::draw_palette;
use app::render::quit_dialog::{draw_quit_dialog, quit_dialog_key_focused, QuitDialogAction, QuitDialogRects};
use app::render::region_prompt::{draw_region_prompt, region_prompt_key_focused, RegionPromptRects};
use app::render::reset_dialog::{draw_reset_dialog, reset_dialog_key_focused, ResetDialogAction, ResetDialogRects};
use app::render::save_name_dialog::{draw_save_name_dialog, save_name_dialog_key, SaveNameAction, SaveNameDialogRects};
use app::render::saves::draw_saves;
use app::render::text_entry_dialog::{
    draw_text_entry_dialog, text_entry_dialog_key, TextEntryAction, TextEntryDialogRects,
};
use app::state::{AppState, OverlayState};

use crate::PaneRects;

/// Per-overlay hit-rects produced by [`draw_all`]. These are the values the
/// dialog-area ladder used to write into `draw_frame` locals that end up in
/// `PaneRects`; `draw_frame` splices them back in unchanged.
pub(crate) struct OverlayRects {
    /// Last-drawn dialog chrome rects. Seeded by the pre-ladder tidy-panel /
    /// room-info / inspector draws in `draw_frame` and conditionally overwritten
    /// by the higher-z-order dialogs in the ladder.
    pub dialog: Option<DialogRects>,
    pub aux_dialog: Option<AuxDialogRects>,
    pub history_prompt: Option<app::render::history_prompt::HistoryPromptRects>,
    pub font_check: Option<app::render::font_check_dialog::FontCheckRects>,
    pub fetch_keep: Option<app::render::fetch_keep_dialog::FetchKeepRects>,
    pub reset_dialog: Option<ResetDialogRects>,
    pub game_over: Option<GameOverDialogRects>,
    pub save_name_dialog: Option<SaveNameDialogRects>,
    pub text_entry: Option<TextEntryDialogRects>,
    pub confirm_delete: Option<ConfirmDeleteDialogRects>,
    pub confirm_overwrite: Option<ConfirmOverwriteDialogRects>,
    pub quit_dialog: Option<QuitDialogRects>,
    pub launch_dialog: Option<LaunchDialogRects>,
    pub region_prompt: Option<RegionPromptRects>,
    pub hints_panel: Option<HintsPanelRects>,
}

/// Draw the z-ordered modal/overlay ladder over the current frame.
///
/// `story_pane` is the story panel's outer rect (border included); the hints
/// panel is laid over it so it visually replaces the story panel.
/// `dialog_seed` carries the `dialog` rect already set by `draw_frame`'s
/// pre-ladder map/story draws (tidy panel, room info, inspector); the ladder's
/// higher-z-order dialogs overwrite it when open. `modal_list_viewport` is the
/// shared list-modal viewport (also written by the verb dock earlier in the
/// frame), threaded by `&mut`. Branch order is z-order — preserved exactly.
pub(crate) fn draw_all(
    state: &AppState,
    screen_model: &ScreenModel,
    story_area: Rect,
    story_pane: Rect,
    full: Rect,
    buf: &mut Buffer,
    dialog_seed: Option<DialogRects>,
    modal_list_viewport: &mut usize,
    palette_hits: &mut Vec<(usize, Rect)>,
) -> OverlayRects {
    let mut out = OverlayRects {
        history_prompt: None,
        font_check: None,
        fetch_keep: None,
        dialog: dialog_seed,
        aux_dialog: None,
        reset_dialog: None,
        game_over: None,
        save_name_dialog: None,
        text_entry: None,
        confirm_delete: None,
        confirm_overwrite: None,
        quit_dialog: None,
        launch_dialog: None,
        region_prompt: None,
        hints_panel: None,
    };

    // Modal dialogs center within the graphics-free text region (story text +
    // map together), never over a Glulx graphics window — the terminal image
    // protocol would otherwise overpaint them (SQ-0203). No graphics → `full`.
    // Clamp to gvm's content bounding box so the graphics-rect walk matches the
    // clamped composite render (the snap-margin has no windows). (SQ-0303)
    let story_bbox = app::render::screen::content_bounds(screen_model, story_area);
    let dialog_area = app::render::screen::dialog_bounds(screen_model, story_bbox, full, state);

    // ── Richer non-dialog modals — z-ordered, not (yet) on the Overlay trait ──

    // ── Hotkey dialog overlay — drawn over everything ─────────────────────
    if state.overlays.hotkey_dialog {
        out.dialog = draw_hotkey_dialog(state, dialog_area, buf);
    }

    // ── Saves-manager overlay — drawn after the hotkey dialog ─────────────
    if state.overlays.saves.is_some() {
        out.dialog = draw_saves(state, dialog_area, buf, modal_list_viewport);
    }

    // ── Replay/rewind overlay ─────────────────────────────────────────────
    if state.overlays.replay.is_some() {
        out.dialog = draw_history(state, dialog_area, buf, modal_list_viewport);
    }

    // ── File-browser overlay — drawn after saves ──────────────────────────
    if state.overlays.file_browser.is_some() {
        out.dialog = draw_file_browser(state, dialog_area, buf, modal_list_viewport);
    }

    // ── VFS file picker overlay (read-mode create_by_prompt) ──────────────
    if state.overlays.file_picker.is_some() {
        out.dialog = draw_file_picker(state, dialog_area, buf, modal_list_viewport);
    }

    // ── Config screen overlay — drawn after other modals ──────────────────
    if state.overlays.config_screen.is_some() {
        out.dialog = draw_config_screen(state, dialog_area, buf, modal_list_viewport);
    }

    // ── Common-dialog modals — drawn through the `Overlay` trait in the exact
    // z-order of the input ladder (aux → reset → save-name → text-entry →
    // confirm-delete → quit → launch). Each impl draws over `dialog_area` and
    // stashes its own hit-rects into `out`. (SQ-0307)
    for ov in COMMON_DIALOGS {
        if ov.is_open(&state.overlays) {
            ov.draw(state, dialog_area, buf, &mut out);
        }
    }

    // ── Command palette popup — drawn over everything (SQ-0419) ────────────
    if state.overlays.palette.is_some() {
        out.dialog = draw_palette(state, dialog_area, buf, modal_list_viewport, palette_hits);
    }

    // ── Hints panel overlay — drawn after the common dialogs ───────────────
    // Laid over the story pane's full rect (not the centered dialog area), so it
    // reads as the story panel temporarily replaced by the hint session and
    // resizes with it.
    if state.overlays.hints.is_some() {
        out.hints_panel = draw_hints_panel(state, story_pane, buf);
    }

    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Overlay trait — unifies the common-dialog modals (SQ-0307)
// ═══════════════════════════════════════════════════════════════════════════
//
// The aux / reset / save-name / text-entry / confirm-delete / quit / launch
// modals used to be seven copy-pasted `if state.<flag> { match &event { … }
// continue }` blocks in the run loop. They now share one decode+apply seam:
// [`topmost_common_dialog`] picks the highest-priority open overlay (the exact
// ladder order below), its [`Overlay`] impl decodes the event — applying pure
// focus / checkbox / text-field changes in place via `&mut AppState` — and
// returns an [`OverlayOutcome`]. Game-affecting side effects (reset, save,
// quit, resume, …) need the loop's `session` / `mapper` / path context, so they
// surface as an [`OverlayAct`] the run loop applies. Draw stays here too, routed
// through [`Overlay::draw`] by `draw_all`.
//
// The `Overlay` impls live in this dispatch module rather than the lib
// `render::*` modules (where the per-dialog key/hit-decode fns they delegate to
// live) because `mouse` hit-tests against [`PaneRects`], a bin-only type the lib
// cannot see. The lib key fns (`*_key_focused`, `*_dialog_key`) are unchanged.

/// A semantic command a common-dialog overlay decoded from an event, for the run
/// loop to apply with its `session` / `mapper` / path context in scope. Pure
/// state changes (focus, checkbox, text field) are applied inside the `Overlay`
/// methods and never surface here.
pub(crate) enum OverlayAct {
    /// Switch `record_turn_history` on and persist it (SQ-1091).
    EnableTurnHistory,
    /// Both font-check stages were answered (SQ-1104, SQ-1245): `.0` = this
    /// terminal draws the Nerd Font icons, so install those presets; `.1` =
    /// the diagonal corner stubs answer, `None` when stage two was skipped
    /// (Esc or the close box) and `diagonal_corners` should be left untouched.
    /// Applying it writes `style.toml` and reloads the live theme, which needs
    /// the run loop's context.
    FontCheck(bool, Option<bool>),
    /// The keep-this-download prompt was answered (SQ-1086): `Some(mode)` copies
    /// the fetched story into the library, `None` plays it where it landed and
    /// forgets it. Applying it needs the run loop's paths, so it surfaces here.
    FetchKeep(Option<app::story_url::KeepMode>),
    AuxArchive,
    AuxGlobal,
    ResetConfirm,
    ResetCancel,
    GameOverPlayAgain,
    GameOverRestore,
    GameOverQuit,
    SaveNameSubmit,
    SaveNameCancel,
    TextEntrySubmit,
    TextEntryCancel,
    ConfirmDelete(bool),
    ConfirmOverwrite(bool),
    QuitSave,
    QuitQuit,
    QuitCancel,
    LaunchResume,
    LaunchNewGame,
    /// The region prompt was answered (SQ-0439). Applying it needs the mapper.
    RegionPrompt(app::state::RegionPromptAct),
}

/// The result of routing one event to the top-most open common-dialog overlay.
pub(crate) enum OverlayOutcome {
    /// Fully handled inside the overlay (focus moved, checkbox toggled, field
    /// edited, click swallowed) — the run loop does nothing further.
    Consumed,
    /// The overlay decoded a game-affecting command for the run loop to apply.
    Act(OverlayAct),
}

/// Identity of a common-dialog overlay — used by the priority-order unit test.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[allow(dead_code)] // introspection hook exercised only by the ladder-order test
pub(crate) enum OverlayKind {
    HistoryPrompt,
    FontCheck,
    FetchKeep,
    Aux,
    Reset,
    GameOver,
    ConfirmOverwrite,
    SaveName,
    TextEntry,
    ConfirmDelete,
    Quit,
    Launch,
    RegionPrompt,
}

pub(crate) trait Overlay {
    /// This overlay's identity (for the priority-order test).
    #[allow(dead_code)] // exercised only by the ladder-order test
    fn kind(&self) -> OverlayKind;
    /// Is this overlay currently open? Priority is the [`COMMON_DIALOGS`] order.
    fn is_open(&self, ov: &OverlayState) -> bool;
    /// Draw over `area` (the graphics-free dialog region), stashing hit-rects in `out`.
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects);
    /// Decode a key press, applying pure state changes in place.
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome;
    /// Decode a mouse event, applying pure state changes in place.
    fn mouse(&self, state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome;
}

/// The common-dialog overlays in strict input-priority / draw z-order (highest
/// priority first): aux ▸ reset ▸ game-over ▸ confirm-overwrite ▸ save-name ▸
/// text-entry ▸ confirm-delete ▸ quit ▸ launch. Confirm-overwrite sits ABOVE
/// save-name deliberately (SQ-0648): the save-as flow leaves the save-name
/// dialog open behind it while it asks, so it must win the priority scan
/// whenever both are `Some` at once, or Cancel would have nothing to fall
/// back into. This is otherwise the exact order of the old run-loop `if`-ladder.
pub(crate) const COMMON_DIALOGS: &[&dyn Overlay] = &[
    // Topmost: the player asked for it by running `open-history`, and it is the
    // only thing on screen when it is up.
    &HistoryPromptOverlay,
    // Also asked for: `/run-font-check`, or the settings screen's row — which
    // stays open behind it, exactly as the path row's text-entry dialog does.
    &FontCheckOverlay,
    &AuxOverlay,
    &ResetOverlay,
    &GameOverOverlay,
    &ConfirmOverwriteOverlay,
    &SaveNameOverlay,
    &TextEntryOverlay,
    &ConfirmDeleteOverlay,
    &QuitOverlay,
    &LaunchOverlay,
    // Below everything the player opened: the keep-this-download prompt is
    // raised by the app at boot, and a resume-or-new-game question about the very
    // session it belongs to has to be settled first (SQ-1086).
    &FetchKeepOverlay,
    // The region prompt sits at the bottom: it is the only modal in this ladder the app raises on
    // its own initiative, so anything the player asked for outranks it (SQ-0439).
    &RegionPromptOverlay,
];

/// The highest-priority open common-dialog overlay, or `None`. Pure over
/// [`OverlayState`]; the run loop routes events to it and `draw_all` draws it.
pub(crate) fn topmost_common_dialog(ov: &OverlayState) -> Option<&'static dyn Overlay> {
    COMMON_DIALOGS.iter().copied().find(|o| o.is_open(ov))
}

/// Left-button-down screen position, or `None` for any other mouse event.
fn left_down(m: &MouseEvent) -> Option<Position> {
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => Some(Position { x: m.column, y: m.row }),
        _ => None,
    }
}

/// True for a Ctrl/Alt/Super-modified printable char (an accelerator, not text).
fn is_ctrl_char(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char(_))
        && key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
}

// ── "Record turn history?" prompt (SQ-1091) ────────────────────────────────
//
// Two buttons and a close, so the whole of it is the shared ladder: `cycle_focus`
// for the focus ring, `dialog_button_key_focused` for Enter/Esc/accelerators, and
// `left_down` + the rects the renderer handed back for the mouse. Nothing here is
// bespoke except which act each button maps to.
struct HistoryPromptOverlay;
impl Overlay for HistoryPromptOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::HistoryPrompt }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.history_prompt }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.history_prompt = app::render::history_prompt::draw_history_prompt(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        match key.code {
            KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, 1);
                OverlayOutcome::Consumed
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, -1);
                OverlayOutcome::Consumed
            }
            KeyCode::Esc => {
                state.overlays.history_prompt = false;
                OverlayOutcome::Consumed
            }
            KeyCode::Enter => {
                state.overlays.history_prompt = false;
                if state.overlays.dialog_focus == 0 {
                    OverlayOutcome::Act(OverlayAct::EnableTurnHistory)
                } else {
                    OverlayOutcome::Consumed
                }
            }
            _ => OverlayOutcome::Consumed,
        }
    }
    fn mouse(&self, state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(hp) = &panes.history_prompt else { return OverlayOutcome::Consumed };
        if hp.enable.is_some_and(|r| r.contains(pt)) {
            state.overlays.history_prompt = false;
            return OverlayOutcome::Act(OverlayAct::EnableTurnHistory);
        }
        if hp.cancel.is_some_and(|r| r.contains(pt)) || hp.close.is_some_and(|r| r.contains(pt)) {
            state.overlays.history_prompt = false;
        }
        OverlayOutcome::Consumed
    }
}

// ── "Which of these two rows does your font draw?" (SQ-1104, SQ-1245) ──────
//
// Two questions, same chrome: stage one asks about the Nerd Font icon glyphs,
// stage two about the diagonal corner stubs, independently of one another in
// both directions. `state.overlays.font_check_icon_answer` carries stage one's
// answer while stage two is up (`None` = still on stage one); both buttons and
// a close, so the whole of each stage is the shared ladder, the same way the
// history prompt is. The pre-game half of this — the FIRST-run ask, before any
// `AppState` exists — is `startup::ask_font_check`, and both drive the same
// `render::font_check_dialog`.
struct FontCheckOverlay;
impl Overlay for FontCheckOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::FontCheck }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.font_check }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.font_check = if state.overlays.font_check_icon_answer.is_some() {
            app::render::font_check_dialog::draw_diagonal_check(state, area, buf)
        } else {
            app::render::font_check_dialog::draw_font_check(state, area, buf)
        };
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        match key.code {
            KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, 1);
                return OverlayOutcome::Consumed;
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, -1);
                return OverlayOutcome::Consumed;
            }
            _ => {}
        }
        match state.overlays.font_check_icon_answer {
            None => {
                use app::render::font_check_dialog::{font_check_key_focused, FontCheckAction};
                match font_check_key_focused(key.code, state.overlays.dialog_focus) {
                    FontCheckAction::None => OverlayOutcome::Consumed,
                    FontCheckAction::Nerd => self.advance_to_stage_two(state, true),
                    FontCheckAction::Plain => self.advance_to_stage_two(state, false),
                }
            }
            Some(nerdfont) => {
                use app::render::font_check_dialog::{diagonal_check_key_focused, DiagonalCheckAction};
                match diagonal_check_key_focused(key.code, state.overlays.dialog_focus) {
                    DiagonalCheckAction::None => OverlayOutcome::Consumed,
                    DiagonalCheckAction::Diagonal => self.finish(state, nerdfont, Some(true)),
                    DiagonalCheckAction::Orthogonal => self.finish(state, nerdfont, Some(false)),
                    DiagonalCheckAction::Skip => self.finish(state, nerdfont, None),
                }
            }
        }
    }
    fn mouse(&self, state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(fc) = &panes.font_check else { return OverlayOutcome::Consumed };
        match state.overlays.font_check_icon_answer {
            None => {
                if fc.nerd.is_some_and(|r| r.contains(pt)) {
                    return self.advance_to_stage_two(state, true);
                }
                // The close box means row 2, for the same reason Esc does: a
                // stage-one dismissal is a stage-one answer, or it comes back
                // every launch.
                if fc.plain.is_some_and(|r| r.contains(pt)) || fc.close.is_some_and(|r| r.contains(pt)) {
                    return self.advance_to_stage_two(state, false);
                }
            }
            Some(nerdfont) => {
                if fc.nerd.is_some_and(|r| r.contains(pt)) {
                    return self.finish(state, nerdfont, Some(true));
                }
                if fc.plain.is_some_and(|r| r.contains(pt)) {
                    return self.finish(state, nerdfont, Some(false));
                }
                // The close box SKIPS stage two rather than answering "no" for
                // the player — see `DiagonalCheckAction::Skip`.
                if fc.close.is_some_and(|r| r.contains(pt)) {
                    return self.finish(state, nerdfont, None);
                }
            }
        }
        OverlayOutcome::Consumed
    }
}

impl FontCheckOverlay {
    /// Stage one answered: park it and move the dialog to stage two, focused on
    /// its own default (row 2 — the fallback every font can draw).
    fn advance_to_stage_two(&self, state: &mut AppState, nerdfont: bool) -> OverlayOutcome {
        state.overlays.font_check_icon_answer = Some(nerdfont);
        state.overlays.dialog_focus = 1;
        OverlayOutcome::Consumed
    }
    /// Stage two answered or skipped: close the whole check and hand both
    /// answers to the run loop to write together.
    fn finish(&self, state: &mut AppState, nerdfont: bool, diagonal: Option<bool>) -> OverlayOutcome {
        state.overlays.font_check = false;
        state.overlays.font_check_icon_answer = None;
        OverlayOutcome::Act(OverlayAct::FontCheck(nerdfont, diagonal))
    }
}

// ── "Keep this download in your library?" prompt (SQ-1086) ─────────────────
//
// Two buttons, or three when the library already holds a file of that name — see
// `render::fetch_keep_dialog` for why the collision case is not allowed to be
// silent. `button_count` is asked rather than hard-coded so the focus ring and
// the drawn row cannot disagree about how many stops it has.
struct FetchKeepOverlay;
impl Overlay for FetchKeepOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::FetchKeep }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.fetch_keep.is_some() }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.fetch_keep = draw_fetch_keep_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        let n = fetch_keep_button_count(state);
        let collision = state.overlays.fetch_keep.as_ref().is_some_and(|p| p.collision);
        match key.code {
            KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, n, 1);
                OverlayOutcome::Consumed
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, n, -1);
                OverlayOutcome::Consumed
            }
            code => match fetch_keep_key_focused(code, state.overlays.dialog_focus, collision) {
                FetchKeepAction::Keep(mode) => {
                    OverlayOutcome::Act(OverlayAct::FetchKeep(Some(mode)))
                }
                FetchKeepAction::Decline => {
                    OverlayOutcome::Act(OverlayAct::FetchKeep(None))
                }
                FetchKeepAction::None => OverlayOutcome::Consumed,
            },
        }
    }
    fn mouse(&self, state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(fk) = &panes.fetch_keep else { return OverlayOutcome::Consumed };
        let collision = state.overlays.fetch_keep.as_ref().is_some_and(|p| p.collision);
        if fk.keep.is_some_and(|r| r.contains(pt)) {
            let mode = if collision { app::story_url::KeepMode::Replace } else { app::story_url::KeepMode::KeepBoth };
            return OverlayOutcome::Act(OverlayAct::FetchKeep(Some(mode)));
        }
        if fk.keep_both.is_some_and(|r| r.contains(pt)) {
            return OverlayOutcome::Act(OverlayAct::FetchKeep(Some(app::story_url::KeepMode::KeepBoth)));
        }
        if fk.decline.is_some_and(|r| r.contains(pt)) || fk.close.is_some_and(|r| r.contains(pt)) {
            return OverlayOutcome::Act(OverlayAct::FetchKeep(None));
        }
        OverlayOutcome::Consumed
    }
}

// ── Aux-storage prompt ─────────────────────────────────────────────────────
struct AuxOverlay;
impl Overlay for AuxOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::Aux }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.aux_prompt }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.aux_dialog = draw_aux_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        match key.code {
            KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, 1);
                OverlayOutcome::Consumed
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, -1);
                OverlayOutcome::Consumed
            }
            code => match aux_dialog_key_focused(code, state.overlays.dialog_focus) {
                AuxDialogAction::Archive => OverlayOutcome::Act(OverlayAct::AuxArchive),
                AuxDialogAction::Global => OverlayOutcome::Act(OverlayAct::AuxGlobal),
                AuxDialogAction::None => OverlayOutcome::Consumed,
            },
        }
    }
    fn mouse(&self, _state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(ad) = &panes.aux_dialog else { return OverlayOutcome::Consumed };
        let in_close = ad.close.is_some_and(|r| r.contains(pt));
        let in_archive = ad.archive.is_some_and(|r| r.contains(pt));
        let in_global = ad.global.is_some_and(|r| r.contains(pt));
        let in_dialog = ad.area.contains(pt);
        if in_close || in_archive || (!in_global && !in_dialog) {
            OverlayOutcome::Act(OverlayAct::AuxArchive)
        } else if in_global {
            OverlayOutcome::Act(OverlayAct::AuxGlobal)
        } else {
            OverlayOutcome::Consumed
        }
    }
}

// ── Reset dialog ───────────────────────────────────────────────────────────
struct ResetOverlay;
impl Overlay for ResetOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::Reset }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.reset_dialog }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.reset_dialog = draw_reset_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        match key.code {
            KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 4, 1);
                OverlayOutcome::Consumed
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 4, -1);
                OverlayOutcome::Consumed
            }
            code => match reset_dialog_key_focused(code, state.overlays.dialog_focus) {
                ResetDialogAction::Confirm => OverlayOutcome::Act(OverlayAct::ResetConfirm),
                ResetDialogAction::Cancel => OverlayOutcome::Act(OverlayAct::ResetCancel),
                ResetDialogAction::ToggleClearMap => {
                    state.overlays.reset_clear_map = !state.overlays.reset_clear_map;
                    OverlayOutcome::Consumed
                }
                ResetDialogAction::ToggleDeleteData => {
                    state.overlays.reset_delete_data = !state.overlays.reset_delete_data;
                    OverlayOutcome::Consumed
                }
                ResetDialogAction::None => OverlayOutcome::Consumed,
            },
        }
    }
    fn mouse(&self, state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(rd) = &panes.reset_dialog else { return OverlayOutcome::Consumed };
        // Check buttons and close in order: close > reset > cancel > checkbox.
        let in_close = rd.close.is_some_and(|r| r.contains(pt));
        let in_reset = rd.reset.is_some_and(|r| r.contains(pt));
        let in_cancel = rd.cancel.is_some_and(|r| r.contains(pt));
        let in_checkbox = rd.checkbox.contains(pt);
        let in_checkbox_data = rd.checkbox_data.contains(pt);
        if in_close || in_cancel {
            OverlayOutcome::Act(OverlayAct::ResetCancel)
        } else if in_reset {
            OverlayOutcome::Act(OverlayAct::ResetConfirm)
        } else if in_checkbox {
            state.overlays.reset_clear_map = !state.overlays.reset_clear_map;
            OverlayOutcome::Consumed
        } else if in_checkbox_data {
            state.overlays.reset_delete_data = !state.overlays.reset_delete_data;
            OverlayOutcome::Consumed
        } else {
            // Click outside the dialog (or its interior): swallow, keep it open.
            OverlayOutcome::Consumed
        }
    }
}

// ── Game-over dialog (Scott-only win/loss) ─────────────────────────────────
struct GameOverOverlay;
impl Overlay for GameOverOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::GameOver }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.game_over }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.game_over = draw_game_over_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        match key.code {
            KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 3, 1);
                OverlayOutcome::Consumed
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 3, -1);
                OverlayOutcome::Consumed
            }
            code => match game_over_dialog_key_focused(code, state.overlays.dialog_focus) {
                GameOverAction::PlayAgain => OverlayOutcome::Act(OverlayAct::GameOverPlayAgain),
                GameOverAction::Restore => OverlayOutcome::Act(OverlayAct::GameOverRestore),
                GameOverAction::Quit => OverlayOutcome::Act(OverlayAct::GameOverQuit),
                GameOverAction::None => OverlayOutcome::Consumed,
            },
        }
    }
    fn mouse(&self, _state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(gd) = &panes.game_over else { return OverlayOutcome::Consumed };
        let in_play_again = gd.play_again.is_some_and(|r| r.contains(pt));
        let in_restore = gd.restore.is_some_and(|r| r.contains(pt));
        let in_quit = gd.quit.is_some_and(|r| r.contains(pt));
        if in_play_again {
            OverlayOutcome::Act(OverlayAct::GameOverPlayAgain)
        } else if in_restore {
            OverlayOutcome::Act(OverlayAct::GameOverRestore)
        } else if in_quit {
            OverlayOutcome::Act(OverlayAct::GameOverQuit)
        } else {
            // Click outside the dialog: swallow, keep it open (the game is over).
            OverlayOutcome::Consumed
        }
    }
}

// ── Confirm-overwrite (two-button, over the save-name dialog) ──────────────
struct ConfirmOverwriteOverlay;
impl Overlay for ConfirmOverwriteOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::ConfirmOverwrite }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.confirm_overwrite_save.is_some() }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.confirm_overwrite = draw_confirm_overwrite_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        match key.code {
            KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, 1);
                OverlayOutcome::Consumed
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, -1);
                OverlayOutcome::Consumed
            }
            code => match confirm_overwrite_key_focused(code, state.overlays.dialog_focus) {
                ConfirmOverwriteAction::Confirm => OverlayOutcome::Act(OverlayAct::ConfirmOverwrite(true)),
                ConfirmOverwriteAction::Cancel => OverlayOutcome::Act(OverlayAct::ConfirmOverwrite(false)),
                ConfirmOverwriteAction::None => OverlayOutcome::Consumed,
            },
        }
    }
    fn mouse(&self, _state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(cd) = &panes.confirm_overwrite else { return OverlayOutcome::Consumed };
        let in_close = cd.close.is_some_and(|r| r.contains(pt));
        let in_overwrite = cd.overwrite.is_some_and(|r| r.contains(pt));
        let in_cancel = cd.cancel.is_some_and(|r| r.contains(pt));
        if in_close || in_cancel {
            OverlayOutcome::Act(OverlayAct::ConfirmOverwrite(false))
        } else if in_overwrite {
            OverlayOutcome::Act(OverlayAct::ConfirmOverwrite(true))
        } else {
            OverlayOutcome::Consumed
        }
    }
}

// ── Save-name dialog (caret text field) ────────────────────────────────────
struct SaveNameOverlay;
impl Overlay for SaveNameOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::SaveName }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.save_name_dialog.is_some() }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.save_name_dialog = draw_save_name_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        if is_ctrl_char(key) {
            return OverlayOutcome::Consumed;
        }
        let focus = state.overlays.dialog_focus;
        let dlg = state.overlays.save_name_dialog.as_mut().unwrap();
        let (act, new_focus) = save_name_dialog_key(key.code, dlg, focus);
        state.overlays.dialog_focus = new_focus;
        match act {
            SaveNameAction::Save => OverlayOutcome::Act(OverlayAct::SaveNameSubmit),
            SaveNameAction::Cancel => OverlayOutcome::Act(OverlayAct::SaveNameCancel),
            SaveNameAction::None => OverlayOutcome::Consumed,
        }
    }
    fn mouse(&self, state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(sd) = &panes.save_name_dialog else { return OverlayOutcome::Consumed };
        let in_close = sd.close.is_some_and(|r| r.contains(pt));
        let in_save = sd.save.is_some_and(|r| r.contains(pt));
        let in_cancel = sd.cancel.is_some_and(|r| r.contains(pt));
        let in_field = sd.field.is_some_and(|r| r.contains(pt));
        if in_close || in_cancel {
            OverlayOutcome::Act(OverlayAct::SaveNameCancel)
        } else if in_save {
            OverlayOutcome::Act(OverlayAct::SaveNameSubmit)
        } else if in_field {
            // Focus + activate the field (caret to end).
            state.overlays.dialog_focus = 0;
            if let Some(dlg) = state.overlays.save_name_dialog.as_mut() {
                dlg.active = true;
                dlg.field.end();
            }
            OverlayOutcome::Consumed
        } else {
            // Click outside: swallow, keep the dialog open.
            OverlayOutcome::Consumed
        }
    }
}

// ── Text-entry dialog (generic single-field) ───────────────────────────────
struct TextEntryOverlay;
impl Overlay for TextEntryOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::TextEntry }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.text_entry.is_some() }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.text_entry = draw_text_entry_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        if is_ctrl_char(key) {
            return OverlayOutcome::Consumed;
        }
        let focus = state.overlays.dialog_focus;
        let dlg = state.overlays.text_entry.as_mut().unwrap();
        let (act, new_focus) = text_entry_dialog_key(key.code, &mut dlg.field, focus);
        state.overlays.dialog_focus = new_focus;
        match act {
            TextEntryAction::Submit => OverlayOutcome::Act(OverlayAct::TextEntrySubmit),
            TextEntryAction::Cancel => OverlayOutcome::Act(OverlayAct::TextEntryCancel),
            TextEntryAction::None => OverlayOutcome::Consumed,
        }
    }
    fn mouse(&self, state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(td) = &panes.text_entry else { return OverlayOutcome::Consumed };
        let in_close = td.close.is_some_and(|r| r.contains(pt));
        let in_ok = td.ok.is_some_and(|r| r.contains(pt));
        let in_cancel = td.cancel.is_some_and(|r| r.contains(pt));
        let in_field = td.field.is_some_and(|r| r.contains(pt));
        if in_close || in_cancel {
            OverlayOutcome::Act(OverlayAct::TextEntryCancel)
        } else if in_ok {
            OverlayOutcome::Act(OverlayAct::TextEntrySubmit)
        } else if in_field {
            // Focus the field (caret to end).
            state.overlays.dialog_focus = 0;
            if let Some(dlg) = state.overlays.text_entry.as_mut() {
                dlg.field.end();
            }
            OverlayOutcome::Consumed
        } else {
            // Click outside: swallow, keep the dialog open.
            OverlayOutcome::Consumed
        }
    }
}

// ── Confirm-delete (two-button, over the saves manager) ────────────────────
struct ConfirmDeleteOverlay;
impl Overlay for ConfirmDeleteOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::ConfirmDelete }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.confirm_delete_save.is_some() }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.confirm_delete = draw_confirm_delete_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        match key.code {
            KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, 1);
                OverlayOutcome::Consumed
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, -1);
                OverlayOutcome::Consumed
            }
            code => match confirm_delete_key_focused(code, state.overlays.dialog_focus) {
                ConfirmDeleteAction::Confirm => OverlayOutcome::Act(OverlayAct::ConfirmDelete(true)),
                ConfirmDeleteAction::Cancel => OverlayOutcome::Act(OverlayAct::ConfirmDelete(false)),
                ConfirmDeleteAction::None => OverlayOutcome::Consumed,
            },
        }
    }
    fn mouse(&self, _state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(cd) = &panes.confirm_delete else { return OverlayOutcome::Consumed };
        let in_close = cd.close.is_some_and(|r| r.contains(pt));
        let in_delete = cd.delete.is_some_and(|r| r.contains(pt));
        let in_cancel = cd.cancel.is_some_and(|r| r.contains(pt));
        if in_close || in_cancel {
            OverlayOutcome::Act(OverlayAct::ConfirmDelete(false))
        } else if in_delete {
            OverlayOutcome::Act(OverlayAct::ConfirmDelete(true))
        } else {
            OverlayOutcome::Consumed
        }
    }
}

// ── Quit dialog ────────────────────────────────────────────────────────────
struct QuitOverlay;
impl Overlay for QuitOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::Quit }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.quit_dialog }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.quit_dialog = draw_quit_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        match key.code {
            KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 3, 1);
                OverlayOutcome::Consumed
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 3, -1);
                OverlayOutcome::Consumed
            }
            code => match quit_dialog_key_focused(code, state.overlays.dialog_focus) {
                QuitDialogAction::Save => OverlayOutcome::Act(OverlayAct::QuitSave),
                QuitDialogAction::Quit => OverlayOutcome::Act(OverlayAct::QuitQuit),
                QuitDialogAction::Cancel => OverlayOutcome::Act(OverlayAct::QuitCancel),
                QuitDialogAction::None => OverlayOutcome::Consumed,
            },
        }
    }
    fn mouse(&self, _state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(qd) = &panes.quit_dialog else { return OverlayOutcome::Consumed };
        let in_close = qd.close.is_some_and(|r| r.contains(pt));
        let in_save = qd.save.is_some_and(|r| r.contains(pt));
        let in_quit = qd.quit.is_some_and(|r| r.contains(pt));
        let in_cancel = qd.cancel.is_some_and(|r| r.contains(pt));
        if in_save {
            OverlayOutcome::Act(OverlayAct::QuitSave)
        } else if in_quit {
            OverlayOutcome::Act(OverlayAct::QuitQuit)
        } else if in_close || in_cancel {
            OverlayOutcome::Act(OverlayAct::QuitCancel)
        } else {
            // Click outside: swallow (keep dialog open).
            OverlayOutcome::Consumed
        }
    }
}

// ── Launch dialog (resume saved game at startup) ───────────────────────────
struct LaunchOverlay;
impl Overlay for LaunchOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::Launch }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.launch_dialog }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.launch_dialog = draw_launch_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        match key.code {
            KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, 1);
                OverlayOutcome::Consumed
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, -1);
                OverlayOutcome::Consumed
            }
            code => match launch_dialog_key_focused(code, state.overlays.dialog_focus) {
                LaunchDialogAction::Resume => OverlayOutcome::Act(OverlayAct::LaunchResume),
                LaunchDialogAction::NewGame => OverlayOutcome::Act(OverlayAct::LaunchNewGame),
                LaunchDialogAction::None => OverlayOutcome::Consumed,
            },
        }
    }
    fn mouse(&self, _state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(ld) = &panes.launch_dialog else { return OverlayOutcome::Consumed };
        let in_resume = ld.resume.is_some_and(|r| r.contains(pt));
        let in_new_game = ld.new_game.is_some_and(|r| r.contains(pt));
        let in_close = ld.close.is_some_and(|r| r.contains(pt));
        if in_resume {
            OverlayOutcome::Act(OverlayAct::LaunchResume)
        } else if in_new_game || in_close {
            // [X] (close) and [New game] both discard the save.
            OverlayOutcome::Act(OverlayAct::LaunchNewGame)
        } else {
            // Click outside: swallow (keep dialog open).
            OverlayOutcome::Consumed
        }
    }
}

// ── Region prompt (the map's own suggestion, and the two manual pickers) ───
struct RegionPromptOverlay;
impl Overlay for RegionPromptOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::RegionPrompt }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.region_prompt.is_some() }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.region_prompt = draw_region_prompt(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        let Some(prompt) = state.overlays.region_prompt.as_ref() else {
            return OverlayOutcome::Consumed;
        };
        let slots = prompt.focus_slots();
        let step = match key.code {
            KeyCode::Tab | KeyCode::Right | KeyCode::Down => Some(1),
            KeyCode::BackTab | KeyCode::Left | KeyCode::Up => Some(-1),
            _ => None,
        };
        if let Some(step) = step {
            let focus = cycle_focus(state.overlays.dialog_focus, slots, step);
            state.overlays.dialog_focus = focus;
            // The options are a radio list, so resting on one CHOOSES it — there is nothing left
            // for a second keystroke to do, and Enter can therefore mean "yes" everywhere.
            if let Some(p) = state.overlays.region_prompt.as_mut() {
                if focus < p.options.len() {
                    p.choice = focus;
                }
            }
            return OverlayOutcome::Consumed;
        }
        match region_prompt_key_focused(key.code, prompt, state.overlays.dialog_focus) {
            Some(act) => OverlayOutcome::Act(OverlayAct::RegionPrompt(act)),
            None => OverlayOutcome::Consumed,
        }
    }
    fn mouse(&self, state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        use app::state::RegionPromptAct as A;
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(rp) = &panes.region_prompt else { return OverlayOutcome::Consumed };
        if let Some(i) = rp.options.iter().position(|r| r.width > 0 && r.contains(pt)) {
            state.overlays.dialog_focus = i;
            if let Some(p) = state.overlays.region_prompt.as_mut() {
                p.choice = i;
            }
            return OverlayOutcome::Consumed;
        }
        let hit = |r: &Option<Rect>| r.is_some_and(|r| r.contains(pt));
        // Closing a suggestion is "not now" — the same as Esc, and not a refusal.
        let closing = if state.overlays.region_prompt.as_ref().is_some_and(|p| p.buttons() == 3) {
            A::Defer
        } else {
            A::Dismiss
        };
        if hit(&rp.accept) {
            OverlayOutcome::Act(OverlayAct::RegionPrompt(A::Accept))
        } else if hit(&rp.never) {
            OverlayOutcome::Act(OverlayAct::RegionPrompt(A::Never))
        } else if hit(&rp.later) || hit(&rp.cancel) || hit(&rp.close) {
            OverlayOutcome::Act(OverlayAct::RegionPrompt(closing))
        } else {
            // Click outside: swallow, keep it open.
            OverlayOutcome::Consumed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Up/Down arrow keys move focus between a dialog's options, not just its
    /// buttons — Down advances the 4-slot reset ring, Up reverses. (SQ-0176 follow-up)
    #[test]
    fn arrow_keys_cycle_dialog_focus() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut state = AppState::default();
        state.overlays.reset_dialog = true;
        state.overlays.dialog_focus = 0;
        let ov = topmost_common_dialog(&state.overlays).expect("reset dialog is open");

        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        ov.key(&mut state, &down);
        assert_eq!(state.overlays.dialog_focus, 1, "Down advances to the next option");
        ov.key(&mut state, &down);
        assert_eq!(state.overlays.dialog_focus, 2, "Down keeps advancing through options and buttons");

        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        ov.key(&mut state, &up);
        assert_eq!(state.overlays.dialog_focus, 1, "Up reverses");
    }

    /// The common-dialog priority ladder resolves to the exact z-order the old
    /// run-loop if-ladder used: aux ▸ reset ▸ game-over ▸ confirm-overwrite ▸
    /// save-name ▸ text-entry ▸ confirm-delete ▸ quit ▸ launch.
    /// `topmost_common_dialog` must return the highest-priority open overlay
    /// regardless of which lower ones are also open.
    /// Enter on the affirmative button asks the run loop to switch recording on;
    /// Esc and the second button just close (SQ-1091).
    ///
    /// The overlay cannot persist the setting itself — `write_config_file` needs
    /// paths the run loop owns — so what is asserted here is the ACT it returns.
    /// The arm that consumes it is three lines in `main.rs` and is type-checked by
    /// the match being exhaustive.
    #[test]
    fn the_history_prompt_returns_an_enable_act_only_on_the_affirmative() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);

        let mut s = AppState::default();
        s.overlays.history_prompt = true;
        s.overlays.dialog_focus = 0;
        let out = HistoryPromptOverlay.key(&mut s, &key(KeyCode::Enter));
        assert!(matches!(out, OverlayOutcome::Act(OverlayAct::EnableTurnHistory)));
        assert!(!s.overlays.history_prompt, "and it closes behind itself");

        // Focus on "Not now" → closes, asks for nothing.
        let mut s = AppState::default();
        s.overlays.history_prompt = true;
        s.overlays.dialog_focus = 1;
        let out = HistoryPromptOverlay.key(&mut s, &key(KeyCode::Enter));
        assert!(matches!(out, OverlayOutcome::Consumed));
        assert!(!s.overlays.history_prompt);

        // Esc → closes, asks for nothing, whatever the focus.
        let mut s = AppState::default();
        s.overlays.history_prompt = true;
        s.overlays.dialog_focus = 0;
        let out = HistoryPromptOverlay.key(&mut s, &key(KeyCode::Esc));
        assert!(matches!(out, OverlayOutcome::Consumed));
        assert!(!s.overlays.history_prompt);

        // Tab moves between exactly two buttons and wraps.
        let mut s = AppState::default();
        s.overlays.history_prompt = true;
        s.overlays.dialog_focus = 0;
        HistoryPromptOverlay.key(&mut s, &key(KeyCode::Tab));
        assert_eq!(s.overlays.dialog_focus, 1);
        HistoryPromptOverlay.key(&mut s, &key(KeyCode::Tab));
        assert_eq!(s.overlays.dialog_focus, 0, "two buttons, so it wraps");
    }

    /// A pending keep-prompt for the tests below.
    fn a_fetch_keep(collision: bool) -> app::state::FetchKeepPrompt {
        app::state::FetchKeepPrompt {
            fetched: app::story_url::FetchedStory {
                url: "https://example.org/curses.z5".into(),
                path: std::path::PathBuf::from("/tmp/lanthorn-fetch/curses.z5"),
            },
            library_dir: std::path::PathBuf::from("/home/p/stories"),
            collision,
            disk_images: Vec::new(),
        }
    }

    /// SQ-1086. The keep prompt must ask for a copy only on an affirmative, must
    /// leave the prompt in place for the run loop's arm to consume (it carries
    /// the destination), and must never make "replace" the answer Enter lands on
    /// from an inherited focus of 0.
    #[test]
    fn the_fetch_keep_prompt_maps_focus_to_the_right_answer() {
        use app::story_url::KeepMode;
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);

        // No collision: 0 keeps, 1 declines, Esc declines.
        let mut s = AppState::default();
        s.overlays.fetch_keep = Some(a_fetch_keep(false));
        s.overlays.dialog_focus = 0;
        let out = FetchKeepOverlay.key(&mut s, &key(KeyCode::Enter));
        assert!(matches!(out, OverlayOutcome::Act(OverlayAct::FetchKeep(Some(KeepMode::KeepBoth)))));
        assert!(s.overlays.fetch_keep.is_some(), "the run loop's arm takes it, not the overlay");

        s.overlays.dialog_focus = 1;
        let out = FetchKeepOverlay.key(&mut s, &key(KeyCode::Enter));
        assert!(matches!(out, OverlayOutcome::Act(OverlayAct::FetchKeep(None))));

        s.overlays.dialog_focus = 0;
        let out = FetchKeepOverlay.key(&mut s, &key(KeyCode::Esc));
        assert!(matches!(out, OverlayOutcome::Act(OverlayAct::FetchKeep(None))));

        // Collision: focus 0 is the harmless keep, 1 replaces, 2 declines.
        let mut s = AppState::default();
        s.overlays.fetch_keep = Some(a_fetch_keep(true));
        s.overlays.dialog_focus = 0;
        let out = FetchKeepOverlay.key(&mut s, &key(KeyCode::Enter));
        assert!(
            matches!(out, OverlayOutcome::Act(OverlayAct::FetchKeep(Some(KeepMode::KeepBoth)))),
            "an inherited focus of 0 must never mean `replace`"
        );
        s.overlays.dialog_focus = 1;
        let out = FetchKeepOverlay.key(&mut s, &key(KeyCode::Enter));
        assert!(matches!(out, OverlayOutcome::Act(OverlayAct::FetchKeep(Some(KeepMode::Replace)))));
        s.overlays.dialog_focus = 2;
        let out = FetchKeepOverlay.key(&mut s, &key(KeyCode::Enter));
        assert!(matches!(out, OverlayOutcome::Act(OverlayAct::FetchKeep(None))));
    }

    /// The focus ring has as many stops as the button row has buttons — three
    /// when the name collides, two otherwise.
    #[test]
    fn the_fetch_keep_focus_ring_matches_its_button_row() {
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        let mut s = AppState::default();
        s.overlays.fetch_keep = Some(a_fetch_keep(false));
        s.overlays.dialog_focus = 0;
        FetchKeepOverlay.key(&mut s, &key(KeyCode::Tab));
        assert_eq!(s.overlays.dialog_focus, 1);
        FetchKeepOverlay.key(&mut s, &key(KeyCode::Tab));
        assert_eq!(s.overlays.dialog_focus, 0, "two buttons, so it wraps");

        s.overlays.fetch_keep = Some(a_fetch_keep(true));
        FetchKeepOverlay.key(&mut s, &key(KeyCode::Tab));
        FetchKeepOverlay.key(&mut s, &key(KeyCode::Tab));
        assert_eq!(s.overlays.dialog_focus, 2, "three buttons when the name collides");
        FetchKeepOverlay.key(&mut s, &key(KeyCode::Tab));
        assert_eq!(s.overlays.dialog_focus, 0);
        // Shift-Tab reverses it, per the standing convention.
        FetchKeepOverlay.key(&mut s, &key(KeyCode::BackTab));
        assert_eq!(s.overlays.dialog_focus, 2);
    }

    /// The keep prompt is raised by the app at boot, so it yields to the
    /// resume-or-new-game question about the very session it belongs to — but it
    /// still outranks the region prompt at the very bottom.
    #[test]
    fn the_fetch_keep_prompt_yields_to_the_launch_dialog() {
        let mut o = OverlayState::default();
        o.fetch_keep = Some(a_fetch_keep(false));
        assert_eq!(topmost_common_dialog(&o).unwrap().kind(), OverlayKind::FetchKeep);
        o.launch_dialog = true;
        assert_eq!(topmost_common_dialog(&o).unwrap().kind(), OverlayKind::Launch);
        o.launch_dialog = false;
        o.region_prompt = Some(a_region_prompt());
        assert_eq!(topmost_common_dialog(&o).unwrap().kind(), OverlayKind::FetchKeep);
    }

    #[test]
    fn topmost_common_dialog_preserves_ladder_order() {
        // No overlay open → None.
        let ov = OverlayState::default();
        assert!(topmost_common_dialog(&ov).is_none());

        // Each overlay alone resolves to itself, in ladder order.
        #[allow(clippy::type_complexity)]
        let cases: &[(fn(&mut OverlayState), OverlayKind)] = &[
            (|o| o.history_prompt = true, OverlayKind::HistoryPrompt),
            (|o| o.font_check = true, OverlayKind::FontCheck),
            (|o| o.aux_prompt = true, OverlayKind::Aux),
            (|o| o.reset_dialog = true, OverlayKind::Reset),
            (|o| o.game_over = true, OverlayKind::GameOver),
            (|o| o.confirm_overwrite_save = Some(app::state::ConfirmOverwriteSave {
                path: std::path::PathBuf::from("s.lanthorn"),
                existing_name: "s".to_string(),
                pending: app::state::PendingOverwrite::SaveAs,
            }), OverlayKind::ConfirmOverwrite),
            (|o| o.save_name_dialog = Some(app::state::SaveNameDialog::new(String::new(), false)), OverlayKind::SaveName),
            (|o| o.text_entry = Some(app::state::TextEntryDialog::new(app::state::TextEntryKind::CreateFile, "")), OverlayKind::TextEntry),
            (|o| o.confirm_delete_save = Some(std::path::PathBuf::from("s.sav")), OverlayKind::ConfirmDelete),
            (|o| o.quit_dialog = true, OverlayKind::Quit),
            (|o| o.launch_dialog = true, OverlayKind::Launch),
            (|o| o.fetch_keep = Some(a_fetch_keep(false)), OverlayKind::FetchKeep),
        ];
        for (open, want) in cases {
            let mut o = OverlayState::default();
            open(&mut o);
            assert_eq!(topmost_common_dialog(&o).unwrap().kind(), *want);
        }

        // With several open at once, the highest-priority (earliest) wins.
        let mut o = OverlayState::default();
        o.launch_dialog = true;
        o.quit_dialog = true;
        o.reset_dialog = true;
        assert_eq!(topmost_common_dialog(&o).unwrap().kind(), OverlayKind::Reset);

        // Drop reset → quit is now top-most (still above launch).
        o.reset_dialog = false;
        assert_eq!(topmost_common_dialog(&o).unwrap().kind(), OverlayKind::Quit);

        // Aux sits above everything.
        o.aux_prompt = true;
        assert_eq!(topmost_common_dialog(&o).unwrap().kind(), OverlayKind::Aux);
    }

    /// A region prompt for the ladder / focus tests: two destinations to choose between.
    #[cfg(test)]
    fn a_region_prompt() -> app::state::RegionPrompt {
        use app::state::{RegionOption, RegionPrompt, RegionPromptKind};
        use mapper::layer::{MoveTarget, Region, MAIN_LAYER};
        RegionPrompt {
            kind: RegionPromptKind::PickDest {
                region: Region { anchor: 2, rooms: [2, 3].into_iter().collect() },
                cut: None,
            },
            title: "Where do these rooms go?".into(),
            body: vec!["More than one layer could take them.".into()],
            rooms: vec!["B".into(), "C".into()],
            options: vec![
                RegionOption::Dest { label: "a new layer".into(), target: MoveTarget::New },
                RegionOption::Dest { label: "Main".into(), target: MoveTarget::Existing(MAIN_LAYER) },
            ],
            choice: 0,
        }
    }

    /// SQ-0439: the region prompt is the only modal in the ladder the APP raises on its own, so it
    /// sits at the bottom — anything the player asked for wins the scan.
    #[test]
    fn a_region_prompt_yields_to_every_modal_the_player_asked_for() {
        let mut o = OverlayState::default();
        o.region_prompt = Some(a_region_prompt());
        assert_eq!(topmost_common_dialog(&o).unwrap().kind(), OverlayKind::RegionPrompt);
        o.quit_dialog = true;
        assert_eq!(topmost_common_dialog(&o).unwrap().kind(), OverlayKind::Quit);
        o.quit_dialog = false;
        o.launch_dialog = true;
        assert_eq!(topmost_common_dialog(&o).unwrap().kind(), OverlayKind::Launch);
    }

    /// The prompt's focus ring runs its options first and then its buttons, and resting on an
    /// option CHOOSES it — so Enter can mean "yes" wherever the ring happens to be. Shift-Tab
    /// reverses, per the standing convention.
    #[test]
    fn region_prompt_focus_chooses_the_option_it_rests_on() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut state = AppState::default();
        state.overlays.region_prompt = Some(a_region_prompt());
        state.overlays.dialog_focus = 0;
        let ov = topmost_common_dialog(&state.overlays).expect("the prompt is open");

        // Two options + Move + Cancel = a four-slot ring.
        ov.key(&mut state, &KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.overlays.dialog_focus, 1);
        assert_eq!(state.overlays.region_prompt.as_ref().unwrap().choice, 1, "focus chose it");

        // Onto the buttons: the choice stays where the ring left it.
        ov.key(&mut state, &KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.overlays.dialog_focus, 2);
        assert_eq!(
            state.overlays.region_prompt.as_ref().unwrap().choice,
            1,
            "a button does not un-choose the option"
        );

        // Shift-Tab reverses back onto it.
        ov.key(&mut state, &KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE));
        assert_eq!(state.overlays.dialog_focus, 1);

        // And the ring wraps at four, not at two.
        for _ in 0..3 {
            ov.key(&mut state, &KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        }
        assert_eq!(state.overlays.dialog_focus, 0, "four slots: two options and two buttons");
    }

    /// SQ-0648: the save-as flow leaves the save-name dialog open BEHIND the
    /// confirm-overwrite overlay so Cancel needs no recovery. Confirm-overwrite
    /// must win the priority scan whenever both are open at once, or the
    /// overlay ladder would route input to the wrong modal.
    #[test]
    fn confirm_overwrite_outranks_save_name_when_both_open() {
        let mut o = OverlayState::default();
        o.save_name_dialog = Some(app::state::SaveNameDialog::new("chapter one".to_string(), false));
        o.confirm_overwrite_save = Some(app::state::ConfirmOverwriteSave {
            path: std::path::PathBuf::from("chapter-one.lanthorn"),
            existing_name: "Chapter One".to_string(),
            pending: app::state::PendingOverwrite::SaveAs,
        });
        assert_eq!(
            topmost_common_dialog(&o).unwrap().kind(),
            OverlayKind::ConfirmOverwrite,
            "confirm-overwrite must be topmost while the save-name dialog waits behind it"
        );
    }

    /// SQ-1245: answering stage one does not close the font check — it moves it
    /// on to stage two, carrying stage one's answer in
    /// `font_check_icon_answer`, and only stage two's answer (or a skip)
    /// finally closes it and hands the run loop both.
    #[test]
    fn the_font_check_reaches_stage_two_before_it_closes() {
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);

        let mut s = AppState::default();
        s.overlays.font_check = true;
        s.overlays.dialog_focus = 0; // row 1 — the Nerd Font answer
        let out = FontCheckOverlay.key(&mut s, &key(KeyCode::Enter));
        assert!(matches!(out, OverlayOutcome::Consumed), "stage one does not close on its own");
        assert!(s.overlays.font_check, "the check is still open");
        assert_eq!(s.overlays.font_check_icon_answer, Some(true), "stage one's answer is parked");
        assert_eq!(s.overlays.dialog_focus, 1, "stage two opens on its own default focus");

        // Stage two, row 1 (diagonals): closes and hands both answers over.
        let out = FontCheckOverlay.key(&mut s, &key(KeyCode::Tab));
        assert!(matches!(out, OverlayOutcome::Consumed), "Tab still just moves focus on stage two");
        let out = FontCheckOverlay.key(&mut s, &key(KeyCode::Enter));
        assert!(
            matches!(out, OverlayOutcome::Act(OverlayAct::FontCheck(true, Some(true)))),
            "row 1 on both stages: nerdfont=true, diagonal=Some(true)"
        );
        assert!(!s.overlays.font_check, "closed behind itself");
        assert!(s.overlays.font_check_icon_answer.is_none(), "and the parked answer is cleared");
    }

    /// The full answer matrix, at the overlay level: icons and diagonals answer
    /// independently in both directions, and stage two's Esc skips (leaves
    /// `None`) rather than answering "no" the way stage one's Esc answers
    /// "plain".
    #[test]
    fn the_font_check_answer_matrix_and_stage_two_esc_is_a_skip() {
        let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);

        // icons no (Esc), diagonals yes.
        let mut s = AppState::default();
        s.overlays.font_check = true;
        FontCheckOverlay.key(&mut s, &key(KeyCode::Esc));
        assert_eq!(s.overlays.font_check_icon_answer, Some(false), "stage one Esc means plain");
        s.overlays.dialog_focus = 0; // row 1 — diagonals
        let out = FontCheckOverlay.key(&mut s, &key(KeyCode::Enter));
        assert!(
            matches!(out, OverlayOutcome::Act(OverlayAct::FontCheck(false, Some(true)))),
            "icons no, diagonals yes"
        );

        // icons yes, diagonals skipped via Esc.
        let mut s = AppState::default();
        s.overlays.font_check = true;
        s.overlays.dialog_focus = 0;
        FontCheckOverlay.key(&mut s, &key(KeyCode::Enter));
        assert_eq!(s.overlays.font_check_icon_answer, Some(true));
        let out = FontCheckOverlay.key(&mut s, &key(KeyCode::Esc));
        assert!(
            matches!(out, OverlayOutcome::Act(OverlayAct::FontCheck(true, None))),
            "stage two Esc is a skip (None), not an answer of its own"
        );
    }
}
